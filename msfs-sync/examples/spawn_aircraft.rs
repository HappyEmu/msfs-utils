#[cfg(windows)]
mod windows {
    use msfs_sync::{
        Error, FreezeState, InitialPosition, SIMCONNECT_OBJECT_ID_USER, SimConnect, data_definition,
    };
    use std::io;
    use std::time::{Duration, Instant};

    #[data_definition]
    #[derive(Debug)]
    struct UserAircraft {
        #[name = "PLANE LATITUDE"]
        #[unit = "Degrees"]
        latitude: f64,
        #[name = "PLANE LONGITUDE"]
        #[unit = "Degrees"]
        longitude: f64,
        #[name = "PLANE ALTITUDE"]
        #[unit = "Feet"]
        altitude: f64,
        #[name = "PLANE PITCH DEGREES"]
        #[unit = "Degrees"]
        pitch: f64,
        #[name = "PLANE BANK DEGREES"]
        #[unit = "Degrees"]
        bank: f64,
        #[name = "PLANE HEADING DEGREES TRUE"]
        #[unit = "Degrees"]
        heading: f64,
        #[name = "SIM ON GROUND"]
        #[unit = "Bool"]
        on_ground: i64,
        #[name = "AIRSPEED INDICATED"]
        #[unit = "Knots"]
        airspeed: f64,
    }

    #[data_definition]
    #[derive(Debug)]
    struct DrivenPose {
        #[name = "PLANE LATITUDE"]
        #[unit = "Degrees"]
        latitude: f64,
        #[name = "PLANE LONGITUDE"]
        #[unit = "Degrees"]
        longitude: f64,
        #[name = "PLANE ALTITUDE"]
        #[unit = "Feet"]
        altitude: f64,
        #[name = "PLANE PITCH DEGREES"]
        #[unit = "Degrees"]
        pitch: f64,
        #[name = "PLANE BANK DEGREES"]
        #[unit = "Degrees"]
        bank: f64,
        #[name = "PLANE HEADING DEGREES TRUE"]
        #[unit = "Degrees"]
        heading: f64,
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let (container_title, tail_number) = args()?;
        let sim = SimConnect::open("SYNC AI AIRCRAFT LIFECYCLE")?;
        let mut exceptions = sim.exceptions()?;
        let user = sim.request_once::<UserAircraft>(SIMCONNECT_OBJECT_ID_USER)?;
        let mut pose = DrivenPose {
            latitude: user.latitude,
            longitude: user.longitude + 0.002,
            altitude: user.altitude,
            pitch: user.pitch,
            bank: user.bank,
            heading: user.heading,
        };
        let aircraft = sim.create_non_atc_aircraft(
            container_title,
            tail_number,
            InitialPosition {
                latitude: pose.latitude,
                longitude: pose.longitude,
                altitude: pose.altitude,
                pitch: pose.pitch,
                bank: pose.bank,
                heading: pose.heading,
                on_ground: user.on_ground != 0,
                airspeed: airspeed_knots(user.airspeed),
            },
        )?;
        let object_id = aircraft.object_id();
        eprintln!("Created AI aircraft with object ID {object_id}.");

        sim.release_ai_control(object_id)?;
        sim.set_freeze(object_id, FreezeState::ALL)?;

        let started = Instant::now();
        let duration = Duration::from_secs(5);
        let interval = Duration::from_secs_f64(1.0 / 60.0);
        while started.elapsed() < duration {
            if let Some(exception) = exceptions.try_recv() {
                return Err(Error::SimConnectException(exception).into());
            }
            pose.heading = (pose.heading + 0.1).rem_euclid(360.0);
            sim.set_data_on_sim_object(object_id, &pose)?;
            std::thread::sleep(interval);
        }

        sim.remove_object(object_id)?;
        eprintln!("Removed AI aircraft {object_id}.");
        Ok(())
    }

    fn airspeed_knots(value: f64) -> u32 {
        if value.is_finite() {
            value.clamp(0.0, u32::MAX as f64).round() as u32
        } else {
            0
        }
    }

    fn args() -> Result<(String, String), io::Error> {
        let mut values = std::env::args().skip(1);
        let container_title = values.next().ok_or_else(usage_error)?;
        let tail_number = values.next().unwrap_or_else(|| "MSFSUTILS".to_owned());
        if values.next().is_some() {
            return Err(usage_error());
        }
        Ok((container_title, tail_number))
    }

    fn usage_error() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: sync_spawn_aircraft <installed-container-title> [tail-number=MSFSUTILS]",
        )
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
