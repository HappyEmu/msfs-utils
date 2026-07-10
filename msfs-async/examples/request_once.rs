#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use msfs_async::{AsyncSimConnect, SIMCONNECT_OBJECT_ID_USER};

/// Await several one-shot requests concurrently. Each future resolves directly
/// to its declared data type; request IDs never enter application code.
#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = AsyncSimConnect::open("ASYNC ONE SHOT").await?;

    let (data, controls, throttle) = futures_util::try_join!(
        sim.request_once::<Data>(SIMCONNECT_OBJECT_ID_USER),
        sim.request_once::<Controls>(SIMCONNECT_OBJECT_ID_USER),
        sim.request_once::<Throttle>(SIMCONNECT_OBJECT_ID_USER),
    )?;

    println!("Data: {data:?}");
    println!("Controls: {controls:?}");
    println!("Throttle: {throttle:?}");
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
