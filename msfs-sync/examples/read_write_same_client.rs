#[cfg(windows)]
mod windows {
    use msfs_sync::{
        Error, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER, SimConnect, data_definition,
    };
    use std::time::Duration;

    #[data_definition]
    #[derive(Debug)]
    struct SharedValue {
        #[name = "L:MSFS_SYNC_READ_WRITE_VALUE"]
        #[unit = "number"]
        value: f64,
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimConnect::open("SYNC READ WRITE")?;
        let updates =
            sim.subscribe::<SharedValue>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)?;

        let writer = sim.clone();
        std::thread::scope(|scope| -> Result<(), Error> {
            let _writer = scope.spawn(move || write_values(writer));
            for value in updates {
                println!("Read: {:?}", value?);
            }
            Ok(())
        })?;
        Ok(())
    }

    fn write_values(sim: SimConnect) -> Result<(), Error> {
        let mut value = 0.0;
        loop {
            value += 1.0;
            sim.set_data_on_sim_object(SIMCONNECT_OBJECT_ID_USER, &SharedValue { value })?;
            println!("Wrote: {value}");
            std::thread::sleep(Duration::from_secs(1));
        }
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
