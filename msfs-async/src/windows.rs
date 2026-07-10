use crate::{AsyncClientDataDefinition, AsyncDataDefinition, Error, FreezeState, Period, Result};
use futures_channel::{mpsc, oneshot};
use futures_core::Stream;
use msfs::sys;
use std::any::TypeId;
use std::collections::HashMap;
use std::ffi::CString;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::task::{Context, Poll};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};

const DEFAULT_SUBSCRIPTION_CAPACITY: usize = 32;

/// A cloneable handle to the native SimConnect driver.
#[derive(Clone)]
pub struct AsyncSimConnect {
    inner: Arc<ClientInner>,
}

impl AsyncSimConnect {
    /// Open a SimConnect session and start its event-driven driver thread.
    pub async fn open(name: impl Into<String>) -> Result<Self> {
        let name = CString::new(name.into()).map_err(|_| Error::InvalidCString)?;
        let sim_event = OwnedEvent::new()?;
        let command_event = OwnedEvent::new()?;
        let command_signal = command_event.signal();
        let (command_tx, command_rx) = std_mpsc::channel();
        let (open_tx, open_rx) = oneshot::channel();

        let inner = Arc::new(ClientInner {
            commands: command_tx,
            command_event: command_signal,
            next_request_id: AtomicU32::new(0),
        });

        std::thread::Builder::new()
            .name("msfs-simconnect".to_owned())
            .spawn(move || {
                Driver::run(name, sim_event, command_event, command_rx, open_tx);
            })
            .map_err(|error| Error::ThreadStart(error.to_string()))?;

        open_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Self { inner })
    }

    /// Request one owned value from the user's aircraft or another simulation object.
    pub async fn request_once<T>(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<T>
    where
        T: AsyncDataDefinition,
    {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.request_once::<T>(request_id, object_id, tx);
        })))?;

        rx.await.map_err(|_| Error::DriverStopped)?
    }

    /// Replace the values in a simulation object's data definition.
    ///
    /// Successful completion means SimConnect accepted the local API call.
    /// Server-side validation failures are delivered later through
    /// [`Self::exceptions`].
    pub async fn set_data_on_sim_object<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        data: &T,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
    {
        let data = *data;
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.set_data_on_sim_object::<T>(object_id, &data));
        })))?;
        rx.await.map_err(|_| Error::DriverStopped)?
    }

    /// Release an object from the simulator's AI controller.
    ///
    /// This is normally required before repeatedly writing position or
    /// attitude to an AI object. Successful completion means SimConnect
    /// accepted the local API call; delayed failures arrive on
    /// [`Self::exceptions`].
    pub async fn release_ai_control(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<()> {
        let request_id = self.next_request_id();
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.release_ai_control(object_id, request_id));
        })))?;
        rx.await.map_err(|_| Error::DriverStopped)?
    }

    /// Set which position and attitude components the simulator should freeze.
    ///
    /// This transmits the three `FREEZE_*_SET` simulation events. The events
    /// are sent separately, so an error can leave a partially updated state.
    /// Delayed server-side failures arrive on [`Self::exceptions`].
    pub async fn set_freeze(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        state: FreezeState,
    ) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.set_freeze(object_id, state));
        })))?;
        rx.await.map_err(|_| Error::DriverStopped)?
    }

    /// Subscribe to asynchronous exceptions reported by the SimConnect server.
    pub fn exceptions(&self) -> Result<ExceptionStream> {
        let (tx, rx) = mpsc::unbounded();
        self.send(Command::Run(Box::new(move |driver| {
            driver.exception_sinks.push(tx);
        })))?;
        Ok(ExceptionStream { items: rx })
    }

    /// Subscribe to typed simulation-object data using a bounded buffer.
    ///
    /// If the consumer falls behind, new values are discarded until capacity
    /// becomes available. Use [`Self::subscribe_with_capacity`] to tune the
    /// buffer for high-frequency data.
    pub async fn subscribe<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: Period,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        self.subscribe_with_capacity(object_id, period, DEFAULT_SUBSCRIPTION_CAPACITY)
            .await
    }

    /// Subscribe with an explicit bounded channel capacity.
    pub async fn subscribe_with_capacity<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: Period,
        capacity: usize,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        let period = recurring_period(period)?;
        let request_id = self.next_request_id();
        let (items_tx, items_rx) = mpsc::channel(capacity);
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe::<T>(request_id, object_id, period, items_tx, ready_tx);
        })))?;

        ready_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Subscription {
            request_id,
            items: items_rx,
            inner: Arc::clone(&self.inner),
            _item: PhantomData,
        })
    }

    /// Register a typed route which maps its values into a shared event stream.
    #[doc(hidden)]
    pub async fn subscribe_mapped<T, E, F>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: Period,
        events: MappedEventSender<E>,
        map: F,
    ) -> Result<MappedSubscription>
    where
        T: AsyncDataDefinition,
        E: Send + 'static,
        F: FnMut(T) -> E + Send + 'static,
    {
        let period = recurring_period(period)?;
        let request_id = self.next_request_id();
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe_mapped::<T, E, F>(
                request_id,
                object_id,
                period,
                events.items,
                map,
                ready_tx,
            );
        })))?;

        ready_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(MappedSubscription {
            request_id,
            inner: Arc::clone(&self.inner),
        })
    }

    /// Allocate a named client-data area owned by this SimConnect session.
    pub async fn create_client_data<T>(&self, name: impl Into<String>) -> Result<ClientDataArea<T>>
    where
        T: AsyncClientDataDefinition,
    {
        let name = name.into();
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.create_client_data::<T>(&name));
        })))?;

        let client_id = rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(ClientDataArea {
            client_id,
            _item: PhantomData,
        })
    }

    /// Open a handle to a named client-data area created by another client.
    pub async fn get_client_data_area<T>(
        &self,
        name: impl Into<String>,
    ) -> Result<ClientDataArea<T>>
    where
        T: AsyncClientDataDefinition,
    {
        let name = name.into();
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.client_data_id(&name));
        })))?;

        let client_id = rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(ClientDataArea {
            client_id,
            _item: PhantomData,
        })
    }

    /// Write a complete value into a client-data area.
    pub async fn set_client_data<T>(&self, area: &ClientDataArea<T>, data: &T) -> Result<()>
    where
        T: AsyncClientDataDefinition,
    {
        let client_id = area.client_id;
        let data = *data;
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.set_client_data::<T>(client_id, &data));
        })))?;
        rx.await.map_err(|_| Error::DriverStopped)?
    }

    /// Subscribe to changes in a named client-data area.
    pub async fn subscribe_client_data<T>(&self, name: impl Into<String>) -> Result<Subscription<T>>
    where
        T: AsyncClientDataDefinition,
    {
        let request_id = self.next_request_id();
        let name = name.into();
        let (items_tx, items_rx) = mpsc::channel(DEFAULT_SUBSCRIPTION_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe_client_data::<T>(request_id, &name, items_tx, ready_tx);
        })))?;

        ready_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Subscription {
            request_id,
            items: items_rx,
            inner: Arc::clone(&self.inner),
            _item: PhantomData,
        })
    }

    fn next_request_id(&self) -> sys::SIMCONNECT_DATA_REQUEST_ID {
        self.inner.next_request_id.fetch_add(1, Ordering::Relaxed) as _
    }

    fn send(&self, command: Command) -> Result<()> {
        self.inner
            .commands
            .send(command)
            .map_err(|_| Error::DriverStopped)?;
        self.inner.command_event.set()
    }
}

/// A typed handle to a named SimConnect client-data area.
#[derive(Clone, Copy, Debug)]
pub struct ClientDataArea<T> {
    client_id: sys::SIMCONNECT_CLIENT_DATA_ID,
    _item: PhantomData<T>,
}

/// Create the shared channel used by mapped subscriptions.
#[doc(hidden)]
pub fn mapped_event_channel<E>(capacity: usize) -> (MappedEventSender<E>, MappedEventStream<E>) {
    let (tx, rx) = mpsc::channel(capacity);
    (
        MappedEventSender { items: tx },
        MappedEventStream { items: rx },
    )
}

/// Sending half of a shared mapped-subscription channel.
#[doc(hidden)]
pub struct MappedEventSender<E> {
    items: mpsc::Sender<Result<E>>,
}

impl<E> Clone for MappedEventSender<E> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
        }
    }
}

/// Stream receiving values from several mapped subscriptions.
#[doc(hidden)]
pub struct MappedEventStream<E> {
    items: mpsc::Receiver<Result<E>>,
}

impl<E> Unpin for MappedEventStream<E> {}

impl<E> Stream for MappedEventStream<E> {
    type Item = Result<E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().items).poll_next(cx)
    }
}

/// Cancellation guard for a mapped recurring request.
#[doc(hidden)]
pub struct MappedSubscription {
    request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    inner: Arc<ClientInner>,
}

impl Drop for MappedSubscription {
    fn drop(&mut self) {
        if self
            .inner
            .commands
            .send(Command::Cancel(self.request_id))
            .is_ok()
        {
            let _ = self.inner.command_event.set();
        }
    }
}

/// A server-side SimConnect exception received after an API call was submitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerException {
    pub code: u32,
    pub send_id: u32,
    pub index: u32,
}

impl std::fmt::Display for ServerException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SimConnect exception {} for send ID {} at argument {}",
            self.code, self.send_id, self.index
        )
    }
}

impl std::error::Error for ServerException {}

/// A stream of asynchronous server-side SimConnect exceptions.
pub struct ExceptionStream {
    items: mpsc::UnboundedReceiver<ServerException>,
}

impl Stream for ExceptionStream {
    type Item = ServerException;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().items).poll_next(cx)
    }
}

/// A typed recurring SimConnect request.
pub struct Subscription<T> {
    request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    items: mpsc::Receiver<Result<T>>,
    inner: Arc<ClientInner>,
    _item: PhantomData<T>,
}

impl<T> Unpin for Subscription<T> {}

impl<T> Stream for Subscription<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().items).poll_next(cx)
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        if self
            .inner
            .commands
            .send(Command::Cancel(self.request_id))
            .is_ok()
        {
            let _ = self.inner.command_event.set();
        }
    }
}

struct ClientInner {
    commands: std_mpsc::Sender<Command>,
    command_event: EventSignal,
    next_request_id: AtomicU32,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.command_event.set();
    }
}

enum Command {
    Run(Box<dyn FnOnce(&mut Driver) + Send + 'static>),
    Cancel(sys::SIMCONNECT_DATA_REQUEST_ID),
    Shutdown,
}

struct Driver {
    connection: RawConnection,
    definitions: HashMap<TypeId, sys::SIMCONNECT_DATA_DEFINITION_ID>,
    client_definitions: HashMap<TypeId, sys::SIMCONNECT_CLIENT_DATA_DEFINITION_ID>,
    client_data_ids: HashMap<String, sys::SIMCONNECT_CLIENT_DATA_ID>,
    routes: HashMap<sys::SIMCONNECT_DATA_REQUEST_ID, Box<dyn Route>>,
    active: HashMap<sys::SIMCONNECT_DATA_REQUEST_ID, ActiveRequest>,
    send_ids: HashMap<u32, sys::SIMCONNECT_DATA_REQUEST_ID>,
    exception_sinks: Vec<mpsc::UnboundedSender<ServerException>>,
    freeze_event_ids: [Option<sys::DWORD>; 3],
    next_event_id: sys::DWORD,
}

impl Driver {
    fn run(
        name: CString,
        sim_event: OwnedEvent,
        command_event: OwnedEvent,
        commands: std_mpsc::Receiver<Command>,
        opened: oneshot::Sender<Result<()>>,
    ) {
        let connection = match RawConnection::open(&name, sim_event.raw()) {
            Ok(connection) => connection,
            Err(error) => {
                let _ = opened.send(Err(error));
                return;
            }
        };

        let mut driver = Self {
            connection,
            definitions: HashMap::new(),
            client_definitions: HashMap::new(),
            client_data_ids: HashMap::new(),
            routes: HashMap::new(),
            active: HashMap::new(),
            send_ids: HashMap::new(),
            exception_sinks: Vec::new(),
            freeze_event_ids: [None; 3],
            next_event_id: 0,
        };
        let _ = opened.send(Ok(()));

        let handles = [sim_event.raw(), command_event.raw()];
        let outcome = loop {
            let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            match wait {
                WAIT_OBJECT_0 => {
                    if let Err(error) = driver.drain_dispatch() {
                        break Err(error);
                    }
                }
                value if value == WAIT_OBJECT_0 + 1 => {
                    if !driver.drain_commands(&commands) {
                        break Ok(());
                    }
                }
                WAIT_FAILED => break Err(last_windows_error()),
                _ => break Err(last_windows_error()),
            }
        };

        if let Err(error) = outcome {
            driver.fail_all(error);
        }
    }

    fn drain_commands(&mut self, commands: &std_mpsc::Receiver<Command>) -> bool {
        loop {
            match commands.try_recv() {
                Ok(Command::Run(command)) => command(self),
                Ok(Command::Cancel(request_id)) => self.cancel(request_id),
                Ok(Command::Shutdown) | Err(std_mpsc::TryRecvError::Disconnected) => return false,
                Err(std_mpsc::TryRecvError::Empty) => return true,
            }
        }
    }

    fn request_once<T>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        tx: oneshot::Sender<Result<T>>,
    ) where
        T: AsyncDataDefinition,
    {
        let route = OneShotRoute::<T> { tx: Some(tx) };
        let _ = self.register_request::<T>(
            request_id,
            object_id,
            sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_ONCE,
            false,
            Box::new(route),
        );
    }

    fn set_data_on_sim_object<T>(
        &mut self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        data: &T,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
    {
        let define_id = self.define::<T>()?;
        check_hresult(unsafe {
            sys::SimConnect_SetDataOnSimObject(
                self.connection.handle,
                define_id,
                object_id,
                0,
                0,
                std::mem::size_of::<T>() as sys::DWORD,
                std::ptr::from_ref(data).cast_mut().cast(),
            )
        })
    }

    fn release_ai_control(
        &mut self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    ) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_AIReleaseControl(self.connection.handle, object_id, request_id)
        })
    }

    fn set_freeze(
        &mut self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        state: FreezeState,
    ) -> Result<()> {
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
        object_id: sys::SIMCONNECT_OBJECT_ID,
        index: usize,
        name: &str,
        freeze: bool,
    ) -> Result<()> {
        let event_id = self.freeze_event_id(index, name)?;
        check_hresult(unsafe {
            sys::SimConnect_TransmitClientEvent(
                self.connection.handle,
                object_id,
                event_id,
                u32::from(freeze),
                0,
                0,
            )
        })
    }

    fn freeze_event_id(&mut self, index: usize, name: &str) -> Result<sys::DWORD> {
        if let Some(event_id) = self.freeze_event_ids[index] {
            return Ok(event_id);
        }

        let event_id = self.next_event_id;
        let name = CString::new(name).map_err(|_| Error::InvalidCString)?;
        check_hresult(unsafe {
            sys::SimConnect_MapClientEventToSimEvent(
                self.connection.handle,
                event_id,
                name.as_ptr(),
            )
        })?;
        self.next_event_id += 1;
        self.freeze_event_ids[index] = Some(event_id);
        Ok(event_id)
    }

    fn subscribe<T>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: sys::SIMCONNECT_PERIOD,
        tx: mpsc::Sender<Result<T>>,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: AsyncDataDefinition,
    {
        let route = SubscriptionRoute::<T> { tx };
        let result =
            self.register_request::<T>(request_id, object_id, period, true, Box::new(route));
        let _ = ready.send(result);
    }

    fn subscribe_mapped<T, E, F>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: sys::SIMCONNECT_PERIOD,
        tx: mpsc::Sender<Result<E>>,
        map: F,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: AsyncDataDefinition,
        E: Send + 'static,
        F: FnMut(T) -> E + Send + 'static,
    {
        let route = MappedSubscriptionRoute::<T, E, F> {
            tx,
            map,
            _item: PhantomData,
        };
        let result =
            self.register_request::<T>(request_id, object_id, period, true, Box::new(route));
        let _ = ready.send(result);
    }

    fn register_request<T>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: sys::SIMCONNECT_PERIOD,
        periodic: bool,
        mut route: Box<dyn Route>,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
    {
        let define_id = match self.define::<T>() {
            Ok(define_id) => define_id,
            Err(error) => {
                route.fail(error.clone());
                return Err(error);
            }
        };

        let result = unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                self.connection.handle,
                request_id,
                define_id,
                object_id,
                period,
                sys::SIMCONNECT_DATA_REQUEST_FLAG_CHANGED,
                0,
                0,
                0,
            )
        };
        if let Err(error) = check_hresult(result) {
            route.fail(error.clone());
            return Err(error);
        }

        self.routes.insert(request_id, route);
        self.active.insert(
            request_id,
            ActiveRequest::SimObject {
                define_id,
                object_id,
                periodic,
            },
        );
        self.remember_last_send_id(request_id);
        Ok(())
    }

    fn define<T>(&mut self) -> Result<sys::SIMCONNECT_DATA_DEFINITION_ID>
    where
        T: AsyncDataDefinition,
    {
        let type_id = TypeId::of::<T>();
        if let Some(define_id) = self.definitions.get(&type_id) {
            return Ok(*define_id);
        }

        let define_id = self.definitions.len() as sys::SIMCONNECT_DATA_DEFINITION_ID;
        for (datum_name, units, epsilon, datatype) in T::DEFINITIONS {
            let datum_name = CString::new(*datum_name).map_err(|_| Error::InvalidCString)?;
            let units = CString::new(*units).map_err(|_| Error::InvalidCString)?;
            check_hresult(unsafe {
                sys::SimConnect_AddToDataDefinition(
                    self.connection.handle,
                    define_id,
                    datum_name.as_ptr(),
                    units.as_ptr(),
                    *datatype,
                    *epsilon,
                    sys::SIMCONNECT_UNUSED,
                )
            })?;
        }

        self.definitions.insert(type_id, define_id);
        Ok(define_id)
    }

    fn create_client_data<T>(&mut self, name: &str) -> Result<sys::SIMCONNECT_CLIENT_DATA_ID>
    where
        T: AsyncClientDataDefinition,
    {
        let client_id = self.client_data_id(name)?;
        check_hresult(unsafe {
            sys::SimConnect_CreateClientData(
                self.connection.handle,
                client_id,
                std::mem::size_of::<T>() as sys::DWORD,
                0,
            )
        })?;
        Ok(client_id)
    }

    fn client_data_id(&mut self, name: &str) -> Result<sys::SIMCONNECT_CLIENT_DATA_ID> {
        if let Some(client_id) = self.client_data_ids.get(name) {
            return Ok(*client_id);
        }

        let client_id = self.client_data_ids.len() as sys::SIMCONNECT_CLIENT_DATA_ID;
        let c_name = CString::new(name).map_err(|_| Error::InvalidCString)?;
        check_hresult(unsafe {
            sys::SimConnect_MapClientDataNameToID(
                self.connection.handle,
                c_name.as_ptr(),
                client_id,
            )
        })?;
        self.client_data_ids.insert(name.to_owned(), client_id);
        Ok(client_id)
    }

    fn client_define<T>(&mut self) -> Result<sys::SIMCONNECT_CLIENT_DATA_DEFINITION_ID>
    where
        T: AsyncClientDataDefinition,
    {
        let type_id = TypeId::of::<T>();
        if let Some(define_id) = self.client_definitions.get(&type_id) {
            return Ok(*define_id);
        }

        let define_id = self.client_definitions.len() as sys::SIMCONNECT_CLIENT_DATA_DEFINITION_ID;
        let mut padding = usize::MAX;
        for (offset, size, epsilon) in T::get_definitions() {
            padding = padding.min(std::mem::size_of::<T>() - (offset + size));
            check_hresult(unsafe {
                sys::SimConnect_AddToClientDataDefinition(
                    self.connection.handle,
                    define_id,
                    offset as sys::DWORD,
                    size as sys::DWORD,
                    epsilon,
                    sys::SIMCONNECT_UNUSED,
                )
            })?;
        }

        if padding > 0 && padding != usize::MAX {
            check_hresult(unsafe {
                sys::SimConnect_AddToClientDataDefinition(
                    self.connection.handle,
                    define_id,
                    (std::mem::size_of::<T>() - padding) as sys::DWORD,
                    padding as sys::DWORD,
                    0.0,
                    sys::SIMCONNECT_UNUSED,
                )
            })?;
        }

        self.client_definitions.insert(type_id, define_id);
        Ok(define_id)
    }

    fn set_client_data<T>(
        &mut self,
        client_id: sys::SIMCONNECT_CLIENT_DATA_ID,
        data: &T,
    ) -> Result<()>
    where
        T: AsyncClientDataDefinition,
    {
        let define_id = self.client_define::<T>()?;
        check_hresult(unsafe {
            sys::SimConnect_SetClientData(
                self.connection.handle,
                client_id,
                define_id,
                0,
                0,
                std::mem::size_of::<T>() as sys::DWORD,
                std::ptr::from_ref(data).cast_mut().cast(),
            )
        })
    }

    fn subscribe_client_data<T>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        name: &str,
        tx: mpsc::Sender<Result<T>>,
        ready: oneshot::Sender<Result<()>>,
    ) where
        T: AsyncClientDataDefinition,
    {
        let result = self.register_client_data::<T>(request_id, name, tx);
        let _ = ready.send(result);
    }

    fn register_client_data<T>(
        &mut self,
        request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
        name: &str,
        tx: mpsc::Sender<Result<T>>,
    ) -> Result<()>
    where
        T: AsyncClientDataDefinition,
    {
        let define_id = self.client_define::<T>()?;
        let client_id = self.client_data_id(name)?;
        check_hresult(unsafe {
            sys::SimConnect_RequestClientData(
                self.connection.handle,
                client_id,
                request_id,
                define_id,
                sys::SIMCONNECT_CLIENT_DATA_PERIOD_SIMCONNECT_CLIENT_DATA_PERIOD_ON_SET,
                sys::SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_CHANGED,
                0,
                0,
                0,
            )
        })?;

        self.routes.insert(
            request_id,
            Box::new(ClientDataSubscriptionRoute::<T> { tx }),
        );
        self.active.insert(
            request_id,
            ActiveRequest::ClientData {
                client_id,
                define_id,
            },
        );
        self.remember_last_send_id(request_id);
        Ok(())
    }

    fn remember_last_send_id(&mut self, request_id: sys::SIMCONNECT_DATA_REQUEST_ID) {
        let mut send_id = 0;
        let result =
            unsafe { sys::SimConnect_GetLastSentPacketID(self.connection.handle, &mut send_id) };
        if result >= 0 {
            self.send_ids.insert(send_id, request_id);
        }
    }

    fn drain_dispatch(&mut self) -> Result<()> {
        loop {
            let mut recv = std::ptr::null_mut();
            let mut size = 0;
            check_hresult(unsafe {
                sys::SimConnect_GetNextDispatch(self.connection.handle, &mut recv, &mut size)
            })?;

            if recv.is_null()
                || unsafe { (*recv).dwID as sys::SIMCONNECT_RECV_ID }
                    == sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_NULL
            {
                return Ok(());
            }

            self.dispatch(recv, size as usize);
        }
    }

    fn dispatch(&mut self, recv: *const sys::SIMCONNECT_RECV, size: usize) {
        let id = unsafe { (*recv).dwID as sys::SIMCONNECT_RECV_ID };
        match id {
            sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_SIMOBJECT_DATA
            | sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_SIMOBJECT_DATA_BYTYPE => {
                let data = unsafe { &*recv.cast::<sys::SIMCONNECT_RECV_SIMOBJECT_DATA>() };
                let request_id = data.dwRequestID;
                let expected_define_id = self.active.get(&request_id).map(ActiveRequest::define_id);
                let remove = match (self.routes.get_mut(&request_id), expected_define_id) {
                    (Some(route), Some(define_id)) => route.deliver(data, size, define_id),
                    _ => false,
                };
                if remove {
                    self.remove_request(request_id);
                }
            }
            sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_CLIENT_DATA => {
                // SIMCONNECT_RECV_CLIENT_DATA derives from the same packet
                // layout and places its inherited base at offset zero.
                let data = unsafe { &*recv.cast::<sys::SIMCONNECT_RECV_SIMOBJECT_DATA>() };
                let request_id = data.dwRequestID;
                let expected_define_id = self.active.get(&request_id).map(ActiveRequest::define_id);
                let remove = match (self.routes.get_mut(&request_id), expected_define_id) {
                    (Some(route), Some(define_id)) => route.deliver(data, size, define_id),
                    _ => false,
                };
                if remove {
                    self.remove_request(request_id);
                }
            }
            sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_EXCEPTION => {
                let exception = unsafe { &*recv.cast::<sys::SIMCONNECT_RECV_EXCEPTION>() };
                let server_exception = ServerException {
                    code: exception.dwException as u32,
                    send_id: exception.dwSendID,
                    index: exception.dwIndex,
                };
                self.exception_sinks
                    .retain(|sink| sink.unbounded_send(server_exception.clone()).is_ok());
                let error = Error::SimConnectException {
                    code: server_exception.code,
                    send_id: server_exception.send_id,
                    index: server_exception.index,
                };
                if let Some(request_id) = self.send_ids.remove(&exception.dwSendID) {
                    if let Some(route) = self.routes.get_mut(&request_id) {
                        route.fail(error);
                    }
                    self.remove_request(request_id);
                }
            }
            sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_QUIT => {
                self.fail_all(Error::DriverStopped);
            }
            _ => {}
        }
    }

    fn cancel(&mut self, request_id: sys::SIMCONNECT_DATA_REQUEST_ID) {
        if let Some(active) = self.active.remove(&request_id) {
            match active {
                ActiveRequest::SimObject {
                    define_id,
                    object_id,
                    periodic: true,
                } => {
                    let _ = unsafe {
                        sys::SimConnect_RequestDataOnSimObject(
                            self.connection.handle,
                            request_id,
                            define_id,
                            object_id,
                            sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_NEVER,
                            sys::SIMCONNECT_DATA_REQUEST_FLAG_CHANGED,
                            0,
                            0,
                            0,
                        )
                    };
                }
                ActiveRequest::ClientData {
                    client_id,
                    define_id,
                } => {
                    let _ = unsafe {
                        sys::SimConnect_RequestClientData(
                            self.connection.handle,
                            client_id,
                            request_id,
                            define_id,
                            sys::SIMCONNECT_CLIENT_DATA_PERIOD_SIMCONNECT_CLIENT_DATA_PERIOD_NEVER,
                            sys::SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_CHANGED,
                            0,
                            0,
                            0,
                        )
                    };
                }
                ActiveRequest::SimObject { .. } => {}
            }
        }
        self.routes.remove(&request_id);
        self.send_ids.retain(|_, value| *value != request_id);
    }

    fn remove_request(&mut self, request_id: sys::SIMCONNECT_DATA_REQUEST_ID) {
        self.routes.remove(&request_id);
        self.active.remove(&request_id);
        self.send_ids.retain(|_, value| *value != request_id);
    }

    fn fail_all(&mut self, error: Error) {
        for route in self.routes.values_mut() {
            route.fail(error.clone());
        }
        self.routes.clear();
        self.active.clear();
        self.send_ids.clear();
    }
}

enum ActiveRequest {
    SimObject {
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        periodic: bool,
    },
    ClientData {
        client_id: sys::SIMCONNECT_CLIENT_DATA_ID,
        define_id: sys::SIMCONNECT_CLIENT_DATA_DEFINITION_ID,
    },
}

impl ActiveRequest {
    fn define_id(&self) -> u32 {
        match self {
            Self::SimObject { define_id, .. } => *define_id as u32,
            Self::ClientData { define_id, .. } => *define_id as u32,
        }
    }
}

trait Route: Send {
    /// Deliver a packet and return whether the route is complete.
    fn deliver(
        &mut self,
        data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
        size: usize,
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
    ) -> bool;

    fn fail(&mut self, error: Error);
}

struct OneShotRoute<T> {
    tx: Option<oneshot::Sender<Result<T>>>,
}

impl<T> Route for OneShotRoute<T>
where
    T: AsyncDataDefinition,
{
    fn deliver(
        &mut self,
        data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
        size: usize,
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
    ) -> bool {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(decode::<T>(data, size, define_id));
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
    tx: mpsc::Sender<Result<T>>,
}

struct MappedSubscriptionRoute<T, E, F> {
    tx: mpsc::Sender<Result<E>>,
    map: F,
    _item: PhantomData<T>,
}

impl<T, E, F> Route for MappedSubscriptionRoute<T, E, F>
where
    T: AsyncDataDefinition,
    E: Send + 'static,
    F: FnMut(T) -> E + Send + 'static,
{
    fn deliver(
        &mut self,
        data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
        size: usize,
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
    ) -> bool {
        let value = decode::<T>(data, size, define_id).map(|value| (self.map)(value));
        match self.tx.try_send(value) {
            Ok(()) => false,
            Err(error) if error.is_full() => false,
            Err(_) => true,
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.tx.try_send(Err(error));
    }
}

impl<T> Route for SubscriptionRoute<T>
where
    T: AsyncDataDefinition,
{
    fn deliver(
        &mut self,
        data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
        size: usize,
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
    ) -> bool {
        match self.tx.try_send(decode::<T>(data, size, define_id)) {
            Ok(()) => false,
            Err(error) if error.is_full() => false,
            Err(_) => true,
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.tx.try_send(Err(error));
        self.tx.close_channel();
    }
}

struct ClientDataSubscriptionRoute<T> {
    tx: mpsc::Sender<Result<T>>,
}

impl<T> Route for ClientDataSubscriptionRoute<T>
where
    T: AsyncClientDataDefinition,
{
    fn deliver(
        &mut self,
        data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
        size: usize,
        define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
    ) -> bool {
        let value = decode_client_data::<T>(data, size, define_id);
        match self.tx.try_send(value) {
            Ok(()) => false,
            Err(error) if error.is_full() => false,
            Err(_) => true,
        }
    }

    fn fail(&mut self, error: Error) {
        let _ = self.tx.try_send(Err(error));
        self.tx.close_channel();
    }
}

fn decode<T>(
    data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
    size: usize,
    expected_define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
) -> Result<T>
where
    T: AsyncDataDefinition,
{
    decode_packet::<T>(data, size, expected_define_id)
}

fn decode_client_data<T>(
    data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
    size: usize,
    expected_define_id: sys::SIMCONNECT_CLIENT_DATA_DEFINITION_ID,
) -> Result<T>
where
    T: AsyncClientDataDefinition,
{
    decode_packet::<T>(data, size, expected_define_id as _)
}

fn decode_packet<T>(
    data: &sys::SIMCONNECT_RECV_SIMOBJECT_DATA,
    size: usize,
    expected_define_id: sys::SIMCONNECT_DATA_DEFINITION_ID,
) -> Result<T>
where
    T: Copy,
{
    if data.dwDefineID != expected_define_id {
        return Err(Error::DefinitionMismatch {
            expected: expected_define_id as u32,
            actual: data.dwDefineID as u32,
        });
    }

    let base = std::ptr::from_ref(data).cast::<u8>() as usize;
    let value = std::ptr::addr_of!(data.dwData).cast::<u8>();
    let offset = value as usize - base;
    let expected = offset + std::mem::size_of::<T>();
    if size < expected {
        return Err(Error::InvalidPacket {
            expected,
            actual: size,
        });
    }

    // SAFETY: callers require the corresponding unsafe data-definition marker
    // trait. The bounds check covers the read, and read_unaligned handles
    // dwData's DWORD alignment.
    Ok(unsafe { value.cast::<T>().read_unaligned() })
}

fn recurring_period(period: Period) -> Result<sys::SIMCONNECT_PERIOD> {
    match period {
        Period::VisualFrame => Ok(sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_VISUAL_FRAME),
        Period::SimFrame => Ok(sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_SIM_FRAME),
        Period::Second => Ok(sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_SECOND),
        Period::Never | Period::Once => Err(Error::InvalidSubscriptionPeriod),
    }
}

struct RawConnection {
    handle: sys::HANDLE,
}

impl RawConnection {
    fn open(name: &CString, event: HANDLE) -> Result<Self> {
        let mut handle = unsafe { std::mem::zeroed() };
        check_hresult(unsafe {
            sys::SimConnect_Open(
                &mut handle,
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                event as _,
                0,
            )
        })?;
        Ok(Self { handle })
    }
}

impl Drop for RawConnection {
    fn drop(&mut self) {
        let _ = unsafe { sys::SimConnect_Close(self.handle) };
    }
}

struct OwnedEvent(HANDLE);

// SAFETY: Windows kernel event handles can be waited on from another thread.
unsafe impl Send for OwnedEvent {}

impl OwnedEvent {
    fn new() -> Result<Self> {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if handle.is_null() {
            Err(last_windows_error())
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }

    fn signal(&self) -> EventSignal {
        EventSignal(self.0)
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Clone, Copy)]
struct EventSignal(HANDLE);

// SAFETY: the driver owns the event while a live client can signal it, and
// Windows event handles are safe to signal from multiple threads.
unsafe impl Send for EventSignal {}
unsafe impl Sync for EventSignal {}

impl EventSignal {
    fn set(self) -> Result<()> {
        if unsafe { SetEvent(self.0) } == 0 {
            Err(last_windows_error())
        } else {
            Ok(())
        }
    }
}

fn check_hresult(result: sys::HRESULT) -> Result<()> {
    if result >= 0 {
        Ok(())
    } else {
        Err(Error::HResult(result as i32))
    }
}

fn last_windows_error() -> Error {
    Error::Windows(unsafe { GetLastError() })
}
