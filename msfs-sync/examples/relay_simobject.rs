#[cfg(windows)]
mod windows {
    use msfs_sync::{FreezeState, RecurringPeriod, SimConnect, data_definition};
    use std::io;

    #[data_definition]
    #[derive(Debug)]
    struct FlightData {
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
        let (source_id, target_id) = object_ids()?;
        eprintln!("Relaying object {source_id} onto {target_id}.");

        let sim = SimConnect::open("SYNC SIMOBJECT RELAY")?;
        let mut exceptions = sim.exceptions()?;
        sim.release_ai_control(target_id)?;
        sim.set_freeze(target_id, FreezeState::ALL)?;
        let frames =
            sim.subscribe_with_capacity::<FlightData>(source_id, RecurringPeriod::SimFrame, 1)?;

        for frame in frames {
            sim.set_data_on_sim_object(target_id, &frame?)?;

            if let Some(exception) = exceptions.try_recv() {
                return Err(Box::new(exception));
            }
        }

        sim.set_freeze(target_id, FreezeState::NONE)?;
        Ok(())
    }

    fn object_ids() -> Result<(u32, u32), Box<dyn std::error::Error>> {
        let mut args = std::env::args().skip(1);
        let source = args.next().ok_or_else(usage_error)?.parse()?;
        let target = args.next().ok_or_else(usage_error)?.parse()?;
        if args.next().is_some() {
            return Err(usage_error().into());
        }
        Ok((source, target))
    }

    fn usage_error() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: sync_relay_simobject <source-object-id> <target-object-id> (the user aircraft ID is normally 0)",
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
