use crate::backend::{
    ClientDataPeriod, ClientDataRequest, DataPeriod, DataRequest, SimConnectBackend,
};
use crate::{Error, Result};
use msfs::sys;
use std::ffi::CStr;
use windows_sys::Win32::Foundation::HANDLE;

pub(crate) struct NativeBackend {
    handle: sys::HANDLE,
}

const _: () = {
    assert!(std::mem::size_of::<sys::SIMCONNECT_RECV>() == 12);
    assert!(std::mem::offset_of!(sys::SIMCONNECT_RECV_SIMOBJECT_DATA, dwData) == 40);
    assert!(std::mem::size_of::<sys::SIMCONNECT_RECV_EXCEPTION>() == 24);
};

// SAFETY: the backend is created, used, and dropped on the dedicated driver
// thread; the bound is needed so the generic driver can be moved there.
unsafe impl Send for NativeBackend {}

impl SimConnectBackend for NativeBackend {
    type OpenContext = HANDLE;

    const RECV_ID_NULL: u32 = sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_NULL as u32;
    const RECV_ID_EXCEPTION: u32 = sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_EXCEPTION as u32;
    const RECV_ID_QUIT: u32 = sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_QUIT as u32;
    const RECV_ID_SIMOBJECT_DATA: u32 =
        sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_SIMOBJECT_DATA as u32;
    const RECV_ID_SIMOBJECT_DATA_BYTYPE: u32 =
        sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_SIMOBJECT_DATA_BYTYPE as u32;
    const RECV_ID_CLIENT_DATA: u32 = sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_CLIENT_DATA as u32;

    fn open(name: &CStr, event: HANDLE) -> Result<Self> {
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
    fn set_data_on_sim_object(
        &mut self,
        define_id: u32,
        object_id: u32,
        data: &[u8],
    ) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_SetDataOnSimObject(
                self.handle,
                define_id,
                object_id,
                0,
                0,
                data.len().try_into().map_err(|_| Error::IdExhausted)?,
                data.as_ptr().cast_mut().cast(),
            )
        })
    }

    fn release_ai_control(&mut self, object_id: u32, request_id: u32) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_AIReleaseControl(self.handle, object_id, request_id)
        })
    }

    fn transmit_client_event(&mut self, object_id: u32, event_id: u32, data: u32) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_TransmitClientEvent(self.handle, object_id, event_id, data, 0, 0)
        })
    }

    fn map_client_event(&mut self, event_id: u32, name: &CStr) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_MapClientEventToSimEvent(self.handle, event_id, name.as_ptr())
        })
    }

    fn request_data(&mut self, request: DataRequest) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                self.handle,
                request.request_id,
                request.define_id,
                request.object_id,
                data_period(request.period),
                if request.changed_only {
                    sys::SIMCONNECT_DATA_REQUEST_FLAG_CHANGED
                } else {
                    0
                },
                request.origin,
                request.interval,
                request.limit,
            )
        })
    }

    fn add_data_definition(
        &mut self,
        define_id: u32,
        datum_name: &CStr,
        units: &CStr,
        datatype: u32,
        epsilon: f32,
    ) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_AddToDataDefinition(
                self.handle,
                define_id,
                datum_name.as_ptr(),
                units.as_ptr(),
                datatype as sys::SIMCONNECT_DATATYPE,
                epsilon,
                sys::SIMCONNECT_UNUSED,
            )
        })
    }

    fn clear_data_definition(&mut self, define_id: u32) -> Result<()> {
        check_hresult(unsafe { sys::SimConnect_ClearDataDefinition(self.handle, define_id) })
    }

    fn create_client_data(&mut self, client_id: u32, size: u32) -> Result<()> {
        check_hresult(unsafe { sys::SimConnect_CreateClientData(self.handle, client_id, size, 0) })
    }

    fn map_client_data_name(&mut self, name: &CStr, client_id: u32) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_MapClientDataNameToID(self.handle, name.as_ptr(), client_id)
        })
    }

    fn add_client_data_definition(
        &mut self,
        define_id: u32,
        offset: u32,
        size_or_type: u32,
        epsilon: f32,
    ) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_AddToClientDataDefinition(
                self.handle,
                define_id,
                offset,
                size_or_type,
                epsilon,
                sys::SIMCONNECT_UNUSED,
            )
        })
    }

    fn clear_client_data_definition(&mut self, define_id: u32) -> Result<()> {
        check_hresult(unsafe { sys::SimConnect_ClearClientDataDefinition(self.handle, define_id) })
    }

    fn set_client_data(&mut self, client_id: u32, define_id: u32, data: &[u8]) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_SetClientData(
                self.handle,
                client_id,
                define_id,
                0,
                0,
                data.len().try_into().map_err(|_| Error::IdExhausted)?,
                data.as_ptr().cast_mut().cast(),
            )
        })
    }

    fn request_client_data(&mut self, request: ClientDataRequest) -> Result<()> {
        check_hresult(unsafe {
            sys::SimConnect_RequestClientData(
                self.handle,
                request.client_id,
                request.request_id,
                request.define_id,
                client_data_period(request.period),
                if request.changed_only {
                    sys::SIMCONNECT_CLIENT_DATA_REQUEST_FLAG_CHANGED
                } else {
                    0
                },
                request.origin,
                request.interval,
                request.limit,
            )
        })
    }

    fn last_send_id(&mut self) -> Result<u32> {
        let mut send_id = 0;
        check_hresult(unsafe { sys::SimConnect_GetLastSentPacketID(self.handle, &mut send_id) })?;
        Ok(send_id)
    }

    fn next_dispatch(&mut self, packet: &mut Vec<u8>) -> Result<bool> {
        let mut recv = std::ptr::null_mut();
        let mut size = 0;
        check_hresult(unsafe {
            sys::SimConnect_GetNextDispatch(self.handle, &mut recv, &mut size)
        })?;
        if recv.is_null() {
            return Ok(false);
        }
        // SAFETY: a successful GetNextDispatch exposes `size` readable bytes
        // in a buffer maintained by the SimConnect client library. The SDK
        // transfers no ownership or durable lifetime to us, so copy it before
        // making another SDK call or returning it to portable driver code.
        packet.extend_from_slice(unsafe {
            std::slice::from_raw_parts(recv.cast::<u8>(), size as usize)
        });
        Ok(true)
    }
}

impl Drop for NativeBackend {
    fn drop(&mut self) {
        let _ = unsafe { sys::SimConnect_Close(self.handle) };
    }
}

fn data_period(period: DataPeriod) -> sys::SIMCONNECT_PERIOD {
    match period {
        DataPeriod::Never => sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_NEVER,
        DataPeriod::Once => sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_ONCE,
        DataPeriod::VisualFrame => sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_VISUAL_FRAME,
        DataPeriod::SimFrame => sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_SIM_FRAME,
        DataPeriod::Second => sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_SECOND,
    }
}

fn client_data_period(period: ClientDataPeriod) -> sys::SIMCONNECT_CLIENT_DATA_PERIOD {
    match period {
        ClientDataPeriod::Never => {
            sys::SIMCONNECT_CLIENT_DATA_PERIOD_SIMCONNECT_CLIENT_DATA_PERIOD_NEVER
        }
        ClientDataPeriod::OnSet => {
            sys::SIMCONNECT_CLIENT_DATA_PERIOD_SIMCONNECT_CLIENT_DATA_PERIOD_ON_SET
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
