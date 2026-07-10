use futures_util::{FutureExt, StreamExt};
use msfs_async::__sys as sys;
use msfs_async::{
    AiAircraft, AsyncClientDataDefinition, AsyncDataDefinition, AsyncSimConnect, ClientDataArea,
    Error, ExceptionStream as AsyncExceptionStream, FreezeState, InitialPosition,
    MappedEventSender, MappedEventStream, MappedSubscription, RecurringPeriod, Result,
    ServerException, Subscription as AsyncSubscription, SubscriptionOptions, mapped_event_channel,
};

/// A cloneable blocking handle to one SimConnect session.
#[derive(Clone)]
pub struct SimConnect {
    inner: AsyncSimConnect,
}

impl SimConnect {
    /// Open a SimConnect session and its event-driven driver thread.
    pub fn open(name: impl Into<String>) -> Result<Self> {
        futures_executor::block_on(AsyncSimConnect::open(name)).map(|inner| Self { inner })
    }

    /// Close the shared native connection and wait for the driver to exit.
    pub fn close(self) -> Result<()> {
        futures_executor::block_on(self.inner.close())
    }

    /// Block until one typed value has been returned by SimConnect.
    pub fn request_once<T>(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<T>
    where
        T: AsyncDataDefinition,
    {
        futures_executor::block_on(self.inner.request_once::<T>(object_id))
    }

    /// Submit a complete typed update for a simulation object.
    pub fn set_data_on_sim_object<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        data: &T,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
    {
        futures_executor::block_on(self.inner.set_data_on_sim_object(object_id, data))
    }

    /// Create a non-ATC AI aircraft and block until SimConnect assigns its object ID.
    pub fn create_non_atc_aircraft(
        &self,
        container_title: impl Into<String>,
        tail_number: impl Into<String>,
        initial_position: InitialPosition,
    ) -> Result<AiAircraft> {
        futures_executor::block_on(self.inner.create_non_atc_aircraft(
            container_title,
            tail_number,
            initial_position,
        ))
    }

    /// Remove an AI aircraft or other client-created simulation object.
    pub fn remove_object(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<()> {
        futures_executor::block_on(self.inner.remove_object(object_id))
    }

    /// Release an object from the simulator's AI controller.
    pub fn release_ai_control(&self, object_id: sys::SIMCONNECT_OBJECT_ID) -> Result<()> {
        futures_executor::block_on(self.inner.release_ai_control(object_id))
    }

    /// Set which position and attitude components the simulator should freeze.
    pub fn set_freeze(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        state: FreezeState,
    ) -> Result<()> {
        futures_executor::block_on(self.inner.set_freeze(object_id, state))
    }

    /// Create a typed recurring data receiver.
    pub fn subscribe<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        futures_executor::block_on(self.inner.subscribe(object_id, period)).map(Subscription::new)
    }

    /// Create a typed recurring receiver with an explicit bounded capacity.
    pub fn subscribe_with_capacity<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
        capacity: usize,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        futures_executor::block_on(
            self.inner
                .subscribe_with_capacity(object_id, period, capacity),
        )
        .map(Subscription::new)
    }

    /// Create a typed receiver with explicit buffering and native request options.
    pub fn subscribe_with_options<T>(
        &self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        options: SubscriptionOptions,
    ) -> Result<Subscription<T>>
    where
        T: AsyncDataDefinition,
    {
        futures_executor::block_on(self.inner.subscribe_with_options(object_id, options))
            .map(Subscription::new)
    }

    /// Create a receiver which combines several typed subscriptions.
    pub fn event_receiver<E>(&self) -> EventReceiver<E>
    where
        E: Send + 'static,
    {
        self.event_receiver_with_capacity(32)
    }

    /// Create a combined receiver with an explicit shared buffer capacity.
    pub fn event_receiver_with_capacity<E>(&self, capacity: usize) -> EventReceiver<E>
    where
        E: Send + 'static,
    {
        let (sender, stream) = mapped_event_channel(capacity);
        EventReceiver {
            subscriptions: Vec::new(),
            client: self.inner.clone(),
            sender,
            stream,
        }
    }

    /// Receive delayed server-side exceptions for this session.
    pub fn exceptions(&self) -> Result<ExceptionReceiver> {
        self.inner
            .exceptions()
            .map(|inner| ExceptionReceiver { inner })
    }

    /// Allocate a named typed client-data area.
    pub fn create_client_data<T>(&self, name: impl Into<String>) -> Result<ClientDataArea<T>>
    where
        T: AsyncClientDataDefinition,
    {
        futures_executor::block_on(self.inner.create_client_data(name))
    }

    /// Open a named client-data area created by another client.
    pub fn get_client_data_area<T>(&self, name: impl Into<String>) -> Result<ClientDataArea<T>>
    where
        T: AsyncClientDataDefinition,
    {
        futures_executor::block_on(self.inner.get_client_data_area(name))
    }

    /// Replace the contents of a typed client-data area.
    pub fn set_client_data<T>(&self, area: &ClientDataArea<T>, data: &T) -> Result<()>
    where
        T: AsyncClientDataDefinition,
    {
        futures_executor::block_on(self.inner.set_client_data(area, data))
    }

    /// Subscribe to changes in a named client-data area.
    pub fn subscribe_client_data<T>(&self, name: impl Into<String>) -> Result<Subscription<T>>
    where
        T: AsyncClientDataDefinition,
    {
        futures_executor::block_on(self.inner.subscribe_client_data(name)).map(Subscription::new)
    }
}

/// A blocking fan-in receiver for several differently typed subscriptions.
pub struct EventReceiver<E> {
    subscriptions: Vec<MappedSubscription>,
    client: AsyncSimConnect,
    sender: MappedEventSender<E>,
    stream: MappedEventStream<E>,
}

impl<E> EventReceiver<E>
where
    E: Send + 'static,
{
    /// Map a typed SimConnect subscription into this receiver's event type.
    pub fn subscribe<T, F>(
        &mut self,
        object_id: sys::SIMCONNECT_OBJECT_ID,
        period: RecurringPeriod,
        map: F,
    ) -> Result<()>
    where
        T: AsyncDataDefinition,
        F: FnMut(T) -> E + Send + 'static,
    {
        let subscription = futures_executor::block_on(self.client.subscribe_mapped::<T, E, F>(
            object_id,
            period,
            self.sender.clone(),
            map,
        ))?;
        self.subscriptions.push(subscription);
        Ok(())
    }

    /// Block until any registered subscription produces a value.
    pub fn recv(&mut self) -> Result<E> {
        futures_executor::block_on(self.stream.next()).unwrap_or(Err(Error::DriverStopped))
    }

    /// Return an immediately available event without blocking.
    pub fn try_recv(&mut self) -> Result<Option<E>> {
        match self.stream.next().now_or_never() {
            None => Ok(None),
            Some(None) => Err(Error::DriverStopped),
            Some(Some(Ok(event))) => Ok(Some(event)),
            Some(Some(Err(error))) => Err(error),
        }
    }
}

impl<E> Iterator for EventReceiver<E>
where
    E: Send + 'static,
{
    type Item = Result<E>;

    fn next(&mut self) -> Option<Self::Item> {
        futures_executor::block_on(self.stream.next())
    }
}

/// A blocking typed receiver for recurring SimConnect data.
pub struct Subscription<T> {
    inner: AsyncSubscription<T>,
}

impl<T> Subscription<T> {
    fn new(inner: AsyncSubscription<T>) -> Self {
        Self { inner }
    }

    /// Block until the next value or stream error is available.
    pub fn recv(&mut self) -> Result<T> {
        futures_executor::block_on(self.inner.next()).unwrap_or(Err(Error::DriverStopped))
    }

    /// Return the next immediately available value without blocking.
    pub fn try_recv(&mut self) -> Result<Option<T>> {
        match self.inner.next().now_or_never() {
            None => Ok(None),
            Some(None) => Err(Error::DriverStopped),
            Some(Some(Ok(value))) => Ok(Some(value)),
            Some(Some(Err(error))) => Err(error),
        }
    }
}

impl<T> Iterator for Subscription<T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        futures_executor::block_on(self.inner.next())
    }
}

/// A blocking receiver for delayed server-side exceptions.
pub struct ExceptionReceiver {
    inner: AsyncExceptionStream,
}

impl ExceptionReceiver {
    /// Block until the next server-side exception is available.
    pub fn recv(&mut self) -> Option<ServerException> {
        futures_executor::block_on(self.inner.next())
    }

    /// Return the next immediately available exception without blocking.
    pub fn try_recv(&mut self) -> Option<ServerException> {
        self.inner.next().now_or_never().flatten()
    }
}

impl Iterator for ExceptionReceiver {
    type Item = ServerException;

    fn next(&mut self) -> Option<Self::Item> {
        self.recv()
    }
}
