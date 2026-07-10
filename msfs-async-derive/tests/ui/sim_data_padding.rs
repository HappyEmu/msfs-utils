#![allow(non_camel_case_types, non_upper_case_globals)]

extern crate self as msfs_async;

use msfs_async_derive::data_definition;

pub mod __sys {
    pub type SIMCONNECT_DATATYPE = u32;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_INT32: u32 = 1;
    pub const SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_FLOAT64: u32 = 2;
}

pub trait DataDefinition: 'static {
    const DEFINITIONS: &'static [(&'static str, &'static str, f32, __sys::SIMCONNECT_DATATYPE)];
}

pub unsafe trait AsyncDataDefinition: DataDefinition + Copy + Send + 'static {}

pub mod __private {
    use std::marker::PhantomData;

    pub trait SimConnectDatum {}
    impl SimConnectDatum for i32 {}
    impl SimConnectDatum for f64 {}

    pub struct AssertSimConnectDatum<T: SimConnectDatum>(PhantomData<T>);
}

#[data_definition]
struct Invalid {
    #[name = "SIM ON GROUND"]
    #[unit = "Bool"]
    on_ground: i32,
    #[name = "RADIO HEIGHT"]
    #[unit = "Feet"]
    height: f64,
}

fn main() {}
