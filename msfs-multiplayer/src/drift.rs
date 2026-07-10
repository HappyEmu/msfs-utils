const EARTH_RADIUS_FEET: f64 = 20_902_260.7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlignedDrift {
    pub north_feet: f64,
    pub east_feet: f64,
    pub along_track_feet: f64,
    pub cross_track_feet: f64,
    pub vertical_feet: f64,
    pub total_feet: f64,
}

/// Resolve actual-minus-expected position error in the expected local frame.
///
/// Positive along-track is ahead of the expected true heading. Positive
/// cross-track is to the right of that course, and positive vertical is above
/// the expected altitude.
pub fn aligned_drift(
    expected_latitude: f64,
    expected_longitude: f64,
    expected_altitude: f64,
    expected_heading: f64,
    actual_latitude: f64,
    actual_longitude: f64,
    actual_altitude: f64,
) -> AlignedDrift {
    let expected_latitude_radians = expected_latitude.to_radians();
    let north_feet = (actual_latitude - expected_latitude).to_radians() * EARTH_RADIUS_FEET;
    let east_feet = (actual_longitude - expected_longitude).to_radians()
        * EARTH_RADIUS_FEET
        * expected_latitude_radians.cos();
    let heading = expected_heading.to_radians();
    let along_track_feet = north_feet * heading.cos() + east_feet * heading.sin();
    let cross_track_feet = east_feet * heading.cos() - north_feet * heading.sin();
    let vertical_feet = actual_altitude - expected_altitude;

    AlignedDrift {
        north_feet,
        east_feet,
        along_track_feet,
        cross_track_feet,
        vertical_feet,
        total_feet: north_feet.hypot(east_feet).hypot(vertical_feet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_course_relative_components_and_preserves_signs() {
        let north = 100.0 / EARTH_RADIUS_FEET;
        let east = 25.0 / EARTH_RADIUS_FEET;
        let drift = aligned_drift(
            0.0,
            0.0,
            1_000.0,
            0.0,
            north.to_degrees(),
            east.to_degrees(),
            1_010.0,
        );
        assert!((drift.along_track_feet - 100.0).abs() < 1e-9);
        assert!((drift.cross_track_feet - 25.0).abs() < 1e-9);
        assert_eq!(drift.vertical_feet, 10.0);

        let eastbound = aligned_drift(
            0.0,
            0.0,
            1_000.0,
            90.0,
            north.to_degrees(),
            east.to_degrees(),
            990.0,
        );
        assert!((eastbound.along_track_feet - 25.0).abs() < 1e-9);
        assert!((eastbound.cross_track_feet + 100.0).abs() < 1e-9);
        assert_eq!(eastbound.vertical_feet, -10.0);
    }
}
