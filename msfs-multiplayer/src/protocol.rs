use std::fmt;

const MAGIC: &[u8; 4] = b"FSMP";
const VERSION: u8 = 1;
const JOIN: u8 = 1;
const WELCOME: u8 = 2;
const UPDATE: u8 = 3;
const SNAPSHOT: u8 = 4;
const LEAVE: u8 = 5;

pub const MAX_PACKET_SIZE: usize = 60_000;

pub type UserId = u64;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AircraftState {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub indicated_airspeed: f64,
    pub heading: f64,
    pub pitch: f64,
    pub bank: f64,
    pub velocity_world_x: f64,
    pub velocity_world_y: f64,
    pub velocity_world_z: f64,
    pub velocity_body_x: f64,
    pub velocity_body_y: f64,
    pub velocity_body_z: f64,
    pub on_ground: bool,
}

impl AircraftState {
    pub fn is_finite(self) -> bool {
        [
            self.latitude,
            self.longitude,
            self.altitude,
            self.indicated_airspeed,
            self.heading,
            self.pitch,
            self.bank,
            self.velocity_world_x,
            self.velocity_world_y,
            self.velocity_world_z,
            self.velocity_body_x,
            self.velocity_body_y,
            self.velocity_body_z,
        ]
        .into_iter()
        .all(f64::is_finite)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AircraftUpdate {
    pub user_id: UserId,
    pub sequence: u64,
    pub timestamp_seconds: f64,
    pub state: AircraftState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Join,
    Welcome {
        user_id: UserId,
    },
    Update(AircraftUpdate),
    Snapshot {
        sequence: u64,
        aircraft: Vec<AircraftUpdate>,
    },
    Leave {
        user_id: UserId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    InvalidMagic,
    UnsupportedVersion(u8),
    UnknownMessage(u8),
    Truncated,
    TrailingBytes,
    InvalidBoolean(u8),
    TooManyAircraft,
    PacketTooLarge,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => formatter.write_str("invalid multiplayer packet magic"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported multiplayer protocol version {version}"
                )
            }
            Self::UnknownMessage(kind) => write!(formatter, "unknown multiplayer message {kind}"),
            Self::Truncated => formatter.write_str("truncated multiplayer packet"),
            Self::TrailingBytes => formatter.write_str("multiplayer packet has trailing bytes"),
            Self::InvalidBoolean(value) => {
                write!(formatter, "invalid multiplayer boolean value {value}")
            }
            Self::TooManyAircraft => formatter.write_str("snapshot contains too many aircraft"),
            Self::PacketTooLarge => formatter.write_str("multiplayer packet is too large for UDP"),
        }
    }
}

impl std::error::Error for ProtocolError {}

pub fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    match message {
        Message::Join => bytes.push(JOIN),
        Message::Welcome { user_id } => {
            bytes.push(WELCOME);
            push_u64(&mut bytes, *user_id);
        }
        Message::Update(update) => {
            bytes.push(UPDATE);
            encode_update(&mut bytes, update);
        }
        Message::Snapshot { sequence, aircraft } => {
            bytes.push(SNAPSHOT);
            push_u64(&mut bytes, *sequence);
            let count =
                u16::try_from(aircraft.len()).map_err(|_| ProtocolError::TooManyAircraft)?;
            bytes.extend_from_slice(&count.to_le_bytes());
            for update in aircraft {
                encode_update(&mut bytes, update);
            }
        }
        Message::Leave { user_id } => {
            bytes.push(LEAVE);
            push_u64(&mut bytes, *user_id);
        }
    }
    if bytes.len() > MAX_PACKET_SIZE {
        return Err(ProtocolError::PacketTooLarge);
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Message, ProtocolError> {
    let mut reader = Reader::new(bytes);
    if reader.take(4)? != MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    let version = reader.u8()?;
    if version != VERSION {
        return Err(ProtocolError::UnsupportedVersion(version));
    }
    let message = match reader.u8()? {
        JOIN => Message::Join,
        WELCOME => Message::Welcome {
            user_id: reader.u64()?,
        },
        UPDATE => Message::Update(reader.update()?),
        SNAPSHOT => {
            let sequence = reader.u64()?;
            let count = reader.u16()?;
            let mut aircraft = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                aircraft.push(reader.update()?);
            }
            Message::Snapshot { sequence, aircraft }
        }
        LEAVE => Message::Leave {
            user_id: reader.u64()?,
        },
        kind => return Err(ProtocolError::UnknownMessage(kind)),
    };
    if reader.remaining() != 0 {
        return Err(ProtocolError::TrailingBytes);
    }
    Ok(message)
}

fn encode_update(bytes: &mut Vec<u8>, update: &AircraftUpdate) {
    push_u64(bytes, update.user_id);
    push_u64(bytes, update.sequence);
    push_f64(bytes, update.timestamp_seconds);
    encode_state(bytes, &update.state);
}

fn encode_state(bytes: &mut Vec<u8>, state: &AircraftState) {
    for value in [
        state.latitude,
        state.longitude,
        state.altitude,
        state.indicated_airspeed,
        state.heading,
        state.pitch,
        state.bank,
        state.velocity_world_x,
        state.velocity_world_y,
        state.velocity_world_z,
        state.velocity_body_x,
        state.velocity_body_y,
        state.velocity_body_z,
    ] {
        push_f64(bytes, value);
    }
    bytes.push(u8::from(state.on_ground));
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f64(bytes: &mut Vec<u8>, value: f64) {
    push_u64(bytes, value.to_bits());
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(ProtocolError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ProtocolError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes = self.take(2)?.try_into().expect("slice length was checked");
        Ok(u16::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes = self.take(8)?.try_into().expect("slice length was checked");
        Ok(u64::from_le_bytes(bytes))
    }

    fn f64(&mut self) -> Result<f64, ProtocolError> {
        Ok(f64::from_bits(self.u64()?))
    }

    fn boolean(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ProtocolError::InvalidBoolean(value)),
        }
    }

    fn update(&mut self) -> Result<AircraftUpdate, ProtocolError> {
        Ok(AircraftUpdate {
            user_id: self.u64()?,
            sequence: self.u64()?,
            timestamp_seconds: self.f64()?,
            state: AircraftState {
                latitude: self.f64()?,
                longitude: self.f64()?,
                altitude: self.f64()?,
                indicated_airspeed: self.f64()?,
                heading: self.f64()?,
                pitch: self.f64()?,
                bank: self.f64()?,
                velocity_world_x: self.f64()?,
                velocity_world_y: self.f64()?,
                velocity_world_z: self.f64()?,
                velocity_body_x: self.f64()?,
                velocity_body_y: self.f64()?,
                velocity_body_z: self.f64()?,
                on_ground: self.boolean()?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(user_id: UserId) -> AircraftUpdate {
        AircraftUpdate {
            user_id,
            sequence: 7,
            timestamp_seconds: 1.5,
            state: AircraftState {
                latitude: 47.0,
                longitude: 8.0,
                altitude: 1_500.0,
                indicated_airspeed: 120.0,
                heading: 270.0,
                pitch: 2.0,
                bank: -3.0,
                velocity_world_x: 1.0,
                velocity_world_y: 2.0,
                velocity_world_z: 3.0,
                velocity_body_x: 4.0,
                velocity_body_y: 5.0,
                velocity_body_z: 6.0,
                on_ground: true,
            },
        }
    }

    #[test]
    fn round_trips_every_message() {
        let messages = [
            Message::Join,
            Message::Welcome { user_id: 42 },
            Message::Update(update(42)),
            Message::Snapshot {
                sequence: 9,
                aircraft: vec![update(42), update(43)],
            },
            Message::Leave { user_id: 42 },
        ];

        for message in messages {
            assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
        }
    }

    #[test]
    fn rejects_truncated_and_trailing_packets() {
        let mut packet = encode(&Message::Update(update(1))).unwrap();
        packet.pop();
        assert_eq!(decode(&packet), Err(ProtocolError::Truncated));

        let mut packet = encode(&Message::Join).unwrap();
        packet.push(0);
        assert_eq!(decode(&packet), Err(ProtocolError::TrailingBytes));
    }

    #[test]
    fn rejects_bad_headers_and_boolean_values() {
        let mut packet = encode(&Message::Join).unwrap();
        packet[0] = b'X';
        assert_eq!(decode(&packet), Err(ProtocolError::InvalidMagic));

        let mut packet = encode(&Message::Update(update(1))).unwrap();
        *packet.last_mut().unwrap() = 2;
        assert_eq!(decode(&packet), Err(ProtocolError::InvalidBoolean(2)));
    }
}
