#![cfg_attr(not(windows), allow(dead_code))]

use crate::backend::{
    ClientDataId, ClientDataPeriod, ClientDataRequest, ClientDefinitionId, DataPeriod, DataRequest,
    DefinitionId, EventId, ObjectId, RequestId, SimConnectBackend,
};
use crate::client_layout;
use crate::latest::Sender as LatestSender;
use crate::packet::decode_value;
use crate::routing::Registry;
use crate::{Error, FreezeState, Result, ServerException};
use futures_channel::{mpsc, oneshot};
use std::any::TypeId;
use std::collections::HashMap;
use std::ffi::CString;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

const RECV_HEADER_SIZE: usize = 12;
const EXCEPTION_PACKET_SIZE: usize = 24;
const DATA_PAYLOAD_OFFSET: usize = 40;

enum DispatchControl {
    Continue,
    QueueEmpty,
    Quit,
}

pub(crate) struct DataDefinitionEntry {
    pub name: CString,
    pub units: CString,
    pub datatype: u32,
    pub epsilon: f32,
}

pub(crate) trait SimData: Copy + Send + 'static {
    fn definitions() -> Result<Vec<DataDefinitionEntry>>;
}

pub(crate) trait ClientData: Copy + Send + 'static {
    fn definitions() -> Vec<client_layout::FieldDefinition>;
}

pub(crate) enum DeliverySender<T> {
    Buffered(mpsc::Sender<T>),
    Latest(LatestSender<T>),
}

impl<T> DeliverySender<T> {
    fn send(&mut self, value: T) -> bool {
        match self {
            Self::Buffered(sender) => match sender.try_send(value) {
                Ok(()) => true,
                Err(error) if error.is_full() => true,
                Err(_) => false,
            },
            Self::Latest(sender) => sender.send(value),
        }
    }

    fn close(&mut self) {
        match self {
            Self::Buffered(sender) => sender.close_channel(),
            Self::Latest(sender) => sender.close(),
        }
    }
}

pub(crate) type MappedThunk<E> = Box<dyn FnOnce() -> E + Send + 'static>;

pub(crate) struct MappedTarget<E, F> {
    pub tx: mpsc::Sender<MappedThunk<E>>,
    pub errors: mpsc::UnboundedSender<Error>,
    pub map: F,
}

#[derive(Clone, Copy)]
pub(crate) struct RequestSettings {
    pub period: DataPeriod,
    pub changed_only: bool,
    pub origin: u32,
    pub interval: u32,
    pub limit: u32,
}

impl RequestSettings {
    pub(crate) fn once() -> Self {
        Self {
            period: DataPeriod::Once,
            changed_only: true,
            origin: 0,
            interval: 0,
            limit: 0,
        }
    }

    pub(crate) fn recurring(period: DataPeriod) -> Self {
        Self {
            period,
            ..Self::once()
        }
    }
}

pub(crate) struct Driver<B> {
    backend: B,
    definitions: HashMap<TypeId, DefinitionId>,
    client_definitions: HashMap<TypeId, ClientDefinitionId>,
    client_data_ids: HashMap<String, ClientDataId>,
    requests: Registry<Box<dyn Route>, ActiveRequest>,
    pub(crate) exception_sinks: Vec<mpsc::UnboundedSender<ServerException>>,
    freeze_event_ids: [Option<EventId>; 3],
    next_event_id: EventId,
    next_define_id: DefinitionId,
    next_client_define_id: ClientDefinitionId,
    next_client_data_id: ClientDataId,
    dispatch_buffer: Vec<u8>,
}

impl<B: SimConnectBackend> Driver<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            definitions: HashMap::new(),
            client_definitions: HashMap::new(),
            client_data_ids: HashMap::new(),
            requests: Registry::new(),
            exception_sinks: Vec::new(),
            freeze_event_ids: [None; 3],
            next_event_id: 0,
            next_define_id: 0,
            next_client_define_id: 0,
            next_client_data_id: 0,
            dispatch_buffer: Vec::new(),
        }
    }

    #[cfg(test)]
    fn backend(&self) -> &B {
        &self.backend
    }

    pub(crate) fn request_once<T>(
        &mut self,
        request_id: RequestId,
        object_id: ObjectId,
        tx: oneshot::Sender<Result<T>>,
    ) where
        T: SimData,
    {
        let route = OneShotRoute::<T> { tx: Some(tx) };
        let _ = self.register_request::<T>(
            request_id,
            object_id,
            RequestSettings::once(),
            Box::new(route),
        );
    }

    pub(crate) fn set_data_on_sim_object<T>(&mut self, object_id: ObjectId, data: &T) -> Result<()>
    where
        T: SimData,
    {
        let define_id = self.define::<T>()?;
        self.backend
            .set_data_on_sim_object(define_id, object_id, value_bytes(data))
    }

    pub(crate) fn release_ai_control(
        &mut self,
        object_id: ObjectId,
        request_id: RequestId,
    ) -> Result<()> {
        self.backend.release_ai_control(object_id, request_id)
    }

    pub(crate) fn set_freeze(&mut self, object_id: ObjectId, state: FreezeState) -> Result<()> {
        self.transmit_freeze_event(
            object_id,
            0,
            "FREEZE_LATITUDE_LONGITUDE_SET",
            state.latitude_longitude,
        )?;
        self.transmit_freeze_event(object_id, 1, "FREEZE_ALTITUDE_SET", state.altitude)?;
        self.transmit_freeze_event(object_id, 2, "FREEZE_ATTITUDE_SET", state.attitude)?;
        Ok(())
    }

    fn transmit_freeze_event(
        &mut self,
        object_id: ObjectId,
        index: usize,
        name: &str,
        freeze: bool,
    ) -> Result<()> {
        let event_id = self.freeze_event_id(index, name)?;
        self.backend
            .transmit_client_event(object_id, event_id, u32::from(freeze))
    }

    fn freeze_event_id(&mut self, index: usize, name: &str) -> Result<EventId> {
        if let Some(event_id) = self.freeze_event_ids[index] {
            return Ok(event_id);
        }

        let event_id = self.next_event_id;
        let name = CString::new(name).map_err(|_| Error::InvalidCString)?;
        self.backend.map_client_event(event_id, &name)?;
        self.next_event_id = self
            .next_event_id
            .checked_add(1)
            .ok_or(Error::IdExhausted)?;
        self.freeze_event_ids[index] = Some(event_id);
        Ok(event_id)
    }

    pub(crate) fn subscribe<T>(
        &mut self,
        request_id: RequestId,
        object_id: ObjectId,
        settings: RequestSettings,
        tx: DeliverySender<T>,
        errors: mpsc::UnboundedSender<Error>,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: SimData,
    {
        let route = SubscriptionRoute::<T> {
            tx,
            errors,
            remaining: (settings.limit != 0).then_some(settings.limit),
        };
        let result = self.register_request::<T>(request_id, object_id, settings, Box::new(route));
        let _ = ready.send(result);
    }

    pub(crate) fn subscribe_mapped<T, E, F>(
        &mut self,
        request_id: RequestId,
        object_id: ObjectId,
        period: DataPeriod,
        target: MappedTarget<E, F>,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: SimData,
        E: Send + 'static,
        F: FnMut(T) -> E + Send + 'static,
    {
        let route = MappedSubscriptionRoute::<T, E, F> {
            tx: target.tx,
            errors: target.errors,
            map: Arc::new(Mutex::new(target.map)),
            _item: PhantomData,
        };
        let result = self.register_request::<T>(
            request_id,
            object_id,
            RequestSettings::recurring(period),
            Box::new(route),
        );
        let _ = ready.send(result);
    }

    fn register_request<T>(
        &mut self,
        request_id: RequestId,
        object_id: ObjectId,
        settings: RequestSettings,
        mut route: Box<dyn Route>,
    ) -> Result<()>
    where
        T: SimData,
    {
        if self.requests.contains(request_id) {
            let error = Error::IdExhausted;
            route.fail(error.clone());
            return Err(error);
        }
        let define_id = match self.define::<T>() {
            Ok(definition) => definition,
            Err(error) => {
                route.fail(error.clone());
                return Err(error);
            }
        };
        let request = DataRequest {
            request_id,
            define_id,
            object_id,
            period: settings.period,
            changed_only: settings.changed_only,
            origin: settings.origin,
            interval: settings.interval,
            limit: settings.limit,
        };
        if let Err(error) = self.backend.request_data(request) {
            route.fail(error.clone());
            return Err(error);
        }

        let inserted = self.requests.insert(
            request_id,
            route,
            ActiveRequest::SimObject {
                define_id,
                object_id,
                periodic: settings.period != DataPeriod::Once,
            },
        );
        debug_assert!(inserted);
        self.remember_last_send_id(request_id);
        Ok(())
    }

    fn define<T>(&mut self) -> Result<DefinitionId>
    where
        T: SimData,
    {
        let type_id = TypeId::of::<T>();
        if let Some(define_id) = self.definitions.get(&type_id) {
            return Ok(*define_id);
        }

        let definitions = T::definitions()?;
        let define_id = self.next_define_id;
        self.next_define_id = self
            .next_define_id
            .checked_add(1)
            .ok_or(Error::IdExhausted)?;

        for definition in definitions {
            if let Err(operation) = self.backend.add_data_definition(
                define_id,
                &definition.name,
                &definition.units,
                definition.datatype,
                definition.epsilon,
            ) {
                return match self.backend.clear_data_definition(define_id) {
                    Ok(()) => Err(operation),
                    Err(rollback) => Err(Error::RollbackFailed {
                        operation: Box::new(operation),
                        rollback: Box::new(rollback),
                    }),
                };
            }
        }

        self.definitions.insert(type_id, define_id);
        Ok(define_id)
    }

    pub(crate) fn create_client_data<T>(&mut self, name: &str) -> Result<ClientDataId>
    where
        T: ClientData,
    {
        let client_id = self.client_data_id(name)?;
        let size = std::mem::size_of::<T>()
            .try_into()
            .map_err(|_| Error::InvalidClientDataDefinition)?;
        self.backend.create_client_data(client_id, size)?;
        Ok(client_id)
    }

    pub(crate) fn client_data_id(&mut self, name: &str) -> Result<ClientDataId> {
        if let Some(client_id) = self.client_data_ids.get(name) {
            return Ok(*client_id);
        }
        let c_name = CString::new(name).map_err(|_| Error::InvalidCString)?;
        let client_id = self.next_client_data_id;
        self.next_client_data_id = self
            .next_client_data_id
            .checked_add(1)
            .ok_or(Error::IdExhausted)?;
        self.backend.map_client_data_name(&c_name, client_id)?;
        self.client_data_ids.insert(name.to_owned(), client_id);
        Ok(client_id)
    }

    fn client_define<T>(&mut self) -> Result<ClientDefinitionId>
    where
        T: ClientData,
    {
        let type_id = TypeId::of::<T>();
        let sdk_definitions = client_layout::plan(T::definitions(), std::mem::size_of::<T>())?;
        if let Some(define_id) = self.client_definitions.get(&type_id) {
            return Ok(*define_id);
        }

        let define_id = self.next_client_define_id;
        self.next_client_define_id = self
            .next_client_define_id
            .checked_add(1)
            .ok_or(Error::IdExhausted)?;
        for (offset, size_or_type, epsilon) in sdk_definitions {
            let offset = offset
                .try_into()
                .map_err(|_| Error::InvalidClientDataDefinition)?;
            if let Err(operation) =
                self.backend
                    .add_client_data_definition(define_id, offset, size_or_type, epsilon)
            {
                return match self.backend.clear_client_data_definition(define_id) {
                    Ok(()) => Err(operation),
                    Err(rollback) => Err(Error::RollbackFailed {
                        operation: Box::new(operation),
                        rollback: Box::new(rollback),
                    }),
                };
            }
        }
        self.client_definitions.insert(type_id, define_id);
        Ok(define_id)
    }

    pub(crate) fn set_client_data<T>(&mut self, client_id: ClientDataId, data: &T) -> Result<()>
    where
        T: ClientData,
    {
        let define_id = self.client_define::<T>()?;
        // SAFETY: ClientData requires these ranges to describe initialized fields.
        let bytes = unsafe { client_layout::encode(data, &T::definitions())? };
        self.backend.set_client_data(client_id, define_id, &bytes)
    }

    pub(crate) fn subscribe_client_data<T>(
        &mut self,
        request_id: RequestId,
        name: &str,
        tx: mpsc::Sender<T>,
        errors: mpsc::UnboundedSender<Error>,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: ClientData,
    {
        let result = self.register_client_data::<T>(request_id, name, tx, errors);
        let _ = ready.send(result);
    }

    fn register_client_data<T>(
        &mut self,
        request_id: RequestId,
        name: &str,
        tx: mpsc::Sender<T>,
        errors: mpsc::UnboundedSender<Error>,
    ) -> Result<()>
    where
        T: ClientData,
    {
        if self.requests.contains(request_id) {
            return Err(Error::IdExhausted);
        }
        let define_id = self.client_define::<T>()?;
        let client_id = self.client_data_id(name)?;
        self.backend.request_client_data(ClientDataRequest {
            client_id,
            request_id,
            define_id,
            period: ClientDataPeriod::OnSet,
            changed_only: true,
            origin: 0,
            interval: 0,
            limit: 0,
        })?;

        let inserted = self.requests.insert(
            request_id,
            Box::new(ClientDataSubscriptionRoute::<T> { tx, errors }),
            ActiveRequest::ClientData {
                client_id,
                define_id,
            },
        );
        debug_assert!(inserted);
        self.remember_last_send_id(request_id);
        Ok(())
    }

    fn remember_last_send_id(&mut self, request_id: RequestId) {
        if let Ok(send_id) = self.backend.last_send_id() {
            self.requests.link_send_id(send_id, request_id);
        }
    }

    /// Drain queued packets and return whether the connection remains usable.
    pub(crate) fn drain_dispatch(&mut self) -> Result<bool> {
        let mut packet = std::mem::take(&mut self.dispatch_buffer);
        loop {
            packet.clear();
            if !self.backend.next_dispatch(&mut packet)? {
                self.dispatch_buffer = packet;
                return Ok(true);
            }
            match self.dispatch(&packet)? {
                DispatchControl::Continue => {}
                DispatchControl::QueueEmpty => {
                    self.dispatch_buffer = packet;
                    return Ok(true);
                }
                DispatchControl::Quit => {
                    self.dispatch_buffer = packet;
                    return Ok(false);
                }
            }
        }
    }

    fn dispatch(&mut self, packet: &[u8]) -> Result<DispatchControl> {
        require_size(packet, RECV_HEADER_SIZE)?;
        let declared_size = read_u32(packet, 0)? as usize;
        if declared_size > packet.len() {
            return Err(Error::InvalidPacket {
                expected: declared_size,
                actual: packet.len(),
            });
        }
        let packet = &packet[..declared_size];
        let id = read_u32(packet, 8)?;
        match id {
            id if id == B::RECV_ID_NULL => return Ok(DispatchControl::QueueEmpty),
            id if id == B::RECV_ID_SIMOBJECT_DATA
                || id == B::RECV_ID_SIMOBJECT_DATA_BYTYPE
                || id == B::RECV_ID_CLIENT_DATA =>
            {
                require_size(packet, DATA_PAYLOAD_OFFSET)?;
                let request_id = read_u32(packet, 12)?;
                let define_id = read_u32(packet, 20)?;
                let define_count = read_u32(packet, 36)?;
                let data_size = (define_count as usize)
                    .checked_mul(8)
                    .and_then(|size| DATA_PAYLOAD_OFFSET.checked_add(size))
                    .ok_or(Error::InvalidPacket {
                        expected: usize::MAX,
                        actual: packet.len(),
                    })?;
                require_size(packet, data_size)?;
                let expected = self
                    .requests
                    .active(request_id)
                    .map(ActiveRequest::define_id);
                let remove = match (self.requests.route_mut(request_id), expected) {
                    (Some(route), Some(expected_id)) => {
                        if define_id != expected_id {
                            route.fail(Error::DefinitionMismatch {
                                expected: expected_id,
                                actual: define_id,
                            });
                            true
                        } else {
                            route.deliver(packet, DATA_PAYLOAD_OFFSET)
                        }
                    }
                    _ => false,
                };
                if remove {
                    self.remove_request(request_id);
                }
            }
            id if id == B::RECV_ID_EXCEPTION => {
                require_size(packet, EXCEPTION_PACKET_SIZE)?;
                let server_exception = ServerException {
                    code: read_u32(packet, 12)?,
                    send_id: read_u32(packet, 16)?,
                    index: read_u32(packet, 20)?,
                };
                self.exception_sinks
                    .retain(|sink| sink.unbounded_send(server_exception.clone()).is_ok());
                let error = Error::SimConnectException(server_exception.clone());
                if let Some(request_id) =
                    self.requests.request_for_send_id(server_exception.send_id)
                {
                    if let Some(route) = self.requests.route_mut(request_id) {
                        route.fail(error);
                    }
                    self.remove_request(request_id);
                }
            }
            id if id == B::RECV_ID_QUIT => return Ok(DispatchControl::Quit),
            _ => {}
        }
        Ok(DispatchControl::Continue)
    }

    pub(crate) fn cancel(&mut self, request_id: RequestId) {
        let (_, active) = self.requests.remove(request_id);
        if let Some(active) = active {
            match active {
                ActiveRequest::SimObject {
                    define_id,
                    object_id,
                    periodic: true,
                    ..
                } => {
                    let _ = self.backend.request_data(DataRequest {
                        request_id,
                        define_id,
                        object_id,
                        period: DataPeriod::Never,
                        changed_only: true,
                        origin: 0,
                        interval: 0,
                        limit: 0,
                    });
                }
                ActiveRequest::ClientData {
                    client_id,
                    define_id,
                    ..
                } => {
                    let _ = self.backend.request_client_data(ClientDataRequest {
                        client_id,
                        request_id,
                        define_id,
                        period: ClientDataPeriod::Never,
                        changed_only: true,
                        origin: 0,
                        interval: 0,
                        limit: 0,
                    });
                }
                ActiveRequest::SimObject { .. } => {}
            }
        }
    }

    fn remove_request(&mut self, request_id: RequestId) {
        self.requests.remove(request_id);
    }

    pub(crate) fn fail_all(&mut self, error: Error) {
        self.requests.fail_all(|route| route.fail(error.clone()));
    }
}

enum ActiveRequest {
    SimObject {
        define_id: DefinitionId,
        object_id: ObjectId,
        periodic: bool,
    },
    ClientData {
        client_id: ClientDataId,
        define_id: ClientDefinitionId,
    },
}

impl ActiveRequest {
    fn define_id(&self) -> u32 {
        match self {
            Self::SimObject { define_id, .. } => *define_id,
            Self::ClientData { define_id, .. } => *define_id,
        }
    }
}

trait Route: Send {
    /// Deliver a packet and return whether the route is complete.
    fn deliver(&mut self, packet: &[u8], payload_offset: usize) -> bool;
    fn fail(&mut self, error: Error);
}

struct OneShotRoute<T> {
    tx: Option<oneshot::Sender<Result<T>>>,
}

impl<T: SimData> Route for OneShotRoute<T> {
    fn deliver(&mut self, packet: &[u8], payload_offset: usize) -> bool {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(decode_value(packet, payload_offset));
        }
        true
    }

    fn fail(&mut self, error: Error) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Err(error));
        }
    }
}

struct SubscriptionRoute<T> {
    tx: DeliverySender<T>,
    errors: mpsc::UnboundedSender<Error>,
    remaining: Option<u32>,
}

impl<T: SimData> Route for SubscriptionRoute<T> {
    fn deliver(&mut self, packet: &[u8], payload_offset: usize) -> bool {
        match decode_value(packet, payload_offset) {
            Ok(value) => {
                if !self.tx.send(value) {
                    return true;
                }
                if let Some(remaining) = &mut self.remaining {
                    *remaining -= 1;
                    *remaining == 0
                } else {
                    false
                }
            }
            Err(error) => {
                self.fail(error);
                true
            }
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.errors.unbounded_send(error);
        self.errors.close_channel();
        self.tx.close();
    }
}

struct MappedSubscriptionRoute<T, E, F> {
    tx: mpsc::Sender<MappedThunk<E>>,
    errors: mpsc::UnboundedSender<Error>,
    map: Arc<Mutex<F>>,
    _item: PhantomData<T>,
}

impl<T, E, F> Route for MappedSubscriptionRoute<T, E, F>
where
    T: SimData,
    E: Send + 'static,
    F: FnMut(T) -> E + Send + 'static,
{
    fn deliver(&mut self, packet: &[u8], payload_offset: usize) -> bool {
        match decode_value(packet, payload_offset) {
            Ok(value) => {
                let map = Arc::clone(&self.map);
                let thunk = Box::new(move || {
                    let mut map = map.lock().unwrap_or_else(|error| error.into_inner());
                    (map)(value)
                });
                match self.tx.try_send(thunk) {
                    Ok(()) => false,
                    Err(error) if error.is_full() => false,
                    Err(_) => true,
                }
            }
            Err(error) => {
                self.fail(error);
                true
            }
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.errors.unbounded_send(error);
        self.errors.close_channel();
        self.tx.close_channel();
    }
}

struct ClientDataSubscriptionRoute<T> {
    tx: mpsc::Sender<T>,
    errors: mpsc::UnboundedSender<Error>,
}

impl<T: ClientData> Route for ClientDataSubscriptionRoute<T> {
    fn deliver(&mut self, packet: &[u8], payload_offset: usize) -> bool {
        match decode_value(packet, payload_offset) {
            Ok(value) => match self.tx.try_send(value) {
                Ok(()) => false,
                Err(error) if error.is_full() => false,
                Err(_) => true,
            },
            Err(error) => {
                self.fail(error);
                true
            }
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.errors.unbounded_send(error);
        self.errors.close_channel();
        self.tx.close_channel();
    }
}

fn value_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: a shared value is readable for exactly its initialized object size.
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(value).cast::<u8>(),
            std::mem::size_of::<T>(),
        )
    }
}

fn require_size(packet: &[u8], expected: usize) -> Result<()> {
    if packet.len() < expected {
        Err(Error::InvalidPacket {
            expected,
            actual: packet.len(),
        })
    } else {
        Ok(())
    }
}

fn read_u32(packet: &[u8], offset: usize) -> Result<u32> {
    require_size(packet, offset + 4)?;
    Ok(u32::from_ne_bytes(
        packet[offset..offset + 4]
            .try_into()
            .expect("length checked"),
    ))
}

#[cfg(test)]
mod tests;
