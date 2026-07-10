#![allow(dead_code)]

use super::*;
use crate::backend::{ClientDataRequest, DataRequest};
use futures_util::{FutureExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
enum Call {
    RequestData(DataRequest),
    RequestClientData(ClientDataRequest),
    AddData(u32),
    ClearData(u32),
    AddClientData(u32),
    ClearClientData(u32),
    CreateAircraft(u32),
    RemoveObject { object_id: u32, request_id: u32 },
    Other,
}

#[derive(Default)]
struct FakeBackend {
    calls: Vec<Call>,
    outcomes: HashMap<&'static str, VecDeque<Result<()>>>,
    packets: VecDeque<Result<Option<Vec<u8>>>>,
    send_ids: VecDeque<Result<u32>>,
    buffer_capacities: Vec<usize>,
    buffer_lengths: Vec<usize>,
}

impl FakeBackend {
    fn succeed(&mut self, operation: &'static str) {
        self.outcomes
            .entry(operation)
            .or_default()
            .push_back(Ok(()));
    }

    fn fail(&mut self, operation: &'static str, error: Error) {
        self.outcomes
            .entry(operation)
            .or_default()
            .push_back(Err(error));
    }

    fn result(&mut self, operation: &'static str) -> Result<()> {
        self.outcomes
            .get_mut(operation)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Ok(()))
    }

    fn packet(&mut self, packet: Vec<u8>) {
        self.packets.push_back(Ok(Some(packet)));
    }
}

impl SimConnectBackend for FakeBackend {
    type OpenContext = Result<Self>;

    const RECV_ID_NULL: u32 = 0;
    const RECV_ID_EXCEPTION: u32 = 1;
    const RECV_ID_QUIT: u32 = 3;
    const RECV_ID_SIMOBJECT_DATA: u32 = 8;
    const RECV_ID_SIMOBJECT_DATA_BYTYPE: u32 = 9;
    const RECV_ID_CLIENT_DATA: u32 = 16;
    const RECV_ID_ASSIGNED_OBJECT_ID: u32 = 17;

    fn open(_: &CStr, context: Self::OpenContext) -> Result<Self> {
        context
    }

    fn set_data_on_sim_object(&mut self, _: u32, _: u32, _: &[u8]) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("set_data")
    }

    fn release_ai_control(&mut self, _: u32, _: u32) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("release")
    }

    fn create_non_atc_aircraft(
        &mut self,
        _: &CStr,
        _: &CStr,
        _: InitialPosition,
        request_id: u32,
    ) -> Result<()> {
        self.calls.push(Call::CreateAircraft(request_id));
        self.result("create_aircraft")
    }

    fn remove_object(&mut self, object_id: u32, request_id: u32) -> Result<()> {
        self.calls.push(Call::RemoveObject {
            object_id,
            request_id,
        });
        self.result("remove_object")
    }

    fn transmit_client_event(&mut self, _: u32, _: u32, _: u32) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("transmit")
    }

    fn map_client_event(&mut self, _: u32, _: &CStr) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("map_event")
    }

    fn request_data(&mut self, request: DataRequest) -> Result<()> {
        self.calls.push(Call::RequestData(request));
        self.result("request_data")
    }

    fn add_data_definition(
        &mut self,
        define_id: u32,
        _: &CStr,
        _: &CStr,
        _: u32,
        _: f32,
    ) -> Result<()> {
        self.calls.push(Call::AddData(define_id));
        self.result("add_data")
    }

    fn clear_data_definition(&mut self, define_id: u32) -> Result<()> {
        self.calls.push(Call::ClearData(define_id));
        self.result("clear_data")
    }

    fn create_client_data(&mut self, _: u32, _: u32) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("create_client_data")
    }

    fn map_client_data_name(&mut self, _: &CStr, _: u32) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("map_client_data")
    }

    fn add_client_data_definition(&mut self, define_id: u32, _: u32, _: u32, _: f32) -> Result<()> {
        self.calls.push(Call::AddClientData(define_id));
        self.result("add_client_definition")
    }

    fn clear_client_data_definition(&mut self, define_id: u32) -> Result<()> {
        self.calls.push(Call::ClearClientData(define_id));
        self.result("clear_client_definition")
    }

    fn set_client_data(&mut self, _: u32, _: u32, _: &[u8]) -> Result<()> {
        self.calls.push(Call::Other);
        self.result("set_client_data")
    }

    fn request_client_data(&mut self, request: ClientDataRequest) -> Result<()> {
        self.calls.push(Call::RequestClientData(request));
        self.result("request_client_data")
    }

    fn last_send_id(&mut self) -> Result<u32> {
        self.send_ids.pop_front().unwrap_or(Ok(100))
    }

    fn next_dispatch(&mut self, packet: &mut Vec<u8>) -> Result<bool> {
        self.buffer_capacities.push(packet.capacity());
        self.buffer_lengths.push(packet.len());
        match self.packets.pop_front().unwrap_or(Ok(None))? {
            Some(next) => {
                packet.extend_from_slice(&next);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

const RECV_ID_NULL: u32 = FakeBackend::RECV_ID_NULL;
const RECV_ID_EXCEPTION: u32 = FakeBackend::RECV_ID_EXCEPTION;
const RECV_ID_QUIT: u32 = FakeBackend::RECV_ID_QUIT;
const RECV_ID_SIMOBJECT_DATA: u32 = FakeBackend::RECV_ID_SIMOBJECT_DATA;
const RECV_ID_ASSIGNED_OBJECT_ID: u32 = FakeBackend::RECV_ID_ASSIGNED_OBJECT_ID;

#[test]
fn fake_backend_scripts_connection_failure() {
    let name = CString::new("test").unwrap();
    let result = FakeBackend::open(name.as_c_str(), Err(Error::HResult(-1)));
    assert!(matches!(result, Err(Error::HResult(-1))));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct TestData(u32);

impl SimData for TestData {
    fn definitions() -> Result<Vec<DataDefinitionEntry>> {
        Ok(vec![definition("TEST")])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
struct TwoData {
    first: u32,
    second: u32,
}

impl SimData for TwoData {
    fn definitions() -> Result<Vec<DataDefinitionEntry>> {
        Ok(vec![definition("FIRST"), definition("SECOND")])
    }
}

impl ClientData for TwoData {
    fn definitions() -> Vec<client_layout::FieldDefinition> {
        vec![(0, 4, 4, 0.0), (4, 4, 4, 0.0)]
    }
}

fn definition(name: &str) -> DataDefinitionEntry {
    DataDefinitionEntry {
        name: CString::new(name).unwrap(),
        units: CString::new("number").unwrap(),
        datatype: 1,
        epsilon: 0.0,
    }
}

fn packet(id: u32, size: usize) -> Vec<u8> {
    let mut packet = vec![0; size];
    write_u32(&mut packet, 0, size as u32);
    write_u32(&mut packet, 8, id);
    packet
}

fn data_packet(request_id: u32, define_id: u32, define_count: u32, value: u32) -> Vec<u8> {
    let mut packet = packet(
        RECV_ID_SIMOBJECT_DATA,
        DATA_PAYLOAD_OFFSET + define_count as usize * 8,
    );
    write_u32(&mut packet, 12, request_id);
    write_u32(&mut packet, 20, define_id);
    write_u32(&mut packet, 36, define_count);
    write_u32(&mut packet, DATA_PAYLOAD_OFFSET, value);
    packet
}

fn exception_packet(code: u32, send_id: u32, index: u32) -> Vec<u8> {
    let mut packet = packet(RECV_ID_EXCEPTION, EXCEPTION_PACKET_SIZE);
    write_u32(&mut packet, 12, code);
    write_u32(&mut packet, 16, send_id);
    write_u32(&mut packet, 20, index);
    packet
}

fn assigned_object_packet(request_id: u32, object_id: u32) -> Vec<u8> {
    let mut packet = packet(RECV_ID_ASSIGNED_OBJECT_ID, ASSIGNED_OBJECT_PACKET_SIZE);
    write_u32(&mut packet, 12, request_id);
    write_u32(&mut packet, 16, object_id);
    packet
}

fn write_u32(packet: &mut [u8], offset: usize, value: u32) {
    packet[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn register_once(
    driver: &mut Driver<FakeBackend>,
    request_id: u32,
) -> oneshot::Receiver<Result<TestData>> {
    let (tx, rx) = oneshot::channel();
    driver.request_once::<TestData>(request_id, 7, tx);
    rx
}

fn initial_position() -> InitialPosition {
    InitialPosition {
        latitude: 47.0,
        longitude: 8.0,
        altitude: 1_500.0,
        pitch: 1.0,
        bank: 2.0,
        heading: 90.0,
        on_ground: false,
        airspeed: 120,
    }
}

fn create_aircraft(
    driver: &mut Driver<FakeBackend>,
    request_id: u32,
) -> oneshot::Receiver<Result<AiAircraft>> {
    let (tx, rx) = oneshot::channel();
    driver.create_non_atc_aircraft(
        request_id,
        &CString::new("Test Aircraft").unwrap(),
        &CString::new("N12345").unwrap(),
        initial_position(),
        tx,
    );
    rx
}

#[test]
fn creates_aircraft_and_delivers_assigned_object_id() {
    let mut driver = Driver::new(FakeBackend::default());
    let aircraft = create_aircraft(&mut driver, 40);
    driver.backend.packet(assigned_object_packet(40, 9001));

    driver.drain_dispatch().unwrap();
    assert_eq!(
        aircraft.now_or_never().unwrap().unwrap(),
        Ok(AiAircraft::new(9001))
    );
    assert!(
        driver
            .backend()
            .calls
            .iter()
            .any(|call| matches!(call, Call::CreateAircraft(request_id) if *request_id == 40))
    );

    let collision = create_aircraft(&mut driver, 40);
    assert_eq!(
        collision.now_or_never().unwrap().unwrap(),
        Err(Error::IdExhausted)
    );
    driver.complete_object_creation(40);
    let replacement = create_aircraft(&mut driver, 40);
    assert!(replacement.now_or_never().is_none());
}

#[test]
fn aircraft_creation_reports_immediate_and_correlated_server_errors() {
    let mut backend = FakeBackend::default();
    backend.fail("create_aircraft", Error::HResult(-30));
    let mut driver = Driver::new(backend);
    let immediate = create_aircraft(&mut driver, 41);
    assert_eq!(
        immediate.now_or_never().unwrap().unwrap(),
        Err(Error::HResult(-30))
    );

    let mut backend = FakeBackend::default();
    backend.send_ids.push_back(Ok(88));
    let mut driver = Driver::new(backend);
    let delayed = create_aircraft(&mut driver, 42);
    driver.backend.packet(exception_packet(22, 88, 0));
    driver.drain_dispatch().unwrap();
    assert_eq!(
        delayed.now_or_never().unwrap().unwrap(),
        Err(Error::SimConnectException(ServerException {
            code: 22,
            send_id: 88,
            index: 0,
        }))
    );
}

#[test]
fn cancellation_before_assignment_removes_the_late_aircraft() {
    let mut backend = FakeBackend::default();
    backend.fail("remove_object", Error::HResult(-31));
    let mut driver = Driver::new(backend);
    let aircraft = create_aircraft(&mut driver, 43);
    drop(aircraft);
    driver.abandon_object_creation(43);
    driver.backend.packet(assigned_object_packet(43, 9002));

    assert!(driver.drain_dispatch().unwrap());
    assert!(driver.backend().calls.iter().any(|call| matches!(
        call,
        Call::RemoveObject {
            object_id: 9002,
            request_id: 43,
        }
    )));
}

#[test]
fn cancellation_after_assignment_was_queued_removes_the_aircraft() {
    let mut driver = Driver::new(FakeBackend::default());
    let aircraft = create_aircraft(&mut driver, 44);
    driver.backend.packet(assigned_object_packet(44, 9003));
    driver.drain_dispatch().unwrap();

    drop(aircraft);
    driver.abandon_object_creation(44);
    assert!(driver.backend().calls.iter().any(|call| matches!(
        call,
        Call::RemoveObject {
            object_id: 9003,
            request_id: 44,
        }
    )));
}

#[test]
fn registers_delivers_and_removes_a_one_shot_request() {
    let mut driver = Driver::new(FakeBackend::default());
    let rx = register_once(&mut driver, 4);
    driver.backend.packet(data_packet(4, 0, 1, 42));

    assert!(driver.drain_dispatch().unwrap());
    assert_eq!(rx.now_or_never().unwrap().unwrap(), Ok(TestData(42)));

    let replacement = register_once(&mut driver, 4);
    assert!(replacement.now_or_never().is_none());
}

#[test]
fn rolls_back_partial_data_definitions_and_reports_rollback_failure() {
    let mut backend = FakeBackend::default();
    backend.succeed("add_data");
    backend.fail("add_data", Error::HResult(-10));
    backend.fail("clear_data", Error::HResult(-11));
    let mut driver = Driver::new(backend);

    let error = driver
        .set_data_on_sim_object(
            1,
            &TwoData {
                first: 1,
                second: 2,
            },
        )
        .unwrap_err();
    assert_eq!(
        error,
        Error::RollbackFailed {
            operation: Box::new(Error::HResult(-10)),
            rollback: Box::new(Error::HResult(-11)),
        }
    );
    assert!(
        driver
            .backend()
            .calls
            .iter()
            .any(|call| matches!(call, Call::ClearData(0)))
    );
}

#[test]
fn rolls_back_partial_client_definitions() {
    let mut backend = FakeBackend::default();
    backend.succeed("add_client_definition");
    backend.fail("add_client_definition", Error::HResult(-20));
    let mut driver = Driver::new(backend);

    assert_eq!(
        driver
            .set_client_data(
                3,
                &TwoData {
                    first: 1,
                    second: 2
                }
            )
            .unwrap_err(),
        Error::HResult(-20)
    );
    assert!(
        driver
            .backend()
            .calls
            .iter()
            .any(|call| matches!(call, Call::ClearClientData(0)))
    );
}

#[test]
fn cancellation_stops_normal_and_client_data_subscriptions() {
    let mut driver = Driver::new(FakeBackend::default());
    let (items_tx, _items_rx) = mpsc::channel(1);
    let (errors_tx, _errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe::<TestData>(
        1,
        2,
        RequestSettings::recurring(DataPeriod::Second),
        DeliverySender::Buffered(items_tx),
        errors_tx,
        ready_tx,
    );
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
    driver.cancel(1);

    let (mapped_tx, _mapped_rx) = mpsc::channel(1);
    let (mapped_errors_tx, _mapped_errors_rx) = mpsc::unbounded();
    let (mapped_ready_tx, mapped_ready_rx) = oneshot::channel();
    driver.subscribe_mapped::<TestData, u32, _>(
        3,
        2,
        DataPeriod::VisualFrame,
        MappedTarget {
            tx: mapped_tx,
            errors: mapped_errors_tx,
            map: |value: TestData| value.0,
        },
        mapped_ready_tx,
    );
    assert_eq!(mapped_ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
    driver.cancel(3);

    let (items_tx, _items_rx) = mpsc::channel(1);
    let (errors_tx, _errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe_client_data::<TwoData>(2, "AREA", items_tx, errors_tx, ready_tx);
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
    driver.cancel(2);

    assert!(driver.backend().calls.iter().any(|call| matches!(
        call,
        Call::RequestData(request) if request.period == DataPeriod::Never
    )));
    assert!(driver.backend().calls.iter().any(|call| matches!(
        call,
        Call::RequestClientData(request) if request.period == ClientDataPeriod::Never
    )));
}

#[test]
fn cancelling_a_pending_one_shot_removes_its_route() {
    let mut driver = Driver::new(FakeBackend::default());
    let pending = register_once(&mut driver, 20);
    driver.cancel(20);
    drop(pending);

    let replacement = register_once(&mut driver, 20);
    assert!(replacement.now_or_never().is_none());
    assert!(!driver.backend().calls.iter().any(|call| matches!(
        call,
        Call::RequestData(request)
            if request.request_id == 20 && request.period == DataPeriod::Never
    )));
}

#[test]
fn quit_and_native_dispatch_failure_terminate_the_driver() {
    let mut backend = FakeBackend::default();
    backend.packet(packet(RECV_ID_QUIT, RECV_HEADER_SIZE));
    let mut driver = Driver::new(backend);
    assert!(!driver.drain_dispatch().unwrap());

    let mut backend = FakeBackend::default();
    backend.packets.push_back(Err(Error::Windows(123)));
    let mut driver = Driver::new(backend);
    assert_eq!(driver.drain_dispatch(), Err(Error::Windows(123)));
}

#[test]
fn null_packet_ends_the_current_drain_without_spinning() {
    let mut backend = FakeBackend::default();
    backend.packet(packet(RECV_ID_NULL, RECV_HEADER_SIZE));
    backend.packet(packet(RECV_ID_QUIT, RECV_HEADER_SIZE));
    let mut driver = Driver::new(backend);

    assert!(driver.drain_dispatch().unwrap());
    assert!(!driver.drain_dispatch().unwrap());
}

#[test]
fn dispatch_storage_reuses_its_allocation() {
    let mut backend = FakeBackend::default();
    backend.packet(packet(RECV_ID_NULL, RECV_HEADER_SIZE));
    backend.packet(packet(RECV_ID_NULL, RECV_HEADER_SIZE));
    let mut driver = Driver::new(backend);

    driver.drain_dispatch().unwrap();
    driver.drain_dispatch().unwrap();

    assert_eq!(driver.backend().buffer_capacities[0], 0);
    assert!(driver.backend().buffer_capacities[1] >= RECV_HEADER_SIZE);
    assert_eq!(driver.backend().buffer_lengths, [0, 0]);
}

#[test]
fn client_data_registration_delivery_and_consumer_removal_succeed() {
    let mut driver = Driver::new(FakeBackend::default());
    let (items_tx, mut items_rx) = mpsc::channel(1);
    let (errors_tx, _errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe_client_data::<TwoData>(30, "AREA", items_tx, errors_tx, ready_tx);
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));

    let mut packet = data_packet(30, 0, 1, 7);
    write_u32(&mut packet, 8, FakeBackend::RECV_ID_CLIENT_DATA);
    driver.backend.packet(packet);
    driver.drain_dispatch().unwrap();
    assert_eq!(
        items_rx.next().now_or_never().unwrap(),
        Some(TwoData {
            first: 7,
            second: 0,
        })
    );

    drop(items_rx);
    let mut packet = data_packet(30, 0, 1, 8);
    write_u32(&mut packet, 8, FakeBackend::RECV_ID_CLIENT_DATA);
    driver.backend.packet(packet);
    driver.drain_dispatch().unwrap();

    let (items_tx, _items_rx) = mpsc::channel(1);
    let (errors_tx, _errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe_client_data::<TwoData>(30, "AREA", items_tx, errors_tx, ready_tx);
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
}

#[test]
fn terminal_error_is_delivered_while_the_data_buffer_is_full() {
    let mut driver = Driver::new(FakeBackend::default());
    let (items_tx, mut items_rx) = mpsc::channel(1);
    let (errors_tx, mut errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe::<TestData>(
        9,
        2,
        RequestSettings::recurring(DataPeriod::SimFrame),
        DeliverySender::Buffered(items_tx),
        errors_tx,
        ready_tx,
    );
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
    driver.backend.packet(data_packet(9, 0, 1, 1));
    driver.backend.packet(data_packet(9, 0, 1, 2));
    driver.drain_dispatch().unwrap();
    driver.fail_all(Error::DriverStopped);

    assert_eq!(items_rx.next().now_or_never().unwrap(), Some(TestData(1)));
    assert_eq!(
        errors_rx.next().now_or_never().unwrap(),
        Some(Error::DriverStopped)
    );
}

#[test]
fn explicit_termination_fails_pending_routes() {
    let mut driver = Driver::new(FakeBackend::default());
    let pending = register_once(&mut driver, 21);
    driver.fail_all(Error::DriverStopped);

    assert_eq!(
        pending.now_or_never().unwrap().unwrap(),
        Err(Error::DriverStopped)
    );
}

#[test]
fn internal_identifier_spaces_report_exhaustion() {
    let mut driver = Driver::new(FakeBackend::default());
    driver.next_define_id = u32::MAX;
    assert_eq!(
        driver.set_data_on_sim_object(1, &TestData(1)),
        Err(Error::IdExhausted)
    );

    let mut driver = Driver::new(FakeBackend::default());
    driver.next_client_define_id = u32::MAX;
    assert_eq!(
        driver.set_client_data(
            1,
            &TwoData {
                first: 1,
                second: 2,
            },
        ),
        Err(Error::IdExhausted)
    );

    let mut driver = Driver::new(FakeBackend::default());
    driver.next_client_data_id = u32::MAX;
    assert_eq!(driver.client_data_id("AREA"), Err(Error::IdExhausted));
}

#[test]
fn rejects_request_id_collisions_before_calling_the_backend() {
    let mut driver = Driver::new(FakeBackend::default());
    let first = register_once(&mut driver, 5);
    let second = register_once(&mut driver, 5);

    assert_eq!(
        second.now_or_never().unwrap().unwrap(),
        Err(Error::IdExhausted)
    );
    assert!(first.now_or_never().is_none());
    let request_calls = driver
        .backend()
        .calls
        .iter()
        .filter(|call| matches!(call, Call::RequestData(_)))
        .count();
    assert_eq!(request_calls, 1);
}

#[test]
fn correlates_server_exceptions_with_the_last_send_id() {
    let mut backend = FakeBackend::default();
    backend.send_ids.push_back(Ok(77));
    let mut driver = Driver::new(backend);
    let rx = register_once(&mut driver, 6);
    let (exceptions_tx, mut exceptions_rx) = mpsc::unbounded();
    driver.exception_sinks.push(exceptions_tx);
    driver.backend.packet(exception_packet(3, 77, 2));

    driver.drain_dispatch().unwrap();
    let expected = ServerException {
        code: 3,
        send_id: 77,
        index: 2,
    };
    assert_eq!(
        exceptions_rx.next().now_or_never().unwrap(),
        Some(expected.clone())
    );
    assert_eq!(
        rx.now_or_never().unwrap().unwrap(),
        Err(Error::SimConnectException(expected))
    );
}

#[test]
fn rejects_truncated_packets_and_inconsistent_definition_counts() {
    let mut backend = FakeBackend::default();
    backend.packet(vec![0; 8]);
    let mut driver = Driver::new(backend);
    assert_eq!(
        driver.drain_dispatch(),
        Err(Error::InvalidPacket {
            expected: RECV_HEADER_SIZE,
            actual: 8,
        })
    );

    let mut driver = Driver::new(FakeBackend::default());
    let rx = register_once(&mut driver, 8);
    let mut malformed = data_packet(8, 0, 1, 1);
    write_u32(&mut malformed, 36, 2);
    driver.backend.packet(malformed);
    assert_eq!(
        driver.drain_dispatch(),
        Err(Error::InvalidPacket {
            expected: DATA_PAYLOAD_OFFSET + 16,
            actual: DATA_PAYLOAD_OFFSET + 8,
        })
    );
    assert!(rx.now_or_never().is_none());

    let mut malformed_assignment = assigned_object_packet(9, 1);
    malformed_assignment.truncate(ASSIGNED_OBJECT_PACKET_SIZE - 1);
    write_u32(
        &mut malformed_assignment,
        0,
        (ASSIGNED_OBJECT_PACKET_SIZE - 1) as u32,
    );
    let mut backend = FakeBackend::default();
    backend.packet(malformed_assignment);
    let mut driver = Driver::new(backend);
    assert_eq!(
        driver.drain_dispatch(),
        Err(Error::InvalidPacket {
            expected: ASSIGNED_OBJECT_PACKET_SIZE,
            actual: ASSIGNED_OBJECT_PACKET_SIZE - 1,
        })
    );
}

#[test]
fn disappearing_consumers_are_removed_while_routing() {
    let mut driver = Driver::new(FakeBackend::default());
    let rx = register_once(&mut driver, 12);
    drop(rx);
    driver.backend.packet(data_packet(12, 0, 1, 1));
    driver.drain_dispatch().unwrap();

    let replacement = register_once(&mut driver, 12);
    assert!(replacement.now_or_never().is_none());
}

#[test]
fn mapped_functions_execute_only_when_the_consumer_runs_the_thunk() {
    let ran = Arc::new(AtomicBool::new(false));
    let map_ran = Arc::clone(&ran);
    let mut driver = Driver::new(FakeBackend::default());
    let (items_tx, mut items_rx) = mpsc::channel(1);
    let (errors_tx, _errors_rx) = mpsc::unbounded();
    let (ready_tx, ready_rx) = oneshot::channel();
    driver.subscribe_mapped::<TestData, u32, _>(
        13,
        1,
        DataPeriod::VisualFrame,
        MappedTarget {
            tx: items_tx,
            errors: errors_tx,
            map: move |_| {
                map_ran.store(true, Ordering::SeqCst);
                panic!("consumer panic")
            },
        },
        ready_tx,
    );
    assert_eq!(ready_rx.now_or_never().unwrap().unwrap(), Ok(()));
    driver.backend.packet(data_packet(13, 0, 1, 5));
    driver.drain_dispatch().unwrap();
    assert!(!ran.load(Ordering::SeqCst));

    let thunk = items_rx.next().now_or_never().unwrap().unwrap();
    assert!(catch_unwind(AssertUnwindSafe(thunk)).is_err());
    assert!(ran.load(Ordering::SeqCst));
}
