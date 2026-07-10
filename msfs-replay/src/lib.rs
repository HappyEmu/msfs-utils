//! Timestamped aircraft-pose recordings with fixed-rate interpolation.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// A writable aircraft pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub pitch: f64,
    pub bank: f64,
    pub heading: f64,
}

impl Pose {
    /// Interpolate between two poses.
    ///
    /// Position uses linear interpolation with wrapped longitude, while
    /// attitude uses quaternion spherical interpolation. `amount` is normally
    /// between zero and one.
    pub fn interpolate(self, other: Self, amount: f64) -> Self {
        let attitude = Quaternion::from_euler(self.pitch, self.bank, self.heading)
            .slerp(
                Quaternion::from_euler(other.pitch, other.bank, other.heading),
                amount,
            )
            .to_euler();

        Self {
            latitude: lerp(self.latitude, other.latitude, amount),
            longitude: interpolate_degrees(self.longitude, other.longitude, amount, -180.0),
            altitude: lerp(self.altitude, other.altitude, amount),
            pitch: attitude.pitch,
            bank: attitude.bank,
            heading: attitude.heading,
        }
    }

    fn is_finite(self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && self.altitude.is_finite()
            && self.pitch.is_finite()
            && self.bank.is_finite()
            && self.heading.is_finite()
    }
}

/// A timestamped aircraft pose, measured in seconds from an arbitrary origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedPose {
    pub seconds: f64,
    pub pose: Pose,
}

/// Linear aircraft velocities in feet per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity {
    pub world_x: f64,
    pub world_y: f64,
    pub world_z: f64,
    pub body_x: f64,
    pub body_y: f64,
    pub body_z: f64,
}

impl Velocity {
    fn interpolate(self, other: Self, amount: f64) -> Self {
        Self {
            world_x: lerp(self.world_x, other.world_x, amount),
            world_y: lerp(self.world_y, other.world_y, amount),
            world_z: lerp(self.world_z, other.world_z, amount),
            body_x: lerp(self.body_x, other.body_x, amount),
            body_y: lerp(self.body_y, other.body_y, amount),
            body_z: lerp(self.body_z, other.body_z, amount),
        }
    }

    fn is_finite(self) -> bool {
        [
            self.world_x,
            self.world_y,
            self.world_z,
            self.body_x,
            self.body_y,
            self.body_z,
        ]
        .into_iter()
        .all(f64::is_finite)
    }
}

/// An interpolated pose and its corresponding linear velocities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub pose: Pose,
    pub velocity: Velocity,
}

/// A validated recording with timestamps normalized to begin at zero.
#[derive(Clone, Debug)]
pub struct Recording {
    samples: Vec<TimedPose>,
    velocities: Vec<Velocity>,
}

impl Recording {
    /// Construct a recording from timestamped samples.
    pub fn new(samples: Vec<TimedPose>) -> Result<Self, RecordingError> {
        let velocities = vec![Velocity::default(); samples.len()];
        Self::new_with_velocities(samples, velocities)
    }

    fn new_with_velocities(
        mut samples: Vec<TimedPose>,
        velocities: Vec<Velocity>,
    ) -> Result<Self, RecordingError> {
        if samples.len() < 2 {
            return Err(RecordingError::NotEnoughSamples);
        }

        let origin = samples[0].seconds;
        if !origin.is_finite() || !samples[0].pose.is_finite() || !velocities[0].is_finite() {
            return Err(RecordingError::InvalidSample { index: 0 });
        }

        for (index, pair) in samples.windows(2).enumerate() {
            let next_index = index + 1;
            if !pair[1].seconds.is_finite()
                || !pair[1].pose.is_finite()
                || !velocities[next_index].is_finite()
            {
                return Err(RecordingError::InvalidSample { index: next_index });
            }
            if pair[1].seconds <= pair[0].seconds {
                return Err(RecordingError::NonIncreasingTimestamp { index: next_index });
            }
        }

        for (index, sample) in samples.iter_mut().enumerate() {
            sample.seconds -= origin;
            if !sample.seconds.is_finite() {
                return Err(RecordingError::InvalidSample { index });
            }
        }
        Ok(Self {
            samples,
            velocities,
        })
    }

    /// Read a recording from a CSV file.
    pub fn from_csv_path(path: impl AsRef<Path>) -> Result<Self, RecordingError> {
        let file = std::fs::File::open(path).map_err(RecordingError::Io)?;
        Self::from_csv_reader(file)
    }

    /// Read CSV data from any byte stream.
    ///
    /// The expected columns are the headerless 13-column format recorded by
    /// the original `fsmp` prototype: elapsed time, position, world velocity,
    /// attitude, and body velocity. Blank lines and lines beginning with `#`
    /// are ignored.
    pub fn from_csv_reader(reader: impl Read) -> Result<Self, RecordingError> {
        let mut samples = Vec::new();
        let mut velocities = Vec::new();
        for (line_index, line) in BufReader::new(reader).lines().enumerate() {
            let line_number = line_index + 1;
            let line = line.map_err(RecordingError::Io)?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let columns: Vec<_> = line.split(',').map(str::trim).collect();
            if columns.len() != 13 {
                return Err(RecordingError::InvalidCsv {
                    line: line_number,
                    message: format!("expected 13 columns, found {}", columns.len()),
                });
            }

            let value = |index: usize| {
                columns[index]
                    .parse::<f64>()
                    .map_err(|error| RecordingError::InvalidCsv {
                        line: line_number,
                        message: format!("invalid column {}: {error}", index + 1),
                    })
            };
            samples.push(TimedPose {
                seconds: value(0)?,
                pose: Pose {
                    latitude: value(1)?,
                    longitude: value(2)?,
                    altitude: value(3)?,
                    pitch: value(7)?,
                    bank: value(8)?,
                    heading: value(9)?,
                },
            });
            velocities.push(Velocity {
                world_x: value(4)?,
                world_y: value(5)?,
                world_z: value(6)?,
                body_x: value(10)?,
                body_y: value(11)?,
                body_z: value(12)?,
            });
        }
        Self::new_with_velocities(samples, velocities)
    }

    /// Duration of the normalized recording in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.samples.last().map_or(0.0, |sample| sample.seconds)
    }

    /// Interpolate the pose at a normalized recording timestamp.
    ///
    /// Returns `None` after the end of the recording. Values at or before zero
    /// return the first pose.
    pub fn sample(&self, seconds: f64) -> Option<Pose> {
        self.sample_with_velocity(seconds).map(|sample| sample.pose)
    }

    /// Interpolate the pose and velocities at a normalized recording timestamp.
    pub fn sample_with_velocity(&self, seconds: f64) -> Option<Sample> {
        if !seconds.is_finite() || seconds > self.duration_seconds() {
            return None;
        }
        if seconds <= 0.0 {
            return Some(Sample {
                pose: self.samples.first()?.pose,
                velocity: self.velocities[0],
            });
        }

        let upper = self
            .samples
            .partition_point(|sample| sample.seconds <= seconds);
        if upper == self.samples.len() {
            return Some(Sample {
                pose: self.samples.last()?.pose,
                velocity: *self.velocities.last()?,
            });
        }

        let before = self.samples[upper - 1];
        let after = self.samples[upper];
        let amount = (seconds - before.seconds) / (after.seconds - before.seconds);
        Some(Sample {
            pose: before.pose.interpolate(after.pose, amount),
            velocity: self.velocities[upper - 1].interpolate(self.velocities[upper], amount),
        })
    }
}

/// Errors produced while loading or validating a recording.
#[non_exhaustive]
#[derive(Debug)]
pub enum RecordingError {
    Io(io::Error),
    InvalidCsv { line: usize, message: String },
    NotEnoughSamples,
    InvalidSample { index: usize },
    NonIncreasingTimestamp { index: usize },
}

impl fmt::Display for RecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not read recording: {error}"),
            Self::InvalidCsv { line, message } => {
                write!(formatter, "invalid recording CSV at line {line}: {message}")
            }
            Self::NotEnoughSamples => {
                formatter.write_str("a recording requires at least 2 samples")
            }
            Self::InvalidSample { index } => {
                write!(
                    formatter,
                    "recording sample {index} contains a non-finite value"
                )
            }
            Self::NonIncreasingTimestamp { index } => write!(
                formatter,
                "recording timestamp at sample {index} is not strictly increasing"
            ),
        }
    }
}

impl std::error::Error for RecordingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct EulerAngles {
    pitch: f64,
    bank: f64,
    heading: f64,
}

#[derive(Clone, Copy)]
struct Quaternion {
    w: f64,
    x: f64,
    y: f64,
    z: f64,
}

impl Quaternion {
    fn from_euler(pitch: f64, bank: f64, heading: f64) -> Self {
        let (roll_sin, roll_cos) = (bank.rem_euclid(360.0).to_radians() * 0.5).sin_cos();
        let (pitch_sin, pitch_cos) = (pitch.rem_euclid(360.0).to_radians() * 0.5).sin_cos();
        let (yaw_sin, yaw_cos) = (heading.rem_euclid(360.0).to_radians() * 0.5).sin_cos();

        Self {
            w: roll_cos * pitch_cos * yaw_cos + roll_sin * pitch_sin * yaw_sin,
            x: roll_sin * pitch_cos * yaw_cos - roll_cos * pitch_sin * yaw_sin,
            y: roll_cos * pitch_sin * yaw_cos + roll_sin * pitch_cos * yaw_sin,
            z: roll_cos * pitch_cos * yaw_sin - roll_sin * pitch_sin * yaw_cos,
        }
        .normalized()
    }

    fn to_euler(self) -> EulerAngles {
        let pitch_sine = 2.0 * (self.w * self.y - self.z * self.x);
        EulerAngles {
            bank: (2.0 * (self.w * self.x + self.y * self.z))
                .atan2(1.0 - 2.0 * (self.x * self.x + self.y * self.y))
                .to_degrees(),
            pitch: pitch_sine.clamp(-1.0, 1.0).asin().to_degrees(),
            heading: (2.0 * (self.w * self.z + self.x * self.y))
                .atan2(1.0 - 2.0 * (self.y * self.y + self.z * self.z))
                .to_degrees()
                .rem_euclid(360.0),
        }
    }

    fn slerp(self, mut other: Self, amount: f64) -> Self {
        let mut dot = self.dot(other);
        if dot < 0.0 {
            other = other.scaled(-1.0);
            dot = -dot;
        }

        if dot > 0.9995 {
            return self
                .scaled(1.0 - amount)
                .add(other.scaled(amount))
                .normalized();
        }

        let angle = dot.clamp(-1.0, 1.0).acos();
        let scale = angle.sin();
        self.scaled(((1.0 - amount) * angle).sin() / scale)
            .add(other.scaled((amount * angle).sin() / scale))
            .normalized()
    }

    fn dot(self, other: Self) -> f64 {
        self.w * other.w + self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn scaled(self, factor: f64) -> Self {
        Self {
            w: self.w * factor,
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            w: self.w + other.w,
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }

    fn normalized(self) -> Self {
        let length = self.dot(self).sqrt();
        self.scaled(length.recip())
    }
}

fn lerp(start: f64, end: f64, amount: f64) -> f64 {
    start + (end - start) * amount
}

fn interpolate_degrees(start: f64, end: f64, amount: f64, minimum: f64) -> f64 {
    let difference = (end - start + 180.0).rem_euclid(360.0) - 180.0;
    (start + difference * amount - minimum).rem_euclid(360.0) + minimum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(longitude: f64, pitch: f64, bank: f64, heading: f64) -> Pose {
        Pose {
            latitude: 10.0,
            longitude,
            altitude: 1_000.0,
            pitch,
            bank,
            heading,
        }
    }

    #[test]
    fn parses_and_normalizes_csv_timestamps() {
        let recording = Recording::from_csv_reader(
            b"10,1,2,3,40,50,60,4,5,6,70,80,90\n\
              12,2,3,4,42,52,62,5,6,7,72,82,92\n"
                .as_slice(),
        )
        .unwrap();

        assert_eq!(recording.duration_seconds(), 2.0);
        let first = recording.sample_with_velocity(0.0).unwrap();
        assert_eq!(first.pose.pitch, 4.0);
        assert_eq!(first.pose.bank, 5.0);
        assert_eq!(first.pose.heading, 6.0);
        let middle = recording.sample_with_velocity(1.0).unwrap();
        assert_eq!(middle.pose.latitude, 1.5);
        assert_eq!(middle.velocity.world_x, 41.0);
        assert_eq!(middle.velocity.body_z, 91.0);
    }

    #[test]
    fn interpolates_position_and_wraps_longitude() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(179.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 2.0,
                pose: Pose {
                    latitude: 12.0,
                    longitude: -179.0,
                    altitude: 1_200.0,
                    pitch: 0.0,
                    bank: 0.0,
                    heading: 0.0,
                },
            },
        ])
        .unwrap();

        let middle = recording.sample(1.0).unwrap();
        assert!((middle.latitude - 11.0).abs() < 1e-9);
        assert!((middle.altitude - 1_100.0).abs() < 1e-9);
        assert!((middle.longitude.abs() - 180.0).abs() < 1e-9);
    }

    #[test]
    fn quaternion_slerp_takes_short_heading_path() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(0.0, 0.0, 0.0, 350.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 0.0, 0.0, 10.0),
            },
        ])
        .unwrap();

        let heading = recording.sample(0.5).unwrap().heading;
        assert!(heading < 1e-9 || (360.0 - heading) < 1e-9);
    }

    #[test]
    fn quaternion_slerp_interpolates_pitch() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 30.0, 0.0, 0.0),
            },
        ])
        .unwrap();

        assert!((recording.sample(0.5).unwrap().pitch - 15.0).abs() < 1e-9);
    }

    #[test]
    fn returns_none_after_recording() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
        ])
        .unwrap();

        assert!(recording.sample(1.001).is_none());
    }

    #[test]
    fn rejects_invalid_csv_shape_and_values() {
        let shape = Recording::from_csv_reader(b"0,1,2\n1,2,3\n".as_slice()).unwrap_err();
        assert!(matches!(shape, RecordingError::InvalidCsv { line: 1, .. }));

        let value = Recording::from_csv_reader(
            b"0,1,2,3,4,5,nope,7,8,9,10,11,12\n\
              1,2,3,4,5,6,7,8,9,10,11,12,13\n"
                .as_slice(),
        )
        .unwrap_err();
        assert!(matches!(value, RecordingError::InvalidCsv { line: 1, .. }));
    }

    #[test]
    fn rejects_non_finite_and_non_increasing_samples() {
        let non_finite = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(f64::NAN, 0.0, 0.0, 0.0),
            },
        ])
        .unwrap_err();
        assert!(matches!(
            non_finite,
            RecordingError::InvalidSample { index: 1 }
        ));

        let duplicate = Recording::new(vec![
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 0.0, 0.0, 0.0),
            },
        ])
        .unwrap_err();
        assert!(matches!(
            duplicate,
            RecordingError::NonIncreasingTimestamp { index: 1 }
        ));
    }

    #[test]
    fn sampling_preserves_both_endpoints() {
        let first = pose(10.0, -5.0, 2.0, 350.0);
        let last = pose(20.0, 5.0, -2.0, 10.0);
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 10.0,
                pose: first,
            },
            TimedPose {
                seconds: 12.0,
                pose: last,
            },
        ])
        .unwrap();

        assert_eq!(recording.sample(0.0), Some(first));
        assert_eq!(recording.sample(2.0), Some(last));
        assert_eq!(recording.sample(-1.0), Some(first));
        assert_eq!(recording.sample(f64::NAN), None);
    }

    #[test]
    fn wraps_longitude_in_both_directions() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(-179.0, 0.0, 0.0, 0.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(179.0, 0.0, 0.0, 0.0),
            },
        ])
        .unwrap();

        let middle = recording.sample(0.5).unwrap().longitude;
        assert!((middle.abs() - 180.0).abs() < 1e-9);
    }

    #[test]
    fn combined_rotation_near_gimbal_lock_remains_finite() {
        let recording = Recording::new(vec![
            TimedPose {
                seconds: 0.0,
                pose: pose(0.0, 89.9, 45.0, 350.0),
            },
            TimedPose {
                seconds: 1.0,
                pose: pose(0.0, 90.1, -45.0, 10.0),
            },
        ])
        .unwrap();

        assert!(recording.sample(0.5).unwrap().is_finite());
    }
}
