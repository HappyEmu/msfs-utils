use crate::backend::{DataPeriod, SimConnectBackend};
use crate::driver::{
    ClientData, DataDefinitionEntry, DeliverySender, Driver, MappedError, MappedTarget,
    MappedThunk, RequestSettings, SimData,
};
use crate::ids::RequestIdAllocator;
use crate::latest::{Receiver as LatestReceiver, channel as latest_channel};
use crate::native::NativeBackend;
use crate::{
    AiAircraft, AsyncClientDataDefinition, AsyncDataDefinition, Error, FreezeState,
    InitialPosition, OverflowPolicy, RecurringPeriod, Result, ServerException, SubscriptionOptions,
};
use futures_channel::{mpsc, oneshot};
use futures_core::Stream;
use msfs::sys;
use std::ffi::CString;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::task::{Context, Poll};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};

const DEFAULT_SUBSCRIPTION_CAPACITY: usize = 32;

enum SubscriptionItems<T> {
    Buffered(mpsc::Receiver<T>),
    Latest(LatestReceiver<T>),
}

impl<T> SubscriptionItems<T> {
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        match self {
            Self::Buffered(receiver) => Pin::new(receiver).poll_next(cx),
            Self::Latest(receiver) => receiver.poll_next(cx),
        }
    }
}

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
            request_ids: RequestIdAllocator::new(),
            thread: Mutex::new(None),
        });

        let thread = std::thread::Builder::new()
            .name("msfs-simconnect".to_owned())
            .spawn(move || {
                run_driver(name, sim_event, command_event, command_rx, open_tx);
            })
            .map_err(|error| Error::ThreadStart(error.to_string()))?;
        *inner
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(thread);

        open_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Self { inner })
    }

    /// Close the shared native connection and wait for the driver to exit.
    ///
    /// This stops the session for every clone of this handle; subsequent
    /// operations on those clones return [`Error::DriverStopped`].
    pub async fn close(self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Shutdown(Some(tx)))?;
        rx.await.map_err(|_| Error::DriverStopped)?;
        self.inner.join_thread()
    }

    /// Request one owned value from the user's aircraft or another simulation object.
    pub async fn request_once<T>(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<T>
    where
        T: AsyncDataDefinition,
    {
        let request_id = self.next_request_id()?;
        let (tx, rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.request_once::<T>(request_id, object_id, tx);
        })))?;

        let mut cancellation = CancellationGuard::new(request_id, Arc::clone(&self.inner));
        let result = rx.await.map_err(|_| Error::DriverStopped)?;
        cancellation.disarm();
        result
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

    /// Create a non-ATC AI aircraft and wait for its assigned object ID.
    ///
    /// `container_title` must identify an installed aircraft model. If this
    /// future is dropped after SimConnect accepts the request, the driver waits
    /// for the assignment and removes the newly created orphan automatically.
    pub async fn create_non_atc_aircraft(
        &self,
        container_title: impl Into<String>,
        tail_number: impl Into<String>,
        initial_position: InitialPosition,
    ) -> Result<AiAircraft> {
        let container_title =
            CString::new(container_title.into()).map_err(|_| Error::InvalidCString)?;
        let tail_number = CString::new(tail_number.into()).map_err(|_| Error::InvalidCString)?;
        let request_id = self.next_request_id()?;
        let (tx, rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.create_non_atc_aircraft(
                request_id,
                &container_title,
                &tail_number,
                initial_position,
                tx,
            );
        })))?;

        let mut creation = CreationGuard::new(request_id, Arc::clone(&self.inner));
        let result = rx.await.map_err(|_| Error::DriverStopped)?;
        match result {
            Ok(aircraft) => {
                self.send(Command::CompleteObjectCreation(request_id))?;
                creation.disarm();
                Ok(aircraft)
            }
            Err(error) => {
                creation.disarm();
                Err(error)
            }
        }
    }

    /// Remove an AI aircraft or other client-created simulation object.
    ///
    /// Successful completion means SimConnect accepted the local API call.
    /// Delayed server-side failures arrive through [`Self::exceptions`].
    pub async fn remove_object(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<()> {
        let request_id = self.next_request_id()?;
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.remove_object(object_id, request_id));
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
        let request_id = self.next_request_id()?;
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

    /// Create a stream which combines several differently typed subscriptions.
    pub fn event_stream<E>(&self) -> EventStream<E>
    where
        E: Send + 'static,
    {
        self.event_stream_with_capacity(DEFAULT_SUBSCRIPTION_CAPACITY)
    }

    /// Create a combined event stream with an explicit shared buffer capacity.
    pub fn event_stream_with_capacity<E>(&self, capacity: usize) -> EventStream<E>
    where
        E: Send + 'static,
    {
        let (sender, stream) = mapped_event_channel(capacity);
        EventStream {
            subscriptions: Vec::new(),
            client: self.clone(),
            sender,
            stream,
        }
    }

    /// Subscribe to typed simulation-object data using a bounded buffer.
    ///
    /// If the consumer falls behind, new values are discarded until capacity
    /// becomes available. Use [`Self::subscribe_with_capacity`] to tune the
    /// buffer for high-frequency data.
    pub async fn subscribe<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        self.subscribe_with_options(object_id, SubscriptionOptions::new(period))
            .await
    }

    /// Subscribe with an explicit bounded channel capacity.
    pub async fn subscribe_with_capacity<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
        capacity: usize,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        let mut options = SubscriptionOptions::new(period);
        options.capacity = capacity;
        self.subscribe_with_options(object_id, options).await
    }

    /// Subscribe with explicit cadence, buffering, and native request options.
    pub async fn subscribe_with_options<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        options: SubscriptionOptions,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        let request_id = self.next_request_id()?;
        let (items_tx, items) = match options.overflow {
            OverflowPolicy::DropNewest => {
                let (tx, rx) = mpsc::channel(options.capacity);
                (
                    DeliverySender::Buffered(tx),
                    SubscriptionItems::Buffered(rx),
                )
            }
            OverflowPolicy::Latest => {
                let (tx, rx) = latest_channel();
                (DeliverySender::Latest(tx), SubscriptionItems::Latest(rx))
            }
        };
        let (errors_tx, errors_rx) = mpsc::unbounded();
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe::<T>(
                request_id,
                object_id,
                RequestSettings::from(options),
                items_tx,
                errors_tx,
                ready_tx,
            );
        })))?;

        ready_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Subscription {
            request_id,
            items,
            errors: errors_rx,
            terminated: false,
            inner: Arc::clone(&self.inner),
        })
    }

    /// Register a typed route which maps its values into a shared event stream.
    #[doc(hidden)]
    pub async fn subscribe_mapped<T, E, F>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
        events: MappedEventSender<E>,
        map: F,
    ) -> Result<MappedSubscription>
    where
        T: AsyncDataDefinition,
        E: Send + 'static,
        F: FnMut(T) -> E + Send + 'static,
    {
        let period = recurring_period(period);
        let request_id = self.next_request_id()?;
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe_mapped::<T, E, F>(
                request_id,
                object_id,
                period,
                MappedTarget {
                    tx: events.items,
                    errors: events.errors,
                    map,
                },
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
        let area_name = name.clone();
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.create_client_data::<T>(&name));
        })))?;

        let client_id = rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(ClientDataArea {
            client_id,
            name: area_name,
            inner: Arc::clone(&self.inner),
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
        let area_name = name.clone();
        let (tx, rx) = oneshot::channel();
        self.send(Command::Run(Box::new(move |driver| {
            let _ = tx.send(driver.client_data_id(&name));
        })))?;

        let client_id = rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(ClientDataArea {
            client_id,
            name: area_name,
            inner: Arc::clone(&self.inner),
            _item: PhantomData,
        })
    }

    /// Write a complete value into a client-data area.
    pub async fn set_client_data<T>(&self, area: &ClientDataArea<T>, data: &T) -> Result<()>
    where
        T: AsyncClientDataDefinition,
    {
        if !Arc::ptr_eq(&self.inner, &area.inner) {
            return Err(Error::ClientDataSessionMismatch);
        }
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
        let request_id = self.next_request_id()?;
        let name = name.into();
        let (items_tx, items_rx) = mpsc::channel(DEFAULT_SUBSCRIPTION_CAPACITY);
        let (errors_tx, errors_rx) = mpsc::unbounded();
        let (ready_tx, ready_rx) = oneshot::channel();

        self.send(Command::Run(Box::new(move |driver| {
            driver.subscribe_client_data::<T>(request_id, &name, items_tx, errors_tx, ready_tx);
        })))?;

        ready_rx.await.map_err(|_| Error::DriverStopped)??;
        Ok(Subscription {
            request_id,
            items: SubscriptionItems::Buffered(items_rx),
            errors: errors_rx,
            terminated: false,
            inner: Arc::clone(&self.inner),
        })
    }

    fn next_request_id(&self) -> Result<sys::SIMCONNECT_DATA_REQUEST_ID> {
        self.inner.request_ids.next()
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
#[derive(Clone)]
pub struct ClientDataArea<T> {
    client_id: sys::SIMCONNECT_CLIENT_DATA_ID,
    name: String,
    inner: Arc<ClientInner>,
    _item: PhantomData<T>,
}

impl<T> std::fmt::Debug for ClientDataArea<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientDataArea")
            .field("client_id", &self.client_id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T> ClientDataArea<T>
where
    T: AsyncClientDataDefinition,
{
    /// Replace the contents of this client-data area.
    pub async fn set(&self, data: &T) -> Result<()> {
        AsyncSimConnect {
            inner: Arc::clone(&self.inner),
        }
        .set_client_data(self, data)
        .await
    }

    /// Subscribe to changes in this client-data area.
    pub async fn subscribe(&self) -> Result<Subscription<T>> {
        AsyncSimConnect {
            inner: Arc::clone(&self.inner),
        }
        .subscribe_client_data(self.name.clone())
        .await
    }
}

/// A heterogeneous stream assembled from several typed SimConnect subscriptions.
///
/// Mapping functions execute when this stream is polled, never on the native
/// driver thread. A route-specific failure is yielded as an error item without
/// terminating the other subscriptions in the stream.
pub struct EventStream<E> {
    subscriptions: Vec<MappedSubscription>,
    client: AsyncSimConnect,
    sender: MappedEventSender<E>,
    stream: MappedEventStream<E>,
}

impl<E> EventStream<E>
where
    E: Send + 'static,
{
    /// Add a typed subscription and map its values into the shared event type.
    pub async fn subscribe<T, F>(
        &mut self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
        map: F,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
        F: FnMut(T) -> E + Send + 'static,
    {
        let subscription = self
            .client
            .subscribe_mapped(object_id, period, self.sender.clone(), map)
            .await?;
        self.subscriptions.push(subscription);
        Ok(())
    }
}

impl<E> Unpin for EventStream<E> {}

impl<E> Stream for EventStream<E>
where
    E: Send + 'static,
{
    type Item = Result<E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().stream).poll_next(cx)
    }
}

/// Create the shared channel used by mapped subscriptions.
#[doc(hidden)]
pub fn mapped_event_channel<E>(capacity: usize) -> (MappedEventSender<E>, MappedEventStream<E>) {
    let (tx, rx) = mpsc::channel(capacity);
    let (errors_tx, errors_rx) = mpsc::unbounded();
    (
        MappedEventSender {
            items: tx,
            errors: errors_tx,
        },
        MappedEventStream {
            items: rx,
            errors: errors_rx,
            terminated: false,
        },
    )
}

/// Sending half of a shared mapped-subscription channel.
#[doc(hidden)]
pub struct MappedEventSender<E> {
    items: mpsc::Sender<MappedThunk<E>>,
    errors: mpsc::UnboundedSender<MappedError>,
}

impl<E> Clone for MappedEventSender<E> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            errors: self.errors.clone(),
        }
    }
}

/// Stream receiving values from several mapped subscriptions.
#[doc(hidden)]
pub struct MappedEventStream<E> {
    items: mpsc::Receiver<MappedThunk<E>>,
    errors: mpsc::UnboundedReceiver<MappedError>,
    terminated: bool,
}

impl<E> Unpin for MappedEventStream<E> {}

impl<E> Stream for MappedEventStream<E> {
    type Item = Result<E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.terminated {
            return Poll::Ready(None);
        }
        if let Poll::Ready(Some(error)) = Pin::new(&mut this.errors).poll_next(cx) {
            return match error {
                MappedError::Route(error) => Poll::Ready(Some(Err(error))),
                MappedError::Session(error) => {
                    this.terminated = true;
                    Poll::Ready(Some(Err(error)))
                }
            };
        }
        Pin::new(&mut this.items)
            .poll_next(cx)
            .map(|item| item.map(|map| Ok(map())))
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
        send_cancel(&self.inner, self.request_id);
    }
}

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
///
/// If the route fails, values already in its data buffer are yielded before
/// the terminal error.
pub struct Subscription<T> {
    request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    items: SubscriptionItems<T>,
    errors: mpsc::UnboundedReceiver<Error>,
    terminated: bool,
    inner: Arc<ClientInner>,
}

impl<T> Unpin for Subscription<T> {}

impl<T> Stream for Subscription<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.terminated {
            return Poll::Ready(None);
        }
        match this.items.poll_next(cx) {
            Poll::Ready(Some(item)) => return Poll::Ready(Some(Ok(item))),
            Poll::Ready(None) => {}
            Poll::Pending => {
                return match Pin::new(&mut this.errors).poll_next(cx) {
                    Poll::Ready(Some(error)) => {
                        this.terminated = true;
                        Poll::Ready(Some(Err(error)))
                    }
                    Poll::Ready(None) | Poll::Pending => Poll::Pending,
                };
            }
        }
        match Pin::new(&mut this.errors).poll_next(cx) {
            Poll::Ready(Some(error)) => {
                this.terminated = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.terminated = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        send_cancel(&self.inner, self.request_id);
    }
}

struct CancellationGuard {
    request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    inner: Arc<ClientInner>,
    armed: bool,
}

struct CreationGuard {
    request_id: sys::SIMCONNECT_DATA_REQUEST_ID,
    inner: Arc<ClientInner>,
    armed: bool,
}

impl CreationGuard {
    fn new(request_id: sys::SIMCONNECT_DATA_REQUEST_ID, inner: Arc<ClientInner>) -> Self {
        Self {
            request_id,
            inner,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CreationGuard {
    fn drop(&mut self) {
        if self.armed {
            send_command(&self.inner, Command::AbandonObjectCreation(self.request_id));
        }
    }
}

impl CancellationGuard {
    fn new(request_id: sys::SIMCONNECT_DATA_REQUEST_ID, inner: Arc<ClientInner>) -> Self {
        Self {
            request_id,
            inner,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            send_cancel(&self.inner, self.request_id);
        }
    }
}

fn send_cancel(inner: &ClientInner, request_id: sys::SIMCONNECT_DATA_REQUEST_ID) {
    send_command(inner, Command::Cancel(request_id));
}

fn send_command(inner: &ClientInner, command: Command) {
    if inner.commands.send(command).is_ok() {
        let _ = inner.command_event.set();
    }
}

struct ClientInner {
    commands: std_mpsc::Sender<Command>,
    command_event: EventSignal,
    request_ids: RequestIdAllocator,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ClientInner {
    fn join_thread(&self) -> Result<()> {
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        thread.map_or(Ok(()), |thread| {
            thread.join().map_err(|_| Error::DriverStopped)
        })
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown(None));
        let _ = self.command_event.set();
    }
}

enum Command {
    Run(Box<dyn FnOnce(&mut Driver<NativeBackend>) + Send + 'static>),
    Cancel(sys::SIMCONNECT_DATA_REQUEST_ID),
    CompleteObjectCreation(sys::SIMCONNECT_DATA_REQUEST_ID),
    AbandonObjectCreation(sys::SIMCONNECT_DATA_REQUEST_ID),
    Shutdown(Option<oneshot::Sender<()>>),
}

enum DriverControl {
    Continue,
    Shutdown(Option<oneshot::Sender<()>>),
}

impl<T: AsyncDataDefinition> SimData for T {
    fn definitions() -> Result<Vec<DataDefinitionEntry>> {
        T::DEFINITIONS
            .iter()
            .map(|(name, units, epsilon, datatype)| {
                Ok(DataDefinitionEntry {
                    name: CString::new(*name).map_err(|_| Error::InvalidCString)?,
                    units: CString::new(*units).map_err(|_| Error::InvalidCString)?,
                    datatype: *datatype as u32,
                    epsilon: *epsilon,
                })
            })
            .collect()
    }
}

impl<T: AsyncClientDataDefinition> ClientData for T {
    fn definitions() -> Vec<crate::client_layout::FieldDefinition> {
        T::async_definitions()
    }
}

impl From<SubscriptionOptions> for RequestSettings {
    fn from(options: SubscriptionOptions) -> Self {
        Self {
            period: recurring_period(options.period),
            changed_only: options.changed_only,
            origin: options.origin,
            interval: options.interval,
            limit: options.limit,
        }
    }
}

fn recurring_period(period: RecurringPeriod) -> DataPeriod {
    match period {
        RecurringPeriod::VisualFrame => DataPeriod::VisualFrame,
        RecurringPeriod::SimFrame => DataPeriod::SimFrame,
        RecurringPeriod::Second => DataPeriod::Second,
    }
}

fn run_driver(
    name: CString,
    sim_event: OwnedEvent,
    command_event: OwnedEvent,
    commands: std_mpsc::Receiver<Command>,
    opened: oneshot::Sender<Result<()>>,
) {
    let backend = match NativeBackend::open(&name, sim_event.raw()) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    let mut driver = Driver::new(backend);
    let _ = opened.send(Ok(()));

    let handles = [sim_event.raw(), command_event.raw()];
    let mut shutdown_ack = None;
    let outcome = loop {
        let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        let control = match wait {
            WAIT_OBJECT_0 => match driver.drain_dispatch() {
                Ok(true) => drain_commands(&mut driver, &commands),
                Ok(false) => break Err(Error::DriverStopped),
                Err(error) => break Err(error),
            },
            value if value == WAIT_OBJECT_0 + 1 => drain_commands(&mut driver, &commands),
            WAIT_FAILED => break Err(last_windows_error()),
            _ => break Err(last_windows_error()),
        };
        match control {
            DriverControl::Continue => {}
            DriverControl::Shutdown(ack) => {
                shutdown_ack = ack;
                break Ok(());
            }
        }
    };

    driver.fail_all(outcome.err().unwrap_or(Error::DriverStopped));
    drop(driver);
    if let Some(ack) = shutdown_ack {
        let _ = ack.send(());
    }
}

fn drain_commands(
    driver: &mut Driver<NativeBackend>,
    commands: &std_mpsc::Receiver<Command>,
) -> DriverControl {
    loop {
        match commands.try_recv() {
            Ok(Command::Run(command)) => command(driver),
            Ok(Command::Cancel(request_id)) => driver.cancel(request_id),
            Ok(Command::CompleteObjectCreation(request_id)) => {
                driver.complete_object_creation(request_id)
            }
            Ok(Command::AbandonObjectCreation(request_id)) => {
                driver.abandon_object_creation(request_id)
            }
            Ok(Command::Shutdown(ack)) => return DriverControl::Shutdown(ack),
            Err(std_mpsc::TryRecvError::Disconnected) => return DriverControl::Shutdown(None),
            Err(std_mpsc::TryRecvError::Empty) => return DriverControl::Continue,
        }
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

fn last_windows_error() -> Error {
    Error::Windows(unsafe { GetLastError() })
}
