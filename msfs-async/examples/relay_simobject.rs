#[cfg(windows)]
mod windows {
    use futures_util::StreamExt;
    use msfs_async::{AsyncSimConnect, FreezeState, RecurringPeriod, data_definition};
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

    #[tokio::main]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let (source_id, target_id) = object_ids()?;
        eprintln!("Relaying object {source_id} onto {target_id}.");

        let sim = AsyncSimConnect::open("ASYNC SIMOBJECT RELAY").await?;
        let mut exceptions = sim.exceptions()?;
        sim.release_ai_control(target_id).await?;
        sim.set_freeze(target_id, FreezeState::ALL).await?;
        let mut frames = sim
            .subscribe_with_capacity::<FlightData>(source_id, RecurringPeriod::SimFrame, 1)
            .await?;

        loop {
            tokio::select! {
                frame = frames.next() => {
                    let Some(frame) = frame else {
                        break;
                    };
                    sim.set_data_on_sim_object(target_id, &frame?).await?;
                }
                exception = exceptions.next() => {
                    return match exception {
                        Some(exception) => Err(Box::new(exception) as Box<dyn std::error::Error>),
                        None => Err(Box::new(msfs_async::Error::DriverStopped)),
                    };
                }
            }
        }

        sim.set_freeze(target_id, FreezeState::NONE).await?;
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
            "usage: relay_simobject <source-object-id> <target-object-id> (the user aircraft ID is normally 0)",
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
