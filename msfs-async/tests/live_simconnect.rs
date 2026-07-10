#![cfg(windows)]

use futures_util::StreamExt;
use msfs_async::{
    AiAircraft, AsyncSimConnect, Error, InitialPosition, RecurringPeriod,
    SIMCONNECT_OBJECT_ID_USER, SubscriptionOptions, client_data_definition, data_definition,
};
use std::time::Duration;
use tokio::time::{sleep, timeout};

const LIVE_TIMEOUT: Duration = Duration::from_secs(10);

#[data_definition]
#[derive(Debug, PartialEq)]
struct LiveAircraftData {
    #[name = "PLANE ALTITUDE"]
    #[unit = "Feet"]
    altitude: f64,
    #[name = "SIM ON GROUND"]
    #[unit = "Bool"]
    on_ground: i64,
}

#[data_definition]
#[derive(Debug, PartialEq)]
struct LiveLVar {
    #[name = "L:MSFS_ASYNC_LIVE_TEST_VALUE"]
    #[unit = "number"]
    value: f64,
}

#[data_definition]
#[derive(Debug)]
struct LivePosition {
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
}

#[client_data_definition]
#[derive(Debug, PartialEq)]
struct LiveClientData {
    sequence: u32,
    value: f64,
}

async fn open(test_name: &str) -> AsyncSimConnect {
    timeout(
        LIVE_TIMEOUT,
        AsyncSimConnect::open(format!("MSFS ASYNC LIVE TEST {test_name}")),
    )
    .await
    .expect("SimConnect_Open timed out; start MSFS and fully load a flight")
    .expect("SimConnect_Open failed; start MSFS and fully load a flight")
}

#[tokio::test]
#[ignore = "requires Windows with a running simulator and a fully loaded flight"]
async fn requests_mixed_live_aircraft_data() {
    let sim = open("REQUEST ONCE").await;
    let value = timeout(
        LIVE_TIMEOUT,
        sim.request_once::<LiveAircraftData>(SIMCONNECT_OBJECT_ID_USER),
    )
    .await
    .expect("live data request timed out")
    .expect("live data request failed");

    assert!(value.altitude.is_finite());
    assert!(matches!(value.on_ground, 0 | 1));
    sim.close().await.expect("driver shutdown failed");
}

#[tokio::test]
#[ignore = "requires Windows with a running simulator and a fully loaded flight"]
async fn writes_and_reads_back_an_l_variable() {
    let sim = open("LVAR ROUND TRIP").await;
    let expected = LiveLVar { value: 42.25 };
    sim.set_data_on_sim_object(SIMCONNECT_OBJECT_ID_USER, &expected)
        .await
        .expect("L-variable write failed");
    let actual = timeout(
        LIVE_TIMEOUT,
        sim.request_once::<LiveLVar>(SIMCONNECT_OBJECT_ID_USER),
    )
    .await
    .expect("L-variable read timed out")
    .expect("L-variable read failed");

    assert_eq!(actual, expected);
    sim.close().await.expect("driver shutdown failed");
}

#[tokio::test]
#[ignore = "requires Windows with a running simulator and a fully loaded flight"]
async fn limited_full_buffer_subscription_terminates() {
    let sim = open("LIMITED SUBSCRIPTION").await;
    let mut options = SubscriptionOptions::new(RecurringPeriod::VisualFrame);
    options.capacity = 0;
    options.changed_only = false;
    options.limit = 2;
    let mut updates = sim
        .subscribe_with_options::<LiveAircraftData>(SIMCONNECT_OBJECT_ID_USER, options)
        .await
        .expect("subscription registration failed");

    // Let both native deliveries occur before polling. A zero-capacity futures
    // channel reserves one sender slot, so the second value is dropped locally.
    sleep(Duration::from_millis(250)).await;
    let first = timeout(LIVE_TIMEOUT, updates.next())
        .await
        .expect("first subscription value timed out")
        .expect("subscription ended before its first value")
        .expect("subscription route failed");
    assert!(first.altitude.is_finite());
    assert!(
        timeout(LIVE_TIMEOUT, updates.next())
            .await
            .expect("limited subscription did not terminate")
            .is_none()
    );
    sim.close().await.expect("driver shutdown failed");
}

#[tokio::test]
#[ignore = "requires Windows with a running simulator and a fully loaded flight"]
async fn exchanges_padded_client_data_between_sessions() {
    let writer = open("CLIENT DATA WRITER").await;
    let reader = open("CLIENT DATA READER").await;
    let area_name = format!("MSFS_ASYNC_LIVE_TEST_{}", std::process::id());
    let area = writer
        .create_client_data::<LiveClientData>(area_name.clone())
        .await
        .expect("client-data creation failed");
    let mut updates = reader
        .subscribe_client_data::<LiveClientData>(area_name)
        .await
        .expect("client-data subscription failed");
    let expected = LiveClientData {
        sequence: 7,
        value: 123.5,
    };
    area.set(&expected).await.expect("client-data write failed");

    let actual = timeout(LIVE_TIMEOUT, async {
        loop {
            let value = updates
                .next()
                .await
                .expect("client-data subscription ended")
                .expect("client-data route failed");
            if value == expected {
                break value;
            }
        }
    })
    .await
    .expect("client-data delivery timed out");
    assert_eq!(actual, expected);
    drop(updates);
    writer.close().await.expect("writer shutdown failed");
    reader.close().await.expect("reader shutdown failed");
}

#[tokio::test]
#[ignore = "requires Windows with a running simulator and a fully loaded flight"]
async fn closing_one_handle_stops_all_clones() {
    let sim = open("CLOSE CLONES").await;
    let clone = sim.clone();
    sim.close().await.expect("driver shutdown failed");

    assert_eq!(
        clone
            .request_once::<LiveAircraftData>(SIMCONNECT_OBJECT_ID_USER)
            .await,
        Err(Error::DriverStopped)
    );
}

#[tokio::test]
#[ignore = "requires a running simulator and an installed test aircraft"]
async fn creates_and_removes_an_ai_aircraft_when_configured() {
    let model_title =
        std::env::var("MSFS_TEST_AIRCRAFT_TITLE").unwrap_or_else(|_| "A320neo V2".to_owned());
    let sim = open("AI AIRCRAFT").await;
    let user = timeout(
        LIVE_TIMEOUT,
        sim.request_once::<LivePosition>(SIMCONNECT_OBJECT_ID_USER),
    )
    .await
    .expect("user-position request timed out")
    .expect("user-position request failed");
    let aircraft: AiAircraft = timeout(
        LIVE_TIMEOUT,
        sim.create_non_atc_aircraft(
            model_title,
            "MSFSTEST",
            InitialPosition {
                latitude: user.latitude,
                longitude: user.longitude + 0.002,
                altitude: user.altitude,
                pitch: 0.0,
                bank: 0.0,
                heading: user.heading,
                on_ground: false,
                airspeed: 0,
            },
        ),
    )
    .await
    .expect("AI-aircraft creation timed out")
    .expect("AI-aircraft creation failed");

    sim.remove_object(aircraft.object_id())
        .await
        .expect("AI-aircraft removal failed");
    sim.close().await.expect("driver shutdown failed");
}
