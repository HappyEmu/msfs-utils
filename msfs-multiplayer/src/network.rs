use crate::DynError;
use crate::protocol::{self, AircraftState, AircraftUpdate, Message, UserId};
use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

pub struct RelayClient {
    socket: UdpSocket,
    user_id: UserId,
}

impl RelayClient {
    pub async fn connect(server: SocketAddr) -> Result<Self, DynError> {
        let bind_address = if server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind_address).await?;
        socket.connect(server).await?;
        let join = protocol::encode(&Message::Join)?;
        let mut buffer = vec![0; protocol::MAX_PACKET_SIZE];

        for _ in 0..5 {
            socket.send(&join).await?;
            let received = tokio::time::timeout(Duration::from_secs(1), socket.recv(&mut buffer));
            if let Ok(Ok(length)) = received.await {
                if let Ok(Message::Welcome { user_id }) = protocol::decode(&buffer[..length]) {
                    return Ok(Self { socket, user_id });
                }
            }
        }

        Err(io::Error::new(io::ErrorKind::TimedOut, "multiplayer relay did not answer").into())
    }

    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    pub async fn send_update(
        &self,
        sequence: u64,
        timestamp_seconds: f64,
        state: AircraftState,
    ) -> Result<(), DynError> {
        let packet = protocol::encode(&Message::Update(AircraftUpdate {
            user_id: self.user_id,
            sequence,
            timestamp_seconds,
            state,
        }))?;
        self.socket.send(&packet).await?;
        Ok(())
    }

    pub async fn receive_snapshot(
        &self,
        buffer: &mut [u8],
    ) -> Result<Option<Vec<AircraftUpdate>>, DynError> {
        let length = self.socket.recv(buffer).await?;
        match protocol::decode(&buffer[..length])? {
            Message::Snapshot { aircraft, .. } => Ok(Some(aircraft)),
            Message::Welcome { .. } => Ok(None),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "relay sent an unexpected multiplayer message",
            )
            .into()),
        }
    }

    pub async fn leave(&self) -> Result<(), DynError> {
        let packet = protocol::encode(&Message::Leave {
            user_id: self.user_id,
        })?;
        self.socket.send(&packet).await?;
        Ok(())
    }
}
