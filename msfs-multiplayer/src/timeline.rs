use crate::protocol::AircraftState;
use msfs_multiplayer::interpolation::interpolate_state;
use std::collections::VecDeque;

const TARGET_HISTORY_SECONDS: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetSample {
    pub receiver_absolute_time: f64,
    pub playback_timestamp_seconds: f64,
    pub expected: AircraftState,
    pub buffer_depth: usize,
    pub underrun: bool,
}

#[derive(Default)]
pub struct TargetTimeline {
    samples: VecDeque<TargetSample>,
}

impl TargetTimeline {
    /// Record a rendered target. Returns `true` if the simulator clock moved
    /// backwards and the old timeline was discarded.
    pub fn record(&mut self, sample: TargetSample) -> bool {
        if !sample.receiver_absolute_time.is_finite() {
            return false;
        }
        let mut reset = false;
        if let Some(last) = self.samples.back() {
            if sample.receiver_absolute_time < last.receiver_absolute_time {
                self.samples.clear();
                reset = true;
            } else if sample.receiver_absolute_time == last.receiver_absolute_time {
                self.samples.pop_back();
                self.samples.push_back(sample);
                return false;
            }
        }

        self.samples.push_back(sample);
        let cutoff = sample.receiver_absolute_time - TARGET_HISTORY_SECONDS;
        while self
            .samples
            .get(1)
            .is_some_and(|entry| entry.receiver_absolute_time < cutoff)
        {
            self.samples.pop_front();
        }
        reset
    }

    pub fn sample(&self, absolute_time: f64) -> Option<TargetSample> {
        if let Some(entry) = self
            .samples
            .iter()
            .find(|entry| entry.receiver_absolute_time == absolute_time)
        {
            return Some(*entry);
        }
        let (start, end) =
            self.samples
                .iter()
                .zip(self.samples.iter().skip(1))
                .find(|(start, end)| {
                    start.receiver_absolute_time < absolute_time
                        && absolute_time < end.receiver_absolute_time
                })?;
        let duration = end.receiver_absolute_time - start.receiver_absolute_time;
        if duration <= 0.0 {
            return None;
        }
        let amount = (absolute_time - start.receiver_absolute_time) / duration;
        Some(TargetSample {
            receiver_absolute_time: absolute_time,
            playback_timestamp_seconds: start.playback_timestamp_seconds
                + (end.playback_timestamp_seconds - start.playback_timestamp_seconds) * amount,
            expected: interpolate_state(start.expected, end.expected, amount),
            buffer_depth: if amount < 0.5 {
                start.buffer_depth
            } else {
                end.buffer_depth
            },
            underrun: if amount < 0.5 {
                start.underrun
            } else {
                end.underrun
            },
        })
    }

    pub fn oldest_time(&self) -> Option<f64> {
        self.samples
            .front()
            .map(|sample| sample.receiver_absolute_time)
    }

    pub fn latest_time(&self) -> Option<f64> {
        self.samples
            .back()
            .map(|sample| sample.receiver_absolute_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(time: f64, latitude: f64) -> TargetSample {
        TargetSample {
            receiver_absolute_time: time,
            playback_timestamp_seconds: time - 0.5,
            expected: AircraftState {
                latitude,
                ..AircraftState::default()
            },
            buffer_depth: 15,
            underrun: false,
        }
    }

    #[test]
    fn interpolates_at_the_measurement_absolute_time() {
        let mut timeline = TargetTimeline::default();
        assert!(!timeline.record(sample(10.0, 20.0)));
        assert!(!timeline.record(sample(10.1, 30.0)));

        let aligned = timeline.sample(10.025).unwrap();
        assert!((aligned.expected.latitude - 22.5).abs() < 1e-9);
        assert!((aligned.playback_timestamp_seconds - 9.525).abs() < 1e-9);
        assert!(timeline.sample(10.2).is_none());
    }

    #[test]
    fn replaces_paused_frames_and_resets_after_clock_rewind() {
        let mut timeline = TargetTimeline::default();
        timeline.record(sample(10.0, 20.0));
        timeline.record(sample(10.0, 25.0));
        assert_eq!(timeline.sample(10.0).unwrap().expected.latitude, 25.0);

        assert!(timeline.record(sample(5.0, 40.0)));
        assert_eq!(timeline.oldest_time(), Some(5.0));
        assert_eq!(timeline.latest_time(), Some(5.0));
        assert!(timeline.sample(10.0).is_none());
    }
}
