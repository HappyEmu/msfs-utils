use crate::DynError;
use crate::protocol::{self, AircraftUpdate, Message, UserId};
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(33);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

struct ClientState {
    user_id: UserId,
    last_seen: Instant,
    update: Option<AircraftUpdate>,
}

pub async fn run(bind_address: SocketAddr) -> Result<(), DynError> {
    let socket = UdpSocket::bind(bind_address).await?;
    eprintln!("Multiplayer relay listening on {}.", socket.local_addr()?);

    let mut clients = HashMap::<SocketAddr, ClientState>::new();
    let mut next_user_id: UserId = 1;
    let mut snapshot_sequence = 0_u64;
    let mut buffer = vec![0; protocol::MAX_PACKET_SIZE];
    let mut snapshots = tokio::time::interval(SNAPSHOT_INTERVAL);
    snapshots.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            received = socket.recv_from(&mut buffer) => {
                let (length, address) = match received {
                    Ok(packet) => packet,
                    Err(error) if is_recoverable_udp_error(&error) => continue,
                    Err(error) => return Err(error.into()),
                };
                let Ok(message) = protocol::decode(&buffer[..length]) else {
                    continue;
                };
                match message {
                    Message::Join => {
                        let now = Instant::now();
                        let user_id = match clients.get_mut(&address) {
                            Some(client) => {
                                client.last_seen = now;
                                client.user_id
                            }
                            None => {
                                let user_id = next_user_id;
                                next_user_id = next_user_id.checked_add(1).ok_or("user ID space exhausted")?;
                                clients.insert(address, ClientState {
                                    user_id,
                                    last_seen: now,
                                    update: None,
                                });
                                eprintln!("Registered multiplayer user {user_id} at {address}.");
                                user_id
                            }
                        };
                        let welcome = protocol::encode(&Message::Welcome { user_id })?;
                        socket.send_to(&welcome, address).await?;
                    }
                    Message::Update(update) => {
                        if let Some(client) = clients.get_mut(&address) {
                            accept_update(client, update, Instant::now());
                        }
                    }
                    Message::Leave { user_id } => {
                        if clients.get(&address).is_some_and(|client| client.user_id == user_id) {
                            clients.remove(&address);
                            eprintln!("Removed multiplayer user {user_id}.");
                        }
                    }
                    Message::Welcome { .. } | Message::Snapshot { .. } => {}
                }
            }
            _ = snapshots.tick() => {
                let now = Instant::now();
                clients.retain(|address, client| {
                    let keep = now.duration_since(client.last_seen) <= CLIENT_TIMEOUT;
                    if !keep {
                        eprintln!("Expired multiplayer user {} at {address}.", client.user_id);
                    }
                    keep
                });

                snapshot_sequence = snapshot_sequence.wrapping_add(1);
                let recipients = clients
                    .iter()
                    .map(|(address, client)| (*address, client.user_id))
                    .collect::<Vec<_>>();
                for (address, recipient_id) in recipients {
                    let aircraft = snapshot_for(&clients, recipient_id);
                    let packet = protocol::encode(&Message::Snapshot {
                        sequence: snapshot_sequence,
                        aircraft,
                    })?;
                    if let Err(error) = socket.send_to(&packet, address).await {
                        if is_recoverable_udp_error(&error) {
                            clients.remove(&address);
                            eprintln!(
                                "Removed unreachable multiplayer user {recipient_id} at {address}."
                            );
                            continue;
                        }
                        return Err(error.into());
                    }
                }
            }
        }
    }
}

fn is_recoverable_udp_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::Interrupted
    )
}

fn accept_update(client: &mut ClientState, update: AircraftUpdate, now: Instant) -> bool {
    if update.user_id != client.user_id
        || !update.timestamp_seconds.is_finite()
        || !update.state.is_finite()
        || client
            .update
            .is_some_and(|current| update.sequence <= current.sequence)
    {
        return false;
    }
    client.last_seen = now;
    client.update = Some(update);
    true
}

fn snapshot_for(
    clients: &HashMap<SocketAddr, ClientState>,
    recipient_id: UserId,
) -> Vec<AircraftUpdate> {
    clients
        .values()
        .filter(|client| client.user_id != recipient_id)
        .filter_map(|client| client.update)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AircraftState;

    fn client(user_id: UserId, sequence: u64) -> ClientState {
        ClientState {
            user_id,
            last_seen: Instant::now(),
            update: Some(AircraftUpdate {
                user_id,
                sequence,
                timestamp_seconds: sequence as f64,
                state: AircraftState::default(),
            }),
        }
    }

    #[test]
    fn snapshots_exclude_the_recipient() {
        let clients = HashMap::from([
            ("127.0.0.1:1001".parse().unwrap(), client(1, 1)),
            ("127.0.0.1:1002".parse().unwrap(), client(2, 1)),
        ]);
        let snapshot = snapshot_for(&clients, 1);

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].user_id, 2);
    }

    #[test]
    fn rejects_old_or_wrong_user_updates() {
        let mut client = client(7, 10);
        let now = Instant::now();
        let old = AircraftUpdate {
            user_id: 7,
            sequence: 9,
            timestamp_seconds: 9.0,
            state: AircraftState::default(),
        };
        assert!(!accept_update(&mut client, old, now));

        let wrong_user = AircraftUpdate {
            user_id: 8,
            sequence: 11,
            timestamp_seconds: 11.0,
            state: AircraftState::default(),
        };
        assert!(!accept_update(&mut client, wrong_user, now));
        assert_eq!(client.update.unwrap().sequence, 10);
    }

    #[test]
    fn peer_disconnect_udp_errors_do_not_stop_the_relay() {
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::Interrupted,
        ] {
            assert!(is_recoverable_udp_error(&io::Error::from(kind)));
        }

        assert!(!is_recoverable_udp_error(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }
}
