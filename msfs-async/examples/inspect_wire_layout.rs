#[cfg(windows)]
mod windows {
    use msfs_async::__sys as sys;
    use std::ffi::CString;
    use std::io;
    use std::time::{Duration, Instant};

    const DEFINE_ID: u32 = 0;
    const REQUEST_ID: u32 = 0;
    const DATA_PAYLOAD_OFFSET: usize = 40;

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let name = CString::new("MSFS WIRE LAYOUT PROBE")?;
        let mut handle: sys::HANDLE = unsafe { std::mem::zeroed() };
        check("SimConnect_Open", unsafe {
            sys::SimConnect_Open(
                &mut handle,
                name.as_ptr(),
                std::mem::zeroed(),
                0,
                std::mem::zeroed(),
                sys::SIMCONNECT_OPEN_CONFIGINDEX_LOCAL,
            )
        })?;
        let connection = Connection(handle);

        add_definition(
            handle,
            "NUMBER OF ENGINES",
            "number",
            sys::SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_INT32,
        )?;
        add_definition(
            handle,
            "PLANE ALTITUDE",
            "feet",
            sys::SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_FLOAT64,
        )?;
        add_definition(
            handle,
            "ENGINE TYPE",
            "enum",
            sys::SIMCONNECT_DATATYPE_SIMCONNECT_DATATYPE_INT32,
        )?;

        check("SimConnect_RequestDataOnSimObject", unsafe {
            sys::SimConnect_RequestDataOnSimObject(
                handle,
                REQUEST_ID,
                DEFINE_ID,
                sys::SIMCONNECT_OBJECT_ID_USER,
                sys::SIMCONNECT_PERIOD_SIMCONNECT_PERIOD_ONCE,
                0,
                0,
                0,
                0,
            )
        })?;

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let mut receive = std::ptr::null_mut();
            let mut received_size = 0;
            let result = unsafe {
                sys::SimConnect_GetNextDispatch(handle, &mut receive, &mut received_size)
            };
            if result >= 0 && !receive.is_null() {
                // SAFETY: SimConnect returned `received_size` readable bytes
                // which remain valid until the next dispatch API call.
                let packet = unsafe {
                    std::slice::from_raw_parts(receive.cast::<u8>(), received_size as usize)
                };
                if inspect_packet(packet)? {
                    drop(connection);
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "no SimObject data packet arrived within ten seconds",
        )
        .into())
    }

    fn add_definition(
        handle: sys::HANDLE,
        name: &str,
        units: &str,
        datatype: sys::SIMCONNECT_DATATYPE,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let operation = format!("SimConnect_AddToDataDefinition({name})");
        let name = CString::new(name)?;
        let units = CString::new(units)?;
        check(&operation, unsafe {
            sys::SimConnect_AddToDataDefinition(
                handle,
                DEFINE_ID,
                name.as_ptr(),
                units.as_ptr(),
                datatype,
                0.0,
                sys::SIMCONNECT_UNUSED,
            )
        })?;
        Ok(())
    }

    fn inspect_packet(packet: &[u8]) -> Result<bool, io::Error> {
        let declared_size = read_u32(packet, 0)? as usize;
        let receive_id = read_u32(packet, 8)?;
        if receive_id == sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_EXCEPTION as u32 {
            return Err(io::Error::other(format!(
                "SimConnect exception code {} for send ID {} at argument {}",
                read_u32(packet, 12)?,
                read_u32(packet, 16)?,
                read_u32(packet, 20)?,
            )));
        }
        if receive_id != sys::SIMCONNECT_RECV_ID_SIMCONNECT_RECV_ID_SIMOBJECT_DATA as u32
            || read_u32(packet, 12)? != REQUEST_ID
        {
            return Ok(false);
        }
        if declared_size > packet.len() || declared_size < DATA_PAYLOAD_OFFSET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid packet size: declared {declared_size}, received {}",
                    packet.len()
                ),
            ));
        }

        let define_count = read_u32(packet, 36)?;
        let payload = &packet[DATA_PAYLOAD_OFFSET..declared_size];
        println!("dwSize: {declared_size}");
        println!("dwDefineCount: {define_count}");
        println!("payload bytes: {}", payload.len());
        print!("payload hex:");
        for byte in payload {
            print!(" {byte:02x}");
        }
        println!();

        println!("header datum-count prediction: 3");
        println!(
            "web 8-byte-element prediction: {}",
            payload.len().div_ceil(8)
        );
        let matches_header = define_count == 3;
        let matches_web = define_count as usize == payload.len().div_ceil(8);
        match (matches_header, matches_web) {
            (true, true) => println!("count result: both interpretations predict this value"),
            (true, false) => {
                println!("count result: matches the header's datum-count interpretation")
            }
            (false, true) => {
                println!(
                    "count result: matches the web documentation's 8-byte-element interpretation"
                )
            }
            (false, false) => {
                println!("count result: matches neither documented interpretation")
            }
        }
        match payload.len() {
            16 => println!("layout result: matches packed scalar sizes"),
            24 => println!("layout result: matches natural C alignment for INT32/FLOAT64/INT32"),
            _ => println!("layout result: matches neither 16-byte nor 24-byte prediction"),
        }

        if payload.len() >= 16 {
            println!("packed INT32 at offset 0: {}", read_i32(payload, 0)?);
            println!("packed FLOAT64 at offset 4: {}", read_f64(payload, 4)?);
            println!("packed INT32 at offset 12: {}", read_i32(payload, 12)?);
        }
        if payload.len() >= 20 {
            println!("C-aligned FLOAT64 at offset 8: {}", read_f64(payload, 8)?);
            println!("C-aligned INT32 at offset 16: {}", read_i32(payload, 16)?);
        }
        Ok(true)
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, io::Error> {
        bytes
            .get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_ne_bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "truncated u32"))
    }

    fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, io::Error> {
        read_u32(bytes, offset).map(|value| value as i32)
    }

    fn read_f64(bytes: &[u8], offset: usize) -> Result<f64, io::Error> {
        bytes
            .get(offset..offset + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(f64::from_ne_bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "truncated f64"))
    }

    fn check(operation: &str, result: sys::HRESULT) -> Result<(), io::Error> {
        if result >= 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "{operation} failed with SimConnect HRESULT {:#010x}",
                result as i32
            )))
        }
    }

    struct Connection(sys::HANDLE);

    impl Drop for Connection {
        fn drop(&mut self) {
            let _ = unsafe { sys::SimConnect_Close(self.0) };
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows, a running MSFS instance, and the MSFS SDK");
}
