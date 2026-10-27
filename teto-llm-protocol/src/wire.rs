use crate::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u32 = 4;
pub const JOURNAL_VERSION: u32 = 2;

#[derive(Clone, Serialize, Deserialize)]
pub struct OpenConfig {
    pub assets: PathBuf,
    pub profile: ProfileId,
    pub output_language: OutputLanguage,
    pub system_prompt: String,
    pub seed: u64,
    pub journal: JournalAccess,
    pub asset_cache: Option<PathBuf>,
    pub host: HostIdentity,
    pub delivery_window: DeliveryWindow,
}
impl std::fmt::Debug for OpenConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpenConfig([redacted])")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    Initialize {
        directory: PathBuf,
        identity: JournalIdentity,
        key: SecretKey,
    },
    Inspect(PathBuf),
    PrepareDelete {
        directory: PathBuf,
        key: SecretKey,
    },
    Delete,
    Open(OpenConfig),
    Start(UserInput),
    Pull(TurnId),
    Played(ClausePlayed),
    Cancel(TurnId),
    Finish {
        turn: TurnId,
        playback: PlaybackStop,
        source: ReceiptSource,
    },
    Reset,
    Retry,
    Close,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Identity(JournalIdentity),
    Ready,
    Started(TurnId),
    Event(ClauseEvent),
    Acknowledged,
    Finished(Box<FinalizedTurn>),
    Reset,
    Closed,
    Error(Error),
}

/// Selects whether encoded bytes require zeroization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    Secret,
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    version: u32,
    payload: T,
}

#[derive(Serialize, Deserialize)]
struct IpcEnvelope<T> {
    version: u32,
    seq: u64,
    payload: T,
}

enum Encoded {
    Public(Vec<u8>),
    Secret(Zeroizing<Vec<u8>>),
}
impl Encoded {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Public(bytes) => bytes,
            Self::Secret(bytes) => bytes,
        }
    }
}

/// Encode journal plaintext. The bytes are zeroized on drop.
pub fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>> {
    let mut writer = SecretWriter(Zeroizing::new(Vec::new()));
    ciborium::into_writer(value, &mut writer).map_err(|_| failure(ErrorKind::Protocol))?;
    Ok(writer.0)
}

struct PublicWriter(Vec<u8>);
impl Write for PublicWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > policy::FRAME_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct SecretWriter(Zeroizing<Vec<u8>>);
impl Write for SecretWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > policy::FRAME_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encode_with<T: Serialize>(value: &T, sensitivity: Sensitivity) -> Result<Encoded> {
    match sensitivity {
        Sensitivity::Public => {
            let mut writer = PublicWriter(Vec::new());
            ciborium::into_writer(value, &mut writer).map_err(|_| failure(ErrorKind::Protocol))?;
            Ok(Encoded::Public(writer.0))
        }
        Sensitivity::Secret => {
            let mut writer = SecretWriter(Zeroizing::new(Vec::new()));
            ciborium::into_writer(value, &mut writer).map_err(|_| failure(ErrorKind::Protocol))?;
            Ok(Encoded::Secret(writer.0))
        }
    }
}

fn write_bytes(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| failure(ErrorKind::Protocol))?;
    writer
        .write_all(&length.to_be_bytes())
        .and_then(|()| writer.write_all(bytes))
        .and_then(|()| writer.flush())
        .map_err(|_| failure(ErrorKind::Worker))
}

fn read_bytes(reader: &mut impl Read) -> Result<Zeroizing<Vec<u8>>> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| failure(ErrorKind::Worker))?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > policy::FRAME_BYTES {
        return Err(failure(ErrorKind::Protocol));
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    reader
        .read_exact(&mut bytes)
        .map_err(|_| failure(ErrorKind::Worker))?;
    Ok(bytes)
}

/// Write one journal frame. The frame has a protocol version.
pub fn write_frame<T: Serialize>(writer: &mut impl Write, payload: &T) -> Result<()> {
    let encoded = encode_with(
        &Envelope {
            version: JOURNAL_VERSION,
            payload,
        },
        Sensitivity::Public,
    )?;
    write_bytes(writer, encoded.as_slice())
}

/// Read one journal frame. The frame must consume the full payload.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
    let bytes = read_bytes(reader)?;
    let mut cursor = std::io::Cursor::new(bytes.as_slice());
    let envelope: Envelope<T> =
        ciborium::from_reader(&mut cursor).map_err(|_| failure(ErrorKind::Protocol))?;
    if envelope.version != JOURNAL_VERSION || cursor.position() != bytes.len() as u64 {
        return Err(failure(ErrorKind::Protocol));
    }
    Ok(envelope.payload)
}

/// Write one IPC frame. `seq` identifies the request or response.
pub fn write_ipc<T: Serialize>(
    writer: &mut impl Write,
    seq: u64,
    sensitivity: Sensitivity,
    payload: &T,
) -> Result<()> {
    let encoded = encode_with(
        &IpcEnvelope {
            version: PROTOCOL_VERSION,
            seq,
            payload,
        },
        sensitivity,
    )?;
    write_bytes(writer, encoded.as_slice())
}

/// Read one IPC frame. Returns the sequence number and the payload.
pub fn read_ipc<T: DeserializeOwned>(reader: &mut impl Read) -> Result<(u64, T)> {
    let bytes = read_bytes(reader)?;
    let mut cursor = std::io::Cursor::new(bytes.as_slice());
    let envelope: IpcEnvelope<T> =
        ciborium::from_reader(&mut cursor).map_err(|_| failure(ErrorKind::Protocol))?;
    if envelope.version != PROTOCOL_VERSION || cursor.position() != bytes.len() as u64 {
        return Err(failure(ErrorKind::Protocol));
    }
    Ok((envelope.seq, envelope.payload))
}
