#[cfg(windows)]
mod windows {
    use msfs_sync::{ClientDataArea, Error, SimConnect, client_data_definition};
    use std::time::Duration;

    #[client_data_definition]
    #[derive(Debug)]
    struct Data {
        foo: u32,
        bar: i8,
        baz: u8,
        qux: i16,
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let writer = SimConnect::open("CLIENT_DATA_WRITER")?;
        let area = writer.create_client_data::<Data>("data")?;

        let reader = SimConnect::open("CLIENT_DATA_READER")?;
        let updates = reader.subscribe_client_data::<Data>("data")?;

        std::thread::scope(|scope| -> Result<(), Error> {
            let writer = scope.spawn(move || write_values(writer, area));
            let reader = scope.spawn(move || read_values(updates));
            writer.join().expect("writer panicked")?;
            reader.join().expect("reader panicked")?;
            Ok(())
        })?;
        Ok(())
    }

    fn write_values(sim: SimConnect, area: ClientDataArea<Data>) -> Result<(), Error> {
        let mut data = Data {
            foo: 0,
            bar: 42,
            baz: 0,
            qux: 13,
        };

        loop {
            data.foo = data.foo.wrapping_add(1);
            data.baz ^= 1;
            sim.set_client_data(&area, &data)?;
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn read_values(updates: msfs_sync::Subscription<Data>) -> Result<(), Error> {
        for data in updates {
            println!("READER: {:?}", data?);
        }
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
