use crate::DynError;
use crate::network::RelayClient;
use crate::protocol::{self, AircraftState};
use msfs_multiplayer::interpolation::InterpolationBuffer;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

const PLAYOUT_DELAY: Duration = Duration::from_millis(500);
const RENDER_INTERVAL: Duration = Duration::from_millis(33);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(3);
const STATS_INTERVAL: Duration = Duration::from_secs(5);

pub type LatestLocalState = Arc<Mutex<Option<AircraftState>>>;
pub type LatestRemoteSnapshot = Arc<Mutex<Option<Vec<RenderedAircraftUpdate>>>>;

#[derive(Clone, Copy, Debug)]
pub struct RenderedAircraftUpdate {
    pub user_id: u64,
    pub playback_timestamp_seconds: f64,
    pub state: AircraftState,
    pub buffer_depth: usize,
    pub underrun: bool,
}

struct RemoteBuffer {
    interpolation: InterpolationBuffer,
    last_observed_sequence: Option<u64>,
}

impl RemoteBuffer {
    fn new() -> Self {
        Self {
            interpolation: InterpolationBuffer::new(PLAYOUT_DELAY),
            last_observed_sequence: None,
        }
    }
}

pub async fn run(
    client: &RelayClient,
    local_state: LatestLocalState,
    remote_snapshot: LatestRemoteSnapshot,
) -> Result<(), DynError> {
    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut updates = tokio::time::interval(Duration::from_millis(33));
    updates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut renders = tokio::time::interval(RENDER_INTERVAL);
    renders.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stats_tick = tokio::time::interval(STATS_INTERVAL);
    stats_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buffer = vec![0; protocol::MAX_PACKET_SIZE];
    let mut remote_buffers = HashMap::<u64, RemoteBuffer>::new();
    let mut last_snapshot_at = None;
    let mut rejected_samples = 0_u64;
    let mut underruns = 0_u64;

    loop {
        tokio::select! {
            _ = updates.tick() => {
                let state = *local_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(state) = state {
                    sequence = sequence.wrapping_add(1);
                    client
                        .send_update(sequence, started.elapsed().as_secs_f64(), state)
                        .await?;
                }
            }
            snapshot = client.receive_snapshot(&mut buffer) => {
                if let Some(snapshot) = snapshot? {
                    let arrived_at = started.elapsed();
                    last_snapshot_at = Some(arrived_at);
                    let present = snapshot
                        .iter()
                        .map(|update| update.user_id)
                        .collect::<HashSet<_>>();
                    remote_buffers.retain(|user_id, _| present.contains(user_id));

                    for update in snapshot {
                        let remote = remote_buffers
                            .entry(update.user_id)
                            .or_insert_with(RemoteBuffer::new);
                        if remote
                            .last_observed_sequence
                            .is_some_and(|sequence| update.sequence == sequence)
                        {
                            continue;
                        }
                        if remote
                            .last_observed_sequence
                            .is_some_and(|sequence| update.sequence < sequence)
                        {
                            rejected_samples += 1;
                            continue;
                        }
                        remote.last_observed_sequence = Some(update.sequence);
                        if !remote.interpolation.push(update, arrived_at) {
                            rejected_samples += 1;
                        }
                    }
                }
            }
            _ = renders.tick() => {
                let now = started.elapsed();
                if last_snapshot_at.is_some_and(|arrival| now.saturating_sub(arrival) > SNAPSHOT_TIMEOUT) {
                    remote_buffers.clear();
                }

                let rendered = remote_buffers
                    .iter_mut()
                    .filter_map(|(&user_id, remote)| {
                        let sample = remote.interpolation.sample_buffered(now)?;
                        if sample.underrun {
                            underruns += 1;
                        }
                        Some(RenderedAircraftUpdate {
                            user_id,
                            playback_timestamp_seconds: sample.timestamp_seconds,
                            state: sample.state,
                            buffer_depth: remote.interpolation.len(),
                            underrun: sample.underrun,
                        })
                    })
                    .collect();
                *remote_snapshot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(rendered);
            }
            _ = stats_tick.tick() => {
                if !remote_buffers.is_empty() || rejected_samples != 0 || underruns != 0 {
                    let depths = remote_buffers
                        .values()
                        .map(|remote| remote.interpolation.len())
                        .collect::<Vec<_>>();
                    let minimum = depths.iter().copied().min().unwrap_or(0);
                    let maximum = depths.iter().copied().max().unwrap_or(0);
                    let average = if depths.is_empty() {
                        0.0
                    } else {
                        depths.iter().sum::<usize>() as f64 / depths.len() as f64
                    };
                    eprintln!(
                        "Playout buffers: users={} depth={minimum}/{average:.1}/{maximum} min/avg/max rejected={rejected_samples} underruns={underruns}",
                        remote_buffers.len(),
                    );
                }
            }
        }
    }
}
