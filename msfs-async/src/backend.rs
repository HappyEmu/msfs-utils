#![cfg_attr(not(windows), allow(dead_code))]

use crate::{InitialPosition, Result};
use std::ffi::CStr;

pub(crate) type RequestId = u32;
pub(crate) type DefinitionId = u32;
pub(crate) type ClientDataId = u32;
pub(crate) type ClientDefinitionId = u32;
pub(crate) type ObjectId = u32;
pub(crate) type EventId = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DataPeriod {
    Never,
    Once,
    VisualFrame,
    SimFrame,
    Second,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClientDataPeriod {
    Never,
    OnSet,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DataRequest {
    pub request_id: RequestId,
    pub define_id: DefinitionId,
    pub object_id: ObjectId,
    pub period: DataPeriod,
    pub changed_only: bool,
    pub origin: u32,
    pub interval: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientDataRequest {
    pub client_id: ClientDataId,
    pub request_id: RequestId,
    pub define_id: ClientDefinitionId,
    pub period: ClientDataPeriod,
    pub changed_only: bool,
    pub origin: u32,
    pub interval: u32,
    pub limit: u32,
}

/// Boundary around every raw SimConnect operation used by the driver.
///
/// Implementations turn synchronous SDK failures into [`crate::Error`] and
/// copy dispatch packets into caller-owned reusable storage. Consequently the
/// state machine contains no FFI calls and never borrows memory owned by
/// SimConnect.
pub(crate) trait SimConnectBackend: Send + 'static {
    type OpenContext;

    const RECV_ID_NULL: u32;
    const RECV_ID_EXCEPTION: u32;
    const RECV_ID_QUIT: u32;
    const RECV_ID_SIMOBJECT_DATA: u32;
    const RECV_ID_SIMOBJECT_DATA_BYTYPE: u32;
    const RECV_ID_CLIENT_DATA: u32;
    const RECV_ID_ASSIGNED_OBJECT_ID: u32;

    fn open(name: &CStr, context: Self::OpenContext) -> Result<Self>
    where
        Self: Sized;

    fn set_data_on_sim_object(
        &mut self,
        define_id: DefinitionId,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<()>;

    fn release_ai_control(&mut self, object_id: ObjectId, request_id: RequestId) -> Result<()>;

    fn create_non_atc_aircraft(
        &mut self,
        container_title: &CStr,
        tail_number: &CStr,
        initial_position: InitialPosition,
        request_id: RequestId,
    ) -> Result<()>;

    fn remove_object(&mut self, object_id: ObjectId, request_id: RequestId) -> Result<()>;

    fn transmit_client_event(
        &mut self,
        object_id: ObjectId,
        event_id: EventId,
        data: u32,
    ) -> Result<()>;

    fn map_client_event(&mut self, event_id: EventId, name: &CStr) -> Result<()>;

    fn request_data(&mut self, request: DataRequest) -> Result<()>;

    fn add_data_definition(
        &mut self,
        define_id: DefinitionId,
        datum_name: &CStr,
        units: &CStr,
        datatype: u32,
        epsilon: f32,
    ) -> Result<()>;

    fn clear_data_definition(&mut self, define_id: DefinitionId) -> Result<()>;

    fn create_client_data(&mut self, client_id: ClientDataId, size: u32) -> Result<()>;

    fn map_client_data_name(&mut self, name: &CStr, client_id: ClientDataId) -> Result<()>;

    fn add_client_data_definition(
        &mut self,
        define_id: ClientDefinitionId,
        offset: u32,
        size_or_type: u32,
        epsilon: f32,
    ) -> Result<()>;

    fn clear_client_data_definition(&mut self, define_id: ClientDefinitionId) -> Result<()>;

    fn set_client_data(
        &mut self,
        client_id: ClientDataId,
        define_id: ClientDefinitionId,
        data: &[u8],
    ) -> Result<()>;

    fn request_client_data(&mut self, request: ClientDataRequest) -> Result<()>;

    fn last_send_id(&mut self) -> Result<u32>;

    /// Copy the next packet into a reusable buffer.
    ///
    /// Returns `false` when the native dispatch queue is empty. The driver
    /// guarantees that `packet` is empty while preserving its allocated
    /// capacity, so implementations only need to append the new packet bytes.
    fn next_dispatch(&mut self, packet: &mut Vec<u8>) -> Result<bool>;
}
