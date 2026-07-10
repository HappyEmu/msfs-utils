#[cfg(windows)]
mod windows {
    use futures_util::TryStreamExt;
    use msfs_async::{
        AsyncSimConnect, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER, data_definition,
    };
    use std::time::Duration;

    #[data_definition]
    #[derive(Debug)]
    struct SharedValue {
        #[name = "L:MSFS_ASYNC_READ_WRITE_VALUE"]
        #[unit = "number"]
        value: f64,
    }

    #[tokio::main]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let sim = AsyncSimConnect::open("ASYNC READ WRITE").await?;

        // Register the reader first so it observes every subsequent change.
        let updates = sim
            .subscribe::<SharedValue>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
            .await?;

        // This clone sends commands to the same driver and SimConnect handle.
        let writer = sim.clone();
        futures_util::try_join!(
            write_values(writer),
            updates.try_for_each(|value| async move {
                println!("Read: {value:?}");
                Ok(())
            }),
        )?;
        Ok(())
    }

    async fn write_values(sim: AsyncSimConnect) -> Result<(), msfs_async::Error> {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        let mut value = 0.0;

        loop {
            ticker.tick().await;
            value += 1.0;
            sim.set_data_on_sim_object(SIMCONNECT_OBJECT_ID_USER, &SharedValue { value })
                .await?;
            println!("Wrote: {value}");
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
