//! Blocking facade for the event-driven `msfs-async` SimConnect driver.
//!
//! The native driver still owns the SimConnect handle on its Windows event
//! thread. This crate blocks only at the public API boundary.

extern crate self as msfs_sync;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{EventReceiver, ExceptionReceiver, SimConnect, Subscription};

#[cfg(windows)]
#[doc(hidden)]
pub use msfs_async::{__private, __sys};
#[cfg(windows)]
pub use msfs_async::{
    AsyncClientDataDefinition, AsyncDataDefinition, ClientDataArea, ClientDataDefinition,
    DataDefinition, DataXYZ, Error, FreezeState, OverflowPolicy, Period, RecurringPeriod, Result,
    SIMCONNECT_OBJECT_ID_USER, ServerException, SubscriptionOptions,
};
#[cfg(windows)]
pub use msfs_async_derive::{
    sync_client_data_definition as client_data_definition, sync_data_definition as data_definition,
};
