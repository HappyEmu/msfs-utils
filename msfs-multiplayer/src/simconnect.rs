use crate::DynError;
use crate::live::{LatestLocalState, LatestRemoteSnapshot};
use crate::protocol::{AircraftState, AircraftUpdate, UserId};
use msfs_sync::{
    AiAircraft, InitialPosition, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER, SimConnect,
    SubscriptionOptions, data_definition,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[data_definition]
#[derive(Debug)]
struct UserAircraftData {
    #[name = "PLANE LATITUDE"]
    #[unit = "Degrees"]
    #[epsilon = 0.000001]
    latitude: f64,
    #[name = "PLANE LONGITUDE"]
    #[unit = "Degrees"]
    #[epsilon = 0.000001]
    longitude: f64,
    #[name = "PLANE ALTITUDE"]
    #[unit = "Feet"]
    #[epsilon = 0.01]
    altitude: f64,
    #[name = "AIRSPEED INDICATED"]
    #[unit = "Knots"]
    #[epsilon = 0.01]
    indicated_airspeed: f64,
    #[name = "PLANE HEADING DEGREES TRUE"]
    #[unit = "Degrees"]
    #[epsilon = 0.01]
    heading: f64,
    #[name = "PLANE PITCH DEGREES"]
    #[unit = "Degrees"]
    #[epsilon = 0.01]
    pitch: f64,
    #[name = "PLANE BANK DEGREES"]
    #[unit = "Degrees"]
    #[epsilon = 0.01]
    bank: f64,
    #[name = "VELOCITY WORLD X"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_world_x: f64,
    #[name = "VELOCITY WORLD Y"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_world_y: f64,
    #[name = "VELOCITY WORLD Z"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_world_z: f64,
    #[name = "VELOCITY BODY X"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_body_x: f64,
    #[name = "VELOCITY BODY Y"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_body_y: f64,
    #[name = "VELOCITY BODY Z"]
    #[unit = "Feet per second"]
    #[epsilon = 0.01]
    velocity_body_z: f64,
    #[name = "SIM ON GROUND"]
    #[unit = "Bool"]
    on_ground: i64,
}

#[data_definition]
#[derive(Debug)]
struct RemoteAircraftData {
    #[name = "PLANE LATITUDE"]
    #[unit = "Degrees"]
    latitude: f64,
    #[name = "PLANE LONGITUDE"]
    #[unit = "Degrees"]
    longitude: f64,
    #[name = "PLANE ALTITUDE"]
    #[unit = "Feet"]
    altitude: f64,
    #[name = "PLANE HEADING DEGREES TRUE"]
    #[unit = "Degrees"]
    heading: f64,
    #[name = "PLANE PITCH DEGREES"]
    #[unit = "Degrees"]
    pitch: f64,
    #[name = "PLANE BANK DEGREES"]
    #[unit = "Degrees"]
    bank: f64,
    #[name = "VELOCITY BODY X"]
    #[unit = "Feet per second"]
    velocity_body_x: f64,
    #[name = "VELOCITY BODY Y"]
    #[unit = "Feet per second"]
    velocity_body_y: f64,
    #[name = "VELOCITY BODY Z"]
    #[unit = "Feet per second"]
    velocity_body_z: f64,
}

pub fn run(
    model_title: &str,
    local_state: LatestLocalState,
    remote_snapshot: LatestRemoteSnapshot,
) -> Result<(), DynError> {
    let sim = SimConnect::open("MSFS MULTIPLAYER")?;
    let mut user_updates = sim.subscribe_with_options::<UserAircraftData>(
        SIMCONNECT_OBJECT_ID_USER,
        SubscriptionOptions::new(RecurringPeriod::SimFrame).latest(),
    )?;
    let mut exceptions = sim.exceptions()?;
    let mut aircraft = HashMap::<UserId, AiAircraft>::new();

    loop {
        while let Some(update) = user_updates.try_recv()? {
            *local_state
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(update.into());
        }

        if let Some(exception) = exceptions.try_recv() {
            return Err(exception.into());
        }

        let snapshot = remote_snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(snapshot) = snapshot {
            reconcile(&sim, model_title, &mut aircraft, &snapshot)?;
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}

fn reconcile(
    sim: &SimConnect,
    model_title: &str,
    aircraft: &mut HashMap<UserId, AiAircraft>,
    snapshot: &[AircraftUpdate],
) -> Result<(), DynError> {
    let present = snapshot
        .iter()
        .map(|update| update.user_id)
        .collect::<HashSet<_>>();

    for update in snapshot {
        if !update.state.is_finite() {
            continue;
        }
        if !aircraft.contains_key(&update.user_id) {
            let created = sim.create_non_atc_aircraft(
                model_title,
                format!("FSMP{}", update.user_id),
                initial_position(update.state),
            )?;
            sim.release_ai_control(created.object_id())?;
            eprintln!(
                "Created remote user {} as object {}.",
                update.user_id,
                created.object_id()
            );
            aircraft.insert(update.user_id, created);
        }

        let object_id = aircraft[&update.user_id].object_id();
        sim.set_data_on_sim_object(object_id, &RemoteAircraftData::from(update.state))?;
    }

    let removed = aircraft
        .keys()
        .copied()
        .filter(|user_id| !present.contains(user_id))
        .collect::<Vec<_>>();
    for user_id in removed {
        let object = aircraft
            .remove(&user_id)
            .expect("the object ID came from this map");
        sim.remove_object(object.object_id())?;
        eprintln!("Removed remote user {user_id}.");
    }
    Ok(())
}

fn initial_position(state: AircraftState) -> InitialPosition {
    InitialPosition {
        latitude: state.latitude,
        longitude: state.longitude,
        altitude: state.altitude,
        pitch: state.pitch,
        bank: state.bank,
        heading: state.heading,
        on_ground: state.on_ground,
        airspeed: airspeed_knots(state.indicated_airspeed),
    }
}

fn airspeed_knots(value: f64) -> u32 {
    if value.is_finite() {
        value.clamp(0.0, u32::MAX as f64).round() as u32
    } else {
        0
    }
}

impl From<UserAircraftData> for AircraftState {
    fn from(data: UserAircraftData) -> Self {
        Self {
            latitude: data.latitude,
            longitude: data.longitude,
            altitude: data.altitude,
            indicated_airspeed: data.indicated_airspeed,
            heading: data.heading,
            pitch: data.pitch,
            bank: data.bank,
            velocity_world_x: data.velocity_world_x,
            velocity_world_y: data.velocity_world_y,
            velocity_world_z: data.velocity_world_z,
            velocity_body_x: data.velocity_body_x,
            velocity_body_y: data.velocity_body_y,
            velocity_body_z: data.velocity_body_z,
            on_ground: data.on_ground != 0,
        }
    }
}

impl From<AircraftState> for RemoteAircraftData {
    fn from(state: AircraftState) -> Self {
        Self {
            latitude: state.latitude,
            longitude: state.longitude,
            altitude: state.altitude,
            heading: state.heading,
            pitch: state.pitch,
            bank: state.bank,
            velocity_body_x: state.velocity_body_x,
            velocity_body_y: state.velocity_body_y,
            velocity_body_z: state.velocity_body_z,
        }
    }
}
