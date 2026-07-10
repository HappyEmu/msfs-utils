use crate::DynError;
use crate::network::RelayClient;
use crate::protocol::{self, AircraftState};
use msfs_replay::{Pose, Recording};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::time::Instant;

pub async fn run(server: SocketAddr, path: &Path, count: usize) -> Result<(), DynError> {
    let recording = Recording::from_csv_path(path)?;
    let mut clients = Vec::with_capacity(count);
    for index in 0..count {
        let recording = recording.clone();
        clients.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(index as u64 * 50)).await;
            run_client(server, recording).await
        }));
    }

    for client in clients {
        client.await??;
    }
    Ok(())
}

async fn run_client(server: SocketAddr, recording: Recording) -> Result<(), DynError> {
    let client = RelayClient::connect(server).await?;
    eprintln!("Replay client {} connected.", client.user_id());
    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut updates = tokio::time::interval(Duration::from_millis(33));
    updates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buffer = vec![0; protocol::MAX_PACKET_SIZE];

    loop {
        tokio::select! {
            _ = updates.tick() => {
                let elapsed = started.elapsed().as_secs_f64();
                let finished = elapsed >= recording.duration_seconds();
                let pose = recording
                    .sample(elapsed.min(recording.duration_seconds()))
                    .expect("a validated recording always has a final sample");
                sequence = sequence.wrapping_add(1);
                client
                    .send_update(sequence, elapsed, state_from_pose(pose))
                    .await?;
                if finished {
                    client.leave().await?;
                    return Ok(());
                }
            }
            snapshot = client.receive_snapshot(&mut buffer) => {
                let _ = snapshot?;
            }
        }
    }
}

fn state_from_pose(pose: Pose) -> AircraftState {
    AircraftState {
        latitude: pose.latitude,
        longitude: pose.longitude,
        altitude: pose.altitude,
        heading: pose.heading,
        pitch: pose.pitch,
        bank: pose.bank,
        ..AircraftState::default()
    }
}
