use msfs_sync::data_definition;

#[data_definition]
#[derive(Debug)]
pub struct Data {
    #[name = "RADIO HEIGHT"]
    #[unit = "Feet"]
    #[epsilon = 0.01]
    height: f64,
    #[name = "AIRSPEED INDICATED"]
    #[unit = "Knots"]
    #[epsilon = 0.01]
    airspeed: f64,
}

#[data_definition]
#[derive(Debug)]
pub struct Controls {
    #[name = "ELEVATOR POSITION"]
    #[unit = "Position"]
    elevator: f64,
    #[name = "AILERON POSITION"]
    #[unit = "Position"]
    ailerons: f64,
    #[name = "RUDDER POSITION"]
    #[unit = "Position"]
    rudder: f64,
}

#[data_definition]
#[derive(Debug)]
pub struct Throttle(
    #[name = "GENERAL ENG THROTTLE LEVER POSITION:1"]
    #[unit = "Percent"]
    f64,
    #[name = "GENERAL ENG THROTTLE LEVER POSITION:2"]
    #[unit = "Percent"]
    f64,
);
