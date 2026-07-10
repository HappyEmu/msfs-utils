#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use futures_util::TryStreamExt;
#[cfg(windows)]
use msfs_async::{AsyncSimConnect, Error, Period, SIMCONNECT_OBJECT_ID_USER, Subscription};

/// Give each data type an independent async consumer task. This is useful when
/// consumers have unrelated processing or output responsibilities.
#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = AsyncSimConnect::open("ASYNC TASKS").await?;
    let data = sim
        .subscribe::<Data>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)
        .await?;
    let controls = sim
        .subscribe::<Controls>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)
        .await?;
    let throttle = sim
        .subscribe::<Throttle>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)
        .await?;

    let (data, controls, throttle) = tokio::try_join!(
        tokio::spawn(print_stream("Data", data)),
        tokio::spawn(print_stream("Controls", controls)),
        tokio::spawn(print_stream("Throttle", throttle)),
    )?;
    data?;
    controls?;
    throttle?;
    Ok(())
}

#[cfg(windows)]
async fn print_stream<T>(label: &'static str, stream: Subscription<T>) -> Result<(), Error>
where
    T: std::fmt::Debug,
{
    stream
        .try_for_each(move |value| async move {
            println!("{label}: {value:?}");
            Ok(())
        })
        .await
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
