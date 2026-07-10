#[cfg(windows)]
mod windows {
    use futures_util::TryStreamExt;
    use msfs_async::{AsyncSimConnect, ClientDataArea, client_data_definition};
    use std::time::Duration;

    #[client_data_definition]
    #[derive(Debug)]
    struct Data {
        foo: u32,
        bar: i8,
        // Use an integer instead of bool so every possible byte is valid.
        baz: u8,
        qux: i16,
    }

    #[tokio::main]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // Use separate SimConnect sessions, matching the upstream example's
        // independent writer and reader clients.
        let writer = AsyncSimConnect::open("CLIENT_DATA_WRITER").await?;
        let area = writer.create_client_data::<Data>("data").await?;

        let reader = AsyncSimConnect::open("CLIENT_DATA_READER").await?;
        let updates = reader.subscribe_client_data::<Data>("data").await?;

        futures_util::try_join!(write_values(writer, area), read_values(updates))?;
        Ok(())
    }

    async fn write_values(
        sim: AsyncSimConnect,
        area: ClientDataArea<Data>,
    ) -> Result<(), msfs_async::Error> {
        let mut data = Data {
            foo: 0,
            bar: 42,
            baz: 0,
            qux: 13,
        };

        loop {
            data.foo = data.foo.wrapping_add(1);
            data.baz ^= 1;
            sim.set_client_data(&area, &data).await?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn read_values(updates: msfs_async::Subscription<Data>) -> Result<(), msfs_async::Error> {
        updates
            .try_for_each(|data| async move {
                println!("READER: {data:?}");
                Ok(())
            })
            .await
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
