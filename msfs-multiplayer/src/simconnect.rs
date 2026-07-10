use crate::DynError;
use crate::drift::aligned_drift;
use crate::live::{LatestLocalState, LatestRemoteSnapshot, RenderedAircraftUpdate};
use crate::protocol::{AircraftState, UserId};
use crate::timeline::{TargetSample, TargetTimeline};
use msfs_sync::{
    AiAircraft, Error, FreezeState, InitialPosition, RecurringPeriod, SIMCONNECT_OBJECT_ID_USER,
    SimConnect, Subscription, SubscriptionOptions, data_definition,
};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

#[data_definition]
#[derive(Debug)]
struct UserAircraftData {
    #[name = "ABSOLUTE TIME"]
    #[unit = "Seconds"]
    absolute_time: f64,
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

#[data_definition]
#[derive(Debug)]
struct RemoteAircraftPlacement {
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

#[data_definition]
#[derive(Debug)]
struct RemoteAircraftPosition {
    #[name = "ABSOLUTE TIME"]
    #[unit = "Seconds"]
    absolute_time: f64,
    #[name = "PLANE LATITUDE"]
    #[unit = "Degrees"]
    latitude: f64,
    #[name = "PLANE LONGITUDE"]
    #[unit = "Degrees"]
    longitude: f64,
    #[name = "PLANE ALTITUDE"]
    #[unit = "Feet"]
    altitude: f64,
}

#[data_definition]
#[derive(Debug)]
struct RemoteAircraftFreezeState {
    #[name = "IS LATITUDE LONGITUDE FREEZE ON"]
    #[unit = "Bool"]
    latitude_longitude: i64,
    #[name = "IS ALTITUDE FREEZE ON"]
    #[unit = "Bool"]
    altitude: i64,
    #[name = "IS ATTITUDE FREEZE ON"]
    #[unit = "Bool"]
    attitude: i64,
}

impl RemoteAircraftFreezeState {
    fn all_frozen(&self) -> bool {
        self.latitude_longitude != 0 && self.altitude != 0 && self.attitude != 0
    }

    fn all_unfrozen(&self) -> bool {
        self.latitude_longitude == 0 && self.altitude == 0 && self.attitude == 0
    }
}

struct RemoteAircraft {
    handle: AiAircraft,
    drift_updates: Option<Subscription<RemoteAircraftPosition>>,
    target_history: TargetTimeline,
    pending_actual: Option<RemoteAircraftPosition>,
    initialization_watchdog_until: Option<f64>,
}

struct CreationRequest {
    user_id: UserId,
    initial_position: InitialPosition,
}

struct CreationCompletion {
    user_id: UserId,
    result: Result<Option<AiAircraft>, Error>,
}

struct CreationWorker {
    requests: mpsc::SyncSender<CreationRequest>,
    completions: mpsc::Receiver<CreationCompletion>,
    desired_users: Arc<Mutex<HashSet<UserId>>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AlignmentPhase {
    WaitingForFreeze,
    WaitingForUnfreeze,
    Ready,
}

struct AligningAircraft {
    handle: AiAircraft,
    freeze_updates: Subscription<RemoteAircraftFreezeState>,
    phase: AlignmentPhase,
    initialization_watchdog_until: f64,
}

struct PendingAlignment {
    handle: AiAircraft,
    release_ai_control: bool,
    initialization_watchdog_until: Option<f64>,
}

struct RemoteFleet {
    aircraft: HashMap<UserId, RemoteAircraft>,
    awaiting_alignment: HashMap<UserId, PendingAlignment>,
    aligning: HashMap<UserId, AligningAircraft>,
    pending_creations: HashSet<UserId>,
    desired_users: HashSet<UserId>,
    creation_worker: CreationWorker,
}

const INITIALIZATION_WATCHDOG_SECONDS: f64 = 30.0;
const INITIALIZATION_STABILITY_SECONDS: f64 = 10.0;
const INITIALIZATION_REALIGN_THRESHOLD_FEET: f64 = 100.0;

impl CreationWorker {
    fn spawn(sim: SimConnect, model_title: String) -> Result<Self, io::Error> {
        let (request_tx, request_rx) = mpsc::sync_channel::<CreationRequest>(1);
        let (completion_tx, completion_rx) = mpsc::channel::<CreationCompletion>();
        let desired_users = Arc::new(Mutex::new(HashSet::<UserId>::new()));
        let worker_desired_users = Arc::clone(&desired_users);
        std::thread::Builder::new()
            .name("msfs-multiplayer-create".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let desired = worker_desired_users
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .contains(&request.user_id);
                    let result = if desired {
                        sim.create_non_atc_aircraft(
                            &model_title,
                            format!("FSMP{}", request.user_id),
                            request.initial_position,
                        )
                        .map(Some)
                    } else {
                        Ok(None)
                    };
                    let completion = CreationCompletion {
                        user_id: request.user_id,
                        result,
                    };
                    if let Err(error) = completion_tx.send(completion) {
                        if let Ok(Some(aircraft)) = error.0.result {
                            let _ = sim.remove_object(aircraft.object_id());
                        }
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: request_tx,
            completions: completion_rx,
            desired_users,
        })
    }

    fn set_desired_users(&self, users: &HashSet<UserId>) {
        *self
            .desired_users
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = users.clone();
    }

    fn try_queue(&self, request: CreationRequest) -> Result<bool, io::Error> {
        match self.requests.try_send(request) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "remote aircraft creation worker stopped",
            )),
        }
    }
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
    let mut fleet = RemoteFleet {
        aircraft: HashMap::new(),
        awaiting_alignment: HashMap::new(),
        aligning: HashMap::new(),
        pending_creations: HashSet::new(),
        desired_users: HashSet::new(),
        creation_worker: CreationWorker::spawn(sim.clone(), model_title.to_owned())?,
    };
    let mut receiver_absolute_time = None;

    loop {
        while let Some(update) = user_updates.try_recv()? {
            if update.absolute_time.is_finite() {
                receiver_absolute_time = Some(update.absolute_time);
            }
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
            reconcile(&sim, &mut fleet, &snapshot, receiver_absolute_time)?;
        }
        collect_creation_completions(&sim, &mut fleet)?;

        std::thread::sleep(Duration::from_millis(5));
    }
}

fn reconcile(
    sim: &SimConnect,
    fleet: &mut RemoteFleet,
    snapshot: &[RenderedAircraftUpdate],
    receiver_absolute_time: Option<f64>,
) -> Result<(), DynError> {
    let present = snapshot
        .iter()
        .map(|update| update.user_id)
        .collect::<HashSet<_>>();
    fleet.desired_users = present.clone();
    fleet.creation_worker.set_desired_users(&present);

    for update in snapshot {
        if !update.state.is_finite() {
            continue;
        }
        let mut alignment_ready = false;
        if let Some(aligning) = fleet.aligning.get_mut(&update.user_id) {
            if let Some(freeze) = aligning.freeze_updates.try_recv()? {
                match aligning.phase {
                    AlignmentPhase::WaitingForFreeze if freeze.all_frozen() => {
                        let object_id = aligning.handle.object_id();
                        sim.set_data_on_sim_object(
                            object_id,
                            &RemoteAircraftPlacement::from(update.state),
                        )?;
                        sim.set_freeze(object_id, FreezeState::NONE)?;
                        aligning.phase = AlignmentPhase::WaitingForUnfreeze;
                        eprintln!(
                            "Positioned remote user {} as object {} at playback {:.3}s; awaiting unfreeze.",
                            update.user_id, object_id, update.playback_timestamp_seconds,
                        );
                    }
                    AlignmentPhase::WaitingForUnfreeze if freeze.all_unfrozen() => {
                        aligning.phase = AlignmentPhase::Ready;
                    }
                    _ => {}
                }
            }
            alignment_ready =
                aligning.phase == AlignmentPhase::Ready && receiver_absolute_time.is_some();
        }
        if alignment_ready {
            let aligning = fleet
                .aligning
                .remove(&update.user_id)
                .expect("the alignment was just confirmed ready");
            let object_id = aligning.handle.object_id();
            let drift_updates = sim.subscribe_with_options::<RemoteAircraftPosition>(
                object_id,
                SubscriptionOptions::new(RecurringPeriod::Second).latest(),
            )?;
            eprintln!(
                "Aligned remote user {} as object {} at playback {:.3}s.",
                update.user_id, object_id, update.playback_timestamp_seconds,
            );
            fleet.aircraft.insert(
                update.user_id,
                RemoteAircraft {
                    handle: aligning.handle,
                    drift_updates: Some(drift_updates),
                    target_history: TargetTimeline::default(),
                    pending_actual: None,
                    initialization_watchdog_until: Some(aligning.initialization_watchdog_until),
                },
            );
        }

        if let Some(receiver_absolute_time) = receiver_absolute_time
            && let Some(pending) = fleet.awaiting_alignment.remove(&update.user_id)
        {
            let object_id = pending.handle.object_id();
            if pending.release_ai_control {
                sim.release_ai_control(object_id)?;
            }
            let freeze_updates = sim.subscribe_with_options::<RemoteAircraftFreezeState>(
                object_id,
                SubscriptionOptions::new(RecurringPeriod::SimFrame).latest(),
            )?;
            sim.set_freeze(object_id, FreezeState::ALL)?;
            fleet.aligning.insert(
                update.user_id,
                AligningAircraft {
                    handle: pending.handle,
                    freeze_updates,
                    phase: AlignmentPhase::WaitingForFreeze,
                    initialization_watchdog_until: pending
                        .initialization_watchdog_until
                        .unwrap_or(receiver_absolute_time + INITIALIZATION_WATCHDOG_SECONDS),
                },
            );
            eprintln!(
                "Freezing remote user {} as object {} before alignment.",
                update.user_id, object_id,
            );
            continue;
        }

        if fleet.aligning.contains_key(&update.user_id) {
            continue;
        }
        if fleet.awaiting_alignment.contains_key(&update.user_id) {
            continue;
        }

        if !fleet.aircraft.contains_key(&update.user_id) {
            if !fleet.pending_creations.contains(&update.user_id)
                && fleet.creation_worker.try_queue(CreationRequest {
                    user_id: update.user_id,
                    initial_position: initial_position(update.state),
                })?
            {
                fleet.pending_creations.insert(update.user_id);
            }
            continue;
        }

        let remote = fleet
            .aircraft
            .get_mut(&update.user_id)
            .expect("the remote aircraft was just aligned or already active");
        if let Some(receiver_absolute_time) = receiver_absolute_time {
            let reset = remote.target_history.record(TargetSample {
                receiver_absolute_time,
                playback_timestamp_seconds: update.playback_timestamp_seconds,
                expected: update.state,
                buffer_depth: update.buffer_depth,
                underrun: update.underrun,
            });
            if reset {
                remote.pending_actual = None;
            }
        }
        let object_id = remote.handle.object_id();
        sim.set_data_on_sim_object(object_id, &RemoteAircraftData::from(update.state))?;
    }

    let removed = fleet
        .aircraft
        .keys()
        .copied()
        .filter(|user_id| !present.contains(user_id))
        .collect::<Vec<_>>();
    for user_id in removed {
        let object = fleet
            .aircraft
            .remove(&user_id)
            .expect("the object ID came from this map");
        sim.remove_object(object.handle.object_id())?;
        eprintln!("Removed remote user {user_id}.");
    }
    let unaligned_removed = fleet
        .awaiting_alignment
        .keys()
        .copied()
        .filter(|user_id| !present.contains(user_id))
        .collect::<Vec<_>>();
    for user_id in unaligned_removed {
        let object = fleet
            .awaiting_alignment
            .remove(&user_id)
            .expect("the object ID came from this map");
        sim.remove_object(object.handle.object_id())?;
        eprintln!("Removed unaligned remote user {user_id}.");
    }
    let positioning_removed = fleet
        .aligning
        .keys()
        .copied()
        .filter(|user_id| !present.contains(user_id))
        .collect::<Vec<_>>();
    for user_id in positioning_removed {
        let object = fleet
            .aligning
            .remove(&user_id)
            .expect("the object ID came from this map");
        sim.remove_object(object.handle.object_id())?;
        eprintln!("Removed positioning remote user {user_id}.");
    }
    let reset_users = log_ready_drifts(&mut fleet.aircraft);
    for user_id in reset_users {
        let remote = fleet
            .aircraft
            .remove(&user_id)
            .expect("the watchdog user came from the active aircraft map");
        let object_id = remote.handle.object_id();
        fleet.awaiting_alignment.insert(
            user_id,
            PendingAlignment {
                handle: remote.handle,
                release_ai_control: false,
                initialization_watchdog_until: remote.initialization_watchdog_until,
            },
        );
        eprintln!(
            "Remote user {user_id} object {object_id} reset during initialization; realigning."
        );
    }
    Ok(())
}

fn collect_creation_completions(sim: &SimConnect, fleet: &mut RemoteFleet) -> Result<(), DynError> {
    loop {
        let completion = match fleet.creation_worker.completions.try_recv() {
            Ok(completion) => completion,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "remote aircraft creation worker stopped",
                )
                .into());
            }
        };
        fleet.pending_creations.remove(&completion.user_id);
        let Some(created) = completion.result? else {
            continue;
        };
        if fleet.desired_users.contains(&completion.user_id) {
            eprintln!(
                "Created remote user {} as object {}; awaiting fresh alignment.",
                completion.user_id,
                created.object_id(),
            );
            if let Some(replaced) = fleet.awaiting_alignment.insert(
                completion.user_id,
                PendingAlignment {
                    handle: created,
                    release_ai_control: true,
                    initialization_watchdog_until: None,
                },
            ) {
                sim.remove_object(replaced.handle.object_id())?;
            }
        } else {
            sim.remove_object(created.object_id())?;
            eprintln!(
                "Removed remote user {} created after it disconnected.",
                completion.user_id,
            );
        }
    }
}

fn log_ready_drifts(aircraft: &mut HashMap<UserId, RemoteAircraft>) -> Vec<UserId> {
    let mut output = String::new();
    let mut reset_users = Vec::new();
    for (&user_id, remote) in aircraft {
        let Some(drift_updates) = remote.drift_updates.as_mut() else {
            continue;
        };
        match drift_updates.try_recv() {
            Ok(Some(actual)) => {
                if actual.absolute_time.is_finite() {
                    remote.pending_actual = Some(actual);
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = writeln!(
                    output,
                    "Stopped measuring remote user {user_id} drift: {error}",
                );
                remote.drift_updates = None;
            }
        }

        let Some(actual) = remote.pending_actual.as_ref() else {
            continue;
        };
        if remote
            .target_history
            .oldest_time()
            .is_some_and(|oldest| actual.absolute_time < oldest)
        {
            remote.pending_actual = None;
            continue;
        }
        let Some(expected) = remote.target_history.sample(actual.absolute_time) else {
            continue;
        };
        let drift = aligned_drift(
            expected.expected.latitude,
            expected.expected.longitude,
            expected.expected.altitude,
            expected.expected.heading,
            actual.latitude,
            actual.longitude,
            actual.altitude,
        );
        let measurement_age_ms = remote
            .target_history
            .latest_time()
            .map(|latest| (latest - actual.absolute_time).max(0.0) * 1_000.0)
            .unwrap_or(0.0);
        let _ = writeln!(
            output,
            "user={user_id} age={measurement_age_ms:.0}ms along={:+.1}ft cross={:+.1}ft vertical={:+.1}ft total={:.1}ft playback={:.3}s depth={} underrun={}",
            drift.along_track_feet,
            drift.cross_track_feet,
            drift.vertical_feet,
            drift.total_feet,
            expected.playback_timestamp_seconds,
            expected.buffer_depth,
            expected.underrun,
        );
        if let Some(watchdog_until) = remote.initialization_watchdog_until {
            if actual.absolute_time > watchdog_until {
                remote.initialization_watchdog_until = None;
                let _ = writeln!(
                    output,
                    "user={user_id} initialization stable; position watchdog disabled",
                );
            } else if drift.total_feet > INITIALIZATION_REALIGN_THRESHOLD_FEET {
                remote.initialization_watchdog_until = Some(
                    watchdog_until.max(actual.absolute_time + INITIALIZATION_STABILITY_SECONDS),
                );
                reset_users.push(user_id);
            }
        }
        remote.pending_actual = None;
    }
    if !output.is_empty() {
        eprint!("{output}");
    }
    reset_users
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
            heading: state.heading,
            pitch: state.pitch,
            bank: state.bank,
            velocity_body_x: state.velocity_body_x,
            velocity_body_y: state.velocity_body_y,
            velocity_body_z: state.velocity_body_z,
        }
    }
}

impl From<AircraftState> for RemoteAircraftPlacement {
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
