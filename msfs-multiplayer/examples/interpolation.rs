use msfs_multiplayer::interpolation::InterpolationBuffer;
use msfs_multiplayer::protocol::{AircraftState, AircraftUpdate};
use std::collections::VecDeque;
use std::time::Duration;

struct Arrival {
    at: Duration,
    update: AircraftUpdate,
}

fn main() {
    let mut arrivals = simulated_network();
    let mut buffer = InterpolationBuffer::new(Duration::from_millis(150));
    let render_interval = Duration::from_secs_f64(1.0 / 60.0);
    let mut render_time = Duration::ZERO;

    println!("render_seconds,target_sender_seconds,latitude,heading,buffered_samples");
    while render_time <= Duration::from_millis(1_300) {
        while arrivals
            .front()
            .is_some_and(|arrival| arrival.at <= render_time)
        {
            let arrival = arrivals.pop_front().unwrap();
            buffer.push(arrival.update, arrival.at);
        }

        if let Some(state) = buffer.sample(render_time) {
            println!(
                "{:.6},{:.6},{:.6},{:.3},{}",
                render_time.as_secs_f64(),
                buffer.playback_timestamp(render_time).unwrap(),
                state.latitude,
                state.heading,
                buffer.len(),
            );
        }
        render_time += render_interval;
    }
}

fn simulated_network() -> VecDeque<Arrival> {
    let jitter_ms = [35, 70, 42, 63, 38, 76, 45, 58, 40, 68, 36];
    jitter_ms
        .into_iter()
        .enumerate()
        .map(|(index, jitter_ms)| {
            let timestamp_seconds = index as f64 * 0.1;
            Arrival {
                at: Duration::from_secs_f64(timestamp_seconds) + Duration::from_millis(jitter_ms),
                update: AircraftUpdate {
                    user_id: 1,
                    sequence: index as u64 + 1,
                    timestamp_seconds,
                    state: AircraftState {
                        latitude: 47.0 + timestamp_seconds * 0.001,
                        longitude: 8.0 + timestamp_seconds * 0.001,
                        altitude: 1_500.0 + timestamp_seconds * 100.0,
                        heading: (350.0 + timestamp_seconds * 20.0).rem_euclid(360.0),
                        velocity_body_z: 150.0,
                        ..AircraftState::default()
                    },
                },
            }
        })
        .collect()
}
