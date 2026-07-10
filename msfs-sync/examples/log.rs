#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use msfs_sync::{Error, Period, SIMCONNECT_OBJECT_ID_USER, SimConnect, Subscription};

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = SimConnect::open("SYNC LOG")?;
    let data = sim.subscribe::<Data>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)?;
    let controls = sim.subscribe::<Controls>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)?;
    let throttle = sim.subscribe::<Throttle>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame)?;

    std::thread::scope(|scope| -> Result<(), Error> {
        let data = scope.spawn(|| consume("Data", data));
        let controls = scope.spawn(|| consume("Controls", controls));
        let throttle = scope.spawn(|| consume("Throttle", throttle));

        data.join().expect("data consumer panicked")?;
        controls.join().expect("controls consumer panicked")?;
        throttle.join().expect("throttle consumer panicked")?;
        Ok(())
    })?;
    Ok(())
}

#[cfg(windows)]
fn consume<T: std::fmt::Debug>(label: &str, subscription: Subscription<T>) -> Result<(), Error> {
    for value in subscription {
        println!("{label}: {:?}", value?);
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
