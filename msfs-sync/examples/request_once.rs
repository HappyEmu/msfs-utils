#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use msfs_sync::{SIMCONNECT_OBJECT_ID_USER, SimConnect};

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = SimConnect::open("SYNC ONE SHOT")?;

    let data = sim.request_once::<Data>(SIMCONNECT_OBJECT_ID_USER)?;
    let controls = sim.request_once::<Controls>(SIMCONNECT_OBJECT_ID_USER)?;
    let throttle = sim.request_once::<Throttle>(SIMCONNECT_OBJECT_ID_USER)?;

    println!("Data: {data:?}");
    println!("Controls: {controls:?}");
    println!("Throttle: {throttle:?}");
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
