#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
mod windows {
    use super::common::{Controls, Data, Throttle};
    use futures_util::TryStreamExt;
    use msfs_async::{AsyncSimConnect, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER};

    #[tokio::main]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let sim = AsyncSimConnect::open("ASYNC LOG").await?;
        let data = sim
            .subscribe::<Data>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
            .await?;
        let controls = sim
            .subscribe::<Controls>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
            .await?;
        let throttle = sim
            .subscribe::<Throttle>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
            .await?;

        futures_util::try_join!(
            data.try_for_each(|value| async move {
                println!("Data: {value:?}");
                Ok(())
            }),
            controls.try_for_each(|value| async move {
                println!("Controls: {value:?}");
                Ok(())
            }),
            throttle.try_for_each(|value| async move {
                println!("Throttle: {value:?}");
                Ok(())
            }),
        )?;
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
