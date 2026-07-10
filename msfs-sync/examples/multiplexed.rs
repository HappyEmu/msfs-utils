#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use msfs_sync::{Period, SIMCONNECT_OBJECT_ID_USER, SimConnect};

#[cfg(windows)]
#[derive(Debug)]
enum Event {
    Data(Data),
    Controls(Controls),
    Throttle(Throttle),
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = SimConnect::open("SYNC MULTIPLEXED")?;
    let mut events = sim.event_receiver::<Event>();

    events.subscribe::<Data, _>(SIMCONNECT_OBJECT_ID_USER, Period::SimFrame, Event::Data)?;
    events.subscribe::<Controls, _>(
        SIMCONNECT_OBJECT_ID_USER,
        Period::SimFrame,
        Event::Controls,
    )?;
    events.subscribe::<Throttle, _>(
        SIMCONNECT_OBJECT_ID_USER,
        Period::SimFrame,
        Event::Throttle,
    )?;

    for event in events {
        match event? {
            Event::Data(value) => println!("Data: {value:?}"),
            Event::Controls(value) => println!("Controls: {value:?}"),
            Event::Throttle(value) => println!("Throttle: {value:?}"),
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
