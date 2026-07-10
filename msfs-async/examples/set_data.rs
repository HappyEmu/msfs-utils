#[cfg(windows)]
mod windows {
    use futures_util::StreamExt;
    use msfs_async::{AsyncSimConnect, SIMCONNECT_OBJECT_ID_USER, data_definition};

    #[data_definition]
    #[derive(Debug)]
    struct ExampleValue {
        // L variables are explicitly documented as readable and writable
        // through SimConnect and avoid changing an aircraft control.
        #[name = "L:MSFS_ASYNC_EXAMPLE_VALUE"]
        #[unit = "number"]
        value: f64,
    }

    #[tokio::main]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let sim = AsyncSimConnect::open("ASYNC SET DATA").await?;
        let mut exceptions = sim.exceptions()?;

        let observed = tokio::select! {
            result = async {
                sim.set_data_on_sim_object(
                    SIMCONNECT_OBJECT_ID_USER,
                    &ExampleValue { value: 42.0 },
                ).await?;

                sim.request_once::<ExampleValue>(SIMCONNECT_OBJECT_ID_USER).await
            } => result?,
            exception = exceptions.next() => {
                return match exception {
                    Some(exception) => Err(Box::new(exception) as Box<dyn std::error::Error>),
                    None => Err(Box::new(msfs_async::Error::DriverStopped)),
                };
            }
        };

        println!("Read back: {observed:?}");
        Ok(())
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
