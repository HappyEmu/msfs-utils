#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use futures_util::StreamExt;
#[cfg(windows)]
use msfs_async::{AsyncSimConnect, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER};

/// Multiplex several typed subscriptions in one control-flow loop. The match
/// is over typed stream values rather than raw SimConnect request IDs.
#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = AsyncSimConnect::open("ASYNC SELECT").await?;
    let mut data = sim
        .subscribe::<Data>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
        .await?;
    let mut controls = sim
        .subscribe::<Controls>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
        .await?;
    let mut throttle = sim
        .subscribe::<Throttle>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
        .await?;

    loop {
        tokio::select! {
            item = data.next() => match item.transpose()? {
                Some(value) => println!("Data: {value:?}"),
                None => break,
            },
            item = controls.next() => match item.transpose()? {
                Some(value) => println!("Controls: {value:?}"),
                None => break,
            },
            item = throttle.next() => match item.transpose()? {
                Some(value) => println!("Throttle: {value:?}"),
                None => break,
            },
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
