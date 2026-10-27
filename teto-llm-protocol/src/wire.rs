use crate::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u32 = 1;
#[derive(Serialize, Deserialize)]
pub struct OpenConfig {
    pub assets: PathBuf,
    pub profile: ProfileId,
    pub output_language: OutputLanguage,
    pub system_prompt: String,
    pub seed: u64,
    pub journal: JournalAccess,
}
impl std::fmt::Debug for OpenConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenConfig([redacted])")
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
#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    version: u32,
    payload: T,
}

/// Bound encoded size during serialization, not after allocating an arbitrary Vec.
pub fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>> {
    let mut writer = LimitedWriter(Zeroizing::new(Vec::new()));
    ciborium::into_writer(value, &mut writer).map_err(|_| failure(ErrorKind::Protocol))?;
    Ok(writer.0)
}
struct LimitedWriter(Zeroizing<Vec<u8>>);
impl Write for LimitedWriter {
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

pub fn write_frame<T: Serialize>(writer: &mut impl Write, payload: &T) -> Result<()> {
    let bytes = encode(&Envelope {
        version: PROTOCOL_VERSION,
        payload,
    })?;
    let len = u32::try_from(bytes.len()).map_err(|_| failure(ErrorKind::Protocol))?;
    writer
        .write_all(&len.to_be_bytes())
        .and_then(|()| writer.write_all(&bytes))
        .and_then(|()| writer.flush())
        .map_err(|_| failure(ErrorKind::Worker))
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
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
    let mut cursor = std::io::Cursor::new(bytes.as_slice());
    let envelope: Envelope<T> =
        ciborium::from_reader(&mut cursor).map_err(|_| failure(ErrorKind::Protocol))?;
    if envelope.version != PROTOCOL_VERSION || cursor.position() != length as u64 {
        return Err(failure(ErrorKind::Protocol));
    }
    Ok(envelope.payload)
}
