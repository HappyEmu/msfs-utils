const EARTH_RADIUS_FEET: f64 = 20_902_260.7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionDrift {
    pub horizontal_feet: f64,
    pub vertical_feet: f64,
    pub total_feet: f64,
}

pub fn position_drift(
    expected_latitude: f64,
    expected_longitude: f64,
    expected_altitude: f64,
    actual_latitude: f64,
    actual_longitude: f64,
    actual_altitude: f64,
) -> PositionDrift {
    let expected_latitude = expected_latitude.to_radians();
    let actual_latitude = actual_latitude.to_radians();
    let latitude_delta = actual_latitude - expected_latitude;
    let longitude_delta = (actual_longitude - expected_longitude).to_radians();
    let haversine = ((latitude_delta * 0.5).sin().powi(2)
        + expected_latitude.cos() * actual_latitude.cos() * (longitude_delta * 0.5).sin().powi(2))
    .clamp(0.0, 1.0);
    let central_angle = 2.0
        * haversine
            .sqrt()
            .atan2((1.0 - haversine).clamp(0.0, 1.0).sqrt());
    let horizontal_feet = EARTH_RADIUS_FEET * central_angle;
    let vertical_feet = actual_altitude - expected_altitude;

    PositionDrift {
        horizontal_feet,
        vertical_feet,
        total_feet: horizontal_feet.hypot(vertical_feet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_horizontal_and_vertical_drift() {
        let vertical = position_drift(47.0, 8.0, 1_000.0, 47.0, 8.0, 1_125.0);
        assert_eq!(vertical.horizontal_feet, 0.0);
        assert_eq!(vertical.vertical_feet, 125.0);
        assert_eq!(vertical.total_feet, 125.0);

        let horizontal = position_drift(0.0, 0.0, 1_000.0, 0.0, 1.0, 1_000.0);
        assert!((horizontal.horizontal_feet - 364_813.0).abs() < 1.0);
        assert_eq!(horizontal.vertical_feet, 0.0);
        assert_eq!(horizontal.total_feet, horizontal.horizontal_feet);
    }
}
