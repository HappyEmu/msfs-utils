#![allow(non_camel_case_types, non_upper_case_globals)]

extern crate self as msfs_async;
extern crate self as msfs_sync;

use msfs_async_derive::{
    client_data_definition, data_definition, sync_client_data_definition, sync_data_definition,
};

pub mod __sys {
    pub type SIMCONNECT_DATATYPE = u32;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_INT32: u32 = 1;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_INT64: u32 = 2;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_FLOAT32: u32 = 3;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_FLOAT64: u32 = 4;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_XYZ: u32 = 5;
}

pub trait DataDefinition: 'static {
    const DEFINITIONS: &'static [(&'static str, &'static str, f32, __sys::SIMCONNECT_DATATYPE)];
}

/// # Safety
///
/// Implementors must have a valid SimConnect wire representation.
pub unsafe trait AsyncDataDefinition: DataDefinition + Copy + Send + 'static {}

pub trait ClientDataDefinition: 'static {
    fn get_definitions() -> Vec<(usize, usize, f32)>;
}

/// # Safety
///
/// Implementors must have a valid client-data wire representation.
pub unsafe trait AsyncClientDataDefinition:
    ClientDataDefinition + Copy + Send + 'static
{
    fn async_definitions() -> Vec<(usize, usize, u32, f32)>;
}

pub mod __private {
    use std::marker::PhantomData;

    pub trait SimConnectDatum {}
    impl SimConnectDatum for f64 {}

    pub trait ClientDataDatum {}
    impl ClientDataDatum for u8 {}
    impl ClientDataDatum for u32 {}

    pub struct AssertSimConnectDatum<T: SimConnectDatum>(PhantomData<T>);
    pub struct AssertClientDataDatum<T: ClientDataDatum>(PhantomData<T>);
}

#[data_definition]
#[derive(Debug)]
struct AircraftData {
    #[name = "RADIO HEIGHT"]
    #[unit = "Feet"]
    #[epsilon = 1]
    height: f64,
}

#[client_data_definition]
#[derive(Debug)]
struct ClientData {
    counter: u32,
    state: u8,
}

#[client_data_definition]
#[derive(Debug)]
struct PaddedClientData {
    byte: u8,
    word: u32,
}

#[sync_data_definition]
#[derive(Debug)]
struct SyncAircraftData {
    #[name = "AIRSPEED INDICATED"]
    #[unit = "Knots"]
    airspeed: f64,
}

#[sync_client_data_definition]
#[derive(Debug)]
struct SyncClientData {
    counter: u32,
}

fn assert_aircraft_data<T: AsyncDataDefinition>() {}
fn assert_client_data<T: AsyncClientDataDefinition>() {}

#[test]
fn generated_types_implement_the_safe_api_contracts() {
    assert_aircraft_data::<AircraftData>();
    assert_client_data::<ClientData>();
    assert_client_data::<PaddedClientData>();
    assert_aircraft_data::<SyncAircraftData>();
    assert_client_data::<SyncClientData>();

    let value = AircraftData { height: 42.0 };
    let copied = value;
    assert_eq!(copied.height, 42.0);
    assert_eq!(AircraftData::DEFINITIONS.len(), 1);

    let definitions = ClientData::get_definitions();
    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0].0, std::mem::offset_of!(ClientData, counter));
    assert_eq!(definitions[1].0, std::mem::offset_of!(ClientData, state));
    let async_definitions = ClientData::async_definitions();
    assert_eq!(async_definitions[0].2, (-3_i32) as u32);
    assert_eq!(async_definitions[0].3, 0.0);
    assert_eq!(async_definitions[1].2, (-1_i32) as u32);

    let padded = PaddedClientData::get_definitions();
    assert_eq!(padded[0].0, 0);
    assert_eq!(padded[1].0, std::mem::offset_of!(PaddedClientData, word));
    assert_eq!(padded[1].1, std::mem::size_of::<u32>());
}
