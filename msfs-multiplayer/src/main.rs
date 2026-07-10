#[cfg(any(windows, test))]
mod drift;
#[cfg(windows)]
mod live;
mod network;
mod replay;
mod server;
#[cfg(windows)]
mod simconnect;
#[cfg(any(windows, test))]
mod timeline;

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::{Arc, Mutex};

pub use msfs_multiplayer::protocol;

pub type DynError = Box<dyn Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), DynError> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("server") => {
            let bind_address = args.next().as_deref().unwrap_or("0.0.0.0:9997").parse()?;
            reject_extra_args(args)?;
            server::run(bind_address).await
        }
        Some("client") => {
            let server = required(&mut args, "server address")?.parse()?;
            let model_title = required(&mut args, "aircraft container title")?;
            reject_extra_args(args)?;
            run_live_client(server, model_title).await
        }
        Some("replay") => {
            let server = required(&mut args, "server address")?.parse()?;
            let recording = PathBuf::from(required(&mut args, "recording path")?);
            let count = args
                .next()
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(1);
            if count == 0 {
                return Err(input_error("replay client count must be greater than zero").into());
            }
            reject_extra_args(args)?;
            replay::run(server, &recording, count).await
        }
        _ => Err(input_error(usage()).into()),
    }
}

#[cfg(windows)]
async fn run_live_client(server: SocketAddr, model_title: String) -> Result<(), DynError> {
    let client = network::RelayClient::connect(server).await?;
    eprintln!("Connected to relay as user {}.", client.user_id());

    let local_state = Arc::new(Mutex::new(None));
    let remote_snapshot = Arc::new(Mutex::new(None));
    let (sim_done_tx, sim_done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("msfs-multiplayer-sim".to_owned())
        .spawn({
            let local_state = Arc::clone(&local_state);
            let remote_snapshot = Arc::clone(&remote_snapshot);
            move || {
                let _ =
                    sim_done_tx.send(simconnect::run(&model_title, local_state, remote_snapshot));
            }
        })?;

    tokio::select! {
        result = live::run(&client, local_state, remote_snapshot) => result,
        result = sim_done_rx => result.map_err(|_| input_error("SimConnect worker stopped without a result"))?,
    }
}

#[cfg(not(windows))]
async fn run_live_client(_server: SocketAddr, _model_title: String) -> Result<(), DynError> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the multiplayer SimConnect client requires Windows and the MSFS SDK",
    )
    .into())
}

fn required(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, DynError> {
    args.next()
        .ok_or_else(|| input_error(format!("missing {name}\n{}", usage())).into())
}

fn reject_extra_args(mut args: impl Iterator<Item = String>) -> Result<(), DynError> {
    if args.next().is_some() {
        Err(input_error(usage()).into())
    } else {
        Ok(())
    }
}

fn input_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn usage() -> &'static str {
    "usage:\n  msfs-multiplayer server [bind-address=0.0.0.0:9997]\n  msfs-multiplayer client <server-address> <aircraft-container-title>\n  msfs-multiplayer replay <server-address> <recording.csv> [client-count=1]"
}
