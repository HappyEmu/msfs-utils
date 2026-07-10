use crate::DynError;
use crate::network::RelayClient;
use crate::protocol::{self, AircraftState};
use msfs_replay::{Recording, Sample};
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
                let sample = recording
                    .sample_with_velocity(elapsed.min(recording.duration_seconds()))
                    .expect("a validated recording always has a final sample");
                sequence = sequence.wrapping_add(1);
                client
                    .send_update(sequence, elapsed, state_from_sample(sample))
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

fn state_from_sample(sample: Sample) -> AircraftState {
    AircraftState {
        latitude: sample.pose.latitude,
        longitude: sample.pose.longitude,
        altitude: sample.pose.altitude,
        heading: sample.pose.heading,
        pitch: sample.pose.pitch,
        bank: sample.pose.bank,
        velocity_world_x: sample.velocity.world_x,
        velocity_world_y: sample.velocity.world_y,
        velocity_world_z: sample.velocity.world_z,
        velocity_body_x: sample.velocity.body_x,
        velocity_body_y: sample.velocity.body_y,
        velocity_body_z: sample.velocity.body_z,
        ..AircraftState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use msfs_replay::{Pose, Velocity};

    #[test]
    fn replay_samples_preserve_recorded_velocities() {
        let state = state_from_sample(Sample {
            pose: Pose {
                latitude: 47.0,
                longitude: 8.0,
                altitude: 1_400.0,
                pitch: 1.0,
                bank: 2.0,
                heading: 3.0,
            },
            velocity: Velocity {
                world_x: 4.0,
                world_y: 5.0,
                world_z: 6.0,
                body_x: 7.0,
                body_y: 8.0,
                body_z: 9.0,
            },
        });

        assert_eq!(state.velocity_world_x, 4.0);
        assert_eq!(state.velocity_world_y, 5.0);
        assert_eq!(state.velocity_world_z, 6.0);
        assert_eq!(state.velocity_body_x, 7.0);
        assert_eq!(state.velocity_body_y, 8.0);
        assert_eq!(state.velocity_body_z, 9.0);
    }
}
