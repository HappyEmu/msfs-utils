#[cfg(windows)]
#[path = "common/mod.rs"]
mod common;

#[cfg(windows)]
use common::{Controls, Data, Throttle};
#[cfg(windows)]
use msfs_sync::{Error, EventReceiver, Period, SIMCONNECT_OBJECT_ID_USER, SimConnect};

#[cfg(windows)]
#[derive(Debug)]
enum FlightEvent {
    Data(Data),
    Throttle(Throttle),
}

#[cfg(windows)]
#[derive(Debug)]
enum ControlEvent {
    Controls(Controls),
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sim = SimConnect::open("SYNC MULTIPLE RECEIVERS")?;

    // Each receiver has its own channel, capacity, and subscription guards.
    let mut flight_events = sim.event_receiver_with_capacity::<FlightEvent>(64);
    flight_events.subscribe::<Data, _>(
        SIMCONNECT_OBJECT_ID_USER,
        Period::SimFrame,
        FlightEvent::Data,
    )?;
    flight_events.subscribe::<Throttle, _>(
        SIMCONNECT_OBJECT_ID_USER,
        Period::SimFrame,
        FlightEvent::Throttle,
    )?;

    let mut control_events = sim.event_receiver::<ControlEvent>();
    control_events.subscribe::<Controls, _>(
        SIMCONNECT_OBJECT_ID_USER,
        Period::SimFrame,
        ControlEvent::Controls,
    )?;

    // Correct: a blocking consumer thread for each independent receiver.
    std::thread::scope(|scope| -> Result<(), Error> {
        let flight = scope.spawn(move || consume_flight_events(flight_events));
        let controls = scope.spawn(move || consume_control_events(control_events));

        flight.join().expect("flight consumer panicked")?;
        controls.join().expect("control consumer panicked")?;
        Ok(())
    })?;
    Ok(())
}

#[cfg(windows)]
fn consume_flight_events(events: EventReceiver<FlightEvent>) -> Result<(), Error> {
    for event in events {
        match event? {
            FlightEvent::Data(value) => println!("Flight data: {value:?}"),
            FlightEvent::Throttle(value) => println!("Throttle: {value:?}"),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn consume_control_events(events: EventReceiver<ControlEvent>) -> Result<(), Error> {
    for event in events {
        match event? {
            ControlEvent::Controls(value) => println!("Controls: {value:?}"),
        }
    }
    Ok(())
}

/// Do not consume independent blocking receivers sequentially on one thread.
/// If `flight_events.recv()` has no value, the controls receiver is never
/// checked even when control data is waiting in its channel.
#[cfg(windows)]
#[allow(dead_code)]
fn sequential_blocking_is_not_multiplexing(
    mut flight_events: EventReceiver<FlightEvent>,
    mut control_events: EventReceiver<ControlEvent>,
) -> Result<(), Error> {
    loop {
        println!("Flight: {:?}", flight_events.recv()?);
        println!("Controls: {:?}", control_events.recv()?);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
