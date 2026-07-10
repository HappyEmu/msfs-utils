use crate::DynError;
use crate::network::RelayClient;
use crate::protocol::{self, AircraftState, AircraftUpdate};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

pub type LatestLocalState = Arc<Mutex<Option<AircraftState>>>;
pub type LatestRemoteSnapshot = Arc<Mutex<Option<Vec<AircraftUpdate>>>>;

pub async fn run(
    client: &RelayClient,
    local_state: LatestLocalState,
    remote_snapshot: LatestRemoteSnapshot,
) -> Result<(), DynError> {
    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut updates = tokio::time::interval(Duration::from_millis(33));
    updates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buffer = vec![0; protocol::MAX_PACKET_SIZE];

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
                    *remote_snapshot
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(snapshot);
                }
            }
        }
    }
}
