use crate::protocol::{AircraftState, AircraftUpdate};
use msfs_replay::Pose;
use std::collections::VecDeque;
use std::time::Duration;

const DEFAULT_MAX_SAMPLES: usize = 256;
const CLOCK_OFFSET_WINDOW_SECONDS: f64 = 10.0;

/// A timestamped jitter buffer for one remote aircraft.
///
/// Sender timestamps are mapped onto the receiver's monotonic clock using the
/// smallest arrival offset in a recent window. Sampling runs behind that
/// estimated clock by the configured delay, leaving time for jittered packets
/// to arrive without locking the clock estimate to a session-old minimum.
pub struct InterpolationBuffer {
    delay_seconds: f64,
    max_samples: usize,
    clock_offset_seconds: Option<f64>,
    clock_offsets: VecDeque<(f64, f64)>,
    last_sequence: Option<u64>,
    samples: VecDeque<AircraftUpdate>,
}

impl InterpolationBuffer {
    /// Create a buffer with a fixed playback delay.
    pub fn new(delay: Duration) -> Self {
        Self {
            delay_seconds: delay.as_secs_f64(),
            max_samples: DEFAULT_MAX_SAMPLES,
            clock_offset_seconds: None,
            clock_offsets: VecDeque::new(),
            last_sequence: None,
            samples: VecDeque::new(),
        }
    }

    /// Insert a newly arrived update.
    ///
    /// Returns `false` for stale sequences, non-increasing timestamps, or
    /// non-finite state.
    pub fn push(&mut self, update: AircraftUpdate, arrived_at: Duration) -> bool {
        if !update.timestamp_seconds.is_finite()
            || !update.state.is_finite()
            || self
                .last_sequence
                .is_some_and(|sequence| update.sequence <= sequence)
            || self
                .samples
                .back()
                .is_some_and(|sample| update.timestamp_seconds <= sample.timestamp_seconds)
        {
            return false;
        }

        let arrival_seconds = arrived_at.as_secs_f64();
        let offset = arrival_seconds - update.timestamp_seconds;
        self.clock_offsets.push_back((arrival_seconds, offset));
        let window_start = arrival_seconds - CLOCK_OFFSET_WINDOW_SECONDS;
        while self
            .clock_offsets
            .front()
            .is_some_and(|(arrival, _)| *arrival < window_start)
        {
            self.clock_offsets.pop_front();
        }
        self.clock_offset_seconds = self
            .clock_offsets
            .iter()
            .map(|(_, offset)| *offset)
            .reduce(f64::min);
        self.last_sequence = Some(update.sequence);
        self.samples.push_back(update);
        while self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }
        true
    }

    /// Sample the buffered aircraft state for the receiver's current time.
    ///
    /// The oldest samples which can no longer bracket a future playback time
    /// are discarded. Before the first sample or after an underrun, the nearest
    /// available state is held.
    pub fn sample(&mut self, now: Duration) -> Option<AircraftState> {
        let timestamp = self.playback_timestamp(now)?;
        while self.samples.len() >= 3 && self.samples[1].timestamp_seconds <= timestamp {
            self.samples.pop_front();
        }

        let first = self.samples.front()?;
        if timestamp <= first.timestamp_seconds {
            return Some(first.state);
        }
        let Some(second) = self.samples.get(1) else {
            return Some(first.state);
        };
        if timestamp >= second.timestamp_seconds {
            return Some(second.state);
        }

        let amount = (timestamp - first.timestamp_seconds)
            / (second.timestamp_seconds - first.timestamp_seconds);
        Some(interpolate_state(first.state, second.state, amount))
    }

    /// Return the sender timestamp currently targeted by playback.
    pub fn playback_timestamp(&self, now: Duration) -> Option<f64> {
        self.clock_offset_seconds
            .map(|offset| now.as_secs_f64() - offset - self.delay_seconds)
    }

    /// Number of samples currently retained.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples have been accepted.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

fn interpolate_state(start: AircraftState, end: AircraftState, amount: f64) -> AircraftState {
    let amount = amount.clamp(0.0, 1.0);
    let pose = Pose {
        latitude: start.latitude,
        longitude: start.longitude,
        altitude: start.altitude,
        pitch: start.pitch,
        bank: start.bank,
        heading: start.heading,
    }
    .interpolate(
        Pose {
            latitude: end.latitude,
            longitude: end.longitude,
            altitude: end.altitude,
            pitch: end.pitch,
            bank: end.bank,
            heading: end.heading,
        },
        amount,
    );

    AircraftState {
        latitude: pose.latitude,
        longitude: pose.longitude,
        altitude: pose.altitude,
        indicated_airspeed: lerp(start.indicated_airspeed, end.indicated_airspeed, amount),
        heading: pose.heading,
        pitch: pose.pitch,
        bank: pose.bank,
        velocity_world_x: lerp(start.velocity_world_x, end.velocity_world_x, amount),
        velocity_world_y: lerp(start.velocity_world_y, end.velocity_world_y, amount),
        velocity_world_z: lerp(start.velocity_world_z, end.velocity_world_z, amount),
        velocity_body_x: lerp(start.velocity_body_x, end.velocity_body_x, amount),
        velocity_body_y: lerp(start.velocity_body_y, end.velocity_body_y, amount),
        velocity_body_z: lerp(start.velocity_body_z, end.velocity_body_z, amount),
        on_ground: if amount < 0.5 {
            start.on_ground
        } else {
            end.on_ground
        },
    }
}

fn lerp(start: f64, end: f64, amount: f64) -> f64 {
    start + (end - start) * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(
        sequence: u64,
        timestamp_seconds: f64,
        latitude: f64,
        heading: f64,
    ) -> AircraftUpdate {
        AircraftUpdate {
            user_id: 1,
            sequence,
            timestamp_seconds,
            state: AircraftState {
                latitude,
                heading,
                ..AircraftState::default()
            },
        }
    }

    #[test]
    fn interpolates_on_the_sender_timeline_behind_arrival_time() {
        let mut buffer = InterpolationBuffer::new(Duration::from_millis(100));
        assert!(buffer.push(update(1, 0.0, 0.0, 350.0), Duration::from_millis(50)));
        assert!(buffer.push(update(2, 0.1, 10.0, 10.0), Duration::from_millis(160)));

        let state = buffer.sample(Duration::from_millis(200)).unwrap();
        assert!((state.latitude - 5.0).abs() < 1e-9);
        assert!(state.heading < 1e-9 || 360.0 - state.heading < 1e-9);
    }

    #[test]
    fn rejects_stale_updates_and_holds_the_newest_state_on_underrun() {
        let mut buffer = InterpolationBuffer::new(Duration::from_millis(100));
        assert!(buffer.push(update(10, 1.0, 10.0, 20.0), Duration::from_millis(1_050)));
        assert!(!buffer.push(update(9, 1.1, 11.0, 21.0), Duration::from_millis(1_150)));
        assert!(buffer.push(update(11, 1.1, 12.0, 22.0), Duration::from_millis(1_160)));

        let state = buffer.sample(Duration::from_secs(10)).unwrap();
        assert_eq!(state.latitude, 12.0);
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn clock_offset_forgets_a_session_old_minimum() {
        let mut buffer = InterpolationBuffer::new(Duration::from_millis(100));
        assert!(buffer.push(update(1, 0.0, 0.0, 0.0), Duration::from_millis(50)));
        assert!(
            (buffer
                .playback_timestamp(Duration::from_millis(200))
                .unwrap()
                - 0.05)
                .abs()
                < 1e-9
        );

        assert!(buffer.push(update(2, 20.0, 1.0, 0.0), Duration::from_millis(20_100)));
        assert!(
            (buffer
                .playback_timestamp(Duration::from_millis(20_200))
                .unwrap()
                - 20.0)
                .abs()
                < 1e-9
        );
    }
}
