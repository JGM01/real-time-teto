//! Single-writer append-only AEAD journal. No replay into model context.
use crate::*;
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub type Identity = JournalIdentity;
#[derive(Serialize, Deserialize)]
struct Header {
    identity: Identity,
    nonce: [u8; 24],
    #[serde(with = "serde_bytes")]
    authentication: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Aad {
    journal: [u8; 16],
    key: [u8; 16],
    schema: u32,
    segment: u32,
    record: u64,
    plain_length: u32,
    cipher_length: u32,
}

#[cfg(test)]
mod tests;
#[derive(Serialize, Deserialize)]
struct Frame {
    aad: Aad,
    nonce: [u8; 24],
    #[serde(with = "serde_bytes")]
    ciphertext: Vec<u8>,
}
struct Pending {
    bytes: Zeroizing<Vec<u8>>,
    segment: u32,
    offset: u64,
    durable: bool,
}

/// Holds the exclusive file lock and key for the entire journal lifetime.
pub struct Journal {
    directory: PathBuf,
    _lock: File,
    identity: Identity,
    key: SecretKey,
    segment: u32,
    length: u64,
    next_record: u64,
    pending: Option<Pending>,
}

fn persistence<T>(_: T) -> Error {
    failure(ErrorKind::Persistence)
}
pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(persistence)?;
    Ok(bytes)
}
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(persistence)
}
fn segment_path(directory: &Path, segment: u32) -> PathBuf {
    directory.join(format!("segment-{segment:02}.cbor"))
}
fn regular(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(persistence)?;
    if !metadata.is_file() {
        return Err(failure(ErrorKind::Persistence));
    }
    Ok(())
}

impl Journal {
    /// Identity is untrusted until `open` authenticates the header with its key.
    pub fn inspect(directory: &Path) -> Result<Identity> {
        regular(&directory.join("header.cbor"))?;
        let header: Header =
            read_frame(&mut File::open(directory.join("header.cbor")).map_err(persistence)?)
                .map_err(persistence)?;
        Ok(header.identity)
    }
    /// Create a new directory exclusively. Never replaces an existing journal.
    pub fn initialize(directory: &Path, identity: Identity, key: &SecretKey) -> Result<()> {
        fs::create_dir(directory).map_err(persistence)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(persistence)?;
        }
        let result = Self::initialize_files(directory, identity, key);
        if result.is_err() {
            let _ = fs::remove_dir_all(directory);
        }
        result
    }

    fn initialize_files(directory: &Path, identity: Identity, key: &SecretKey) -> Result<()> {
        let lock = private_options()
            .write(true)
            .create_new(true)
            .open(directory.join("writer.lock"))
            .map_err(persistence)?;
        lock.try_lock_exclusive().map_err(persistence)?;
        let aad = encode(&identity)?;
        let nonce = random()?;
        let authentication = XChaCha20Poly1305::new(key.bytes().into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: b"Teto journal v1",
                    aad: &aad,
                },
            )
            .map_err(persistence)?;
        let header = Header {
            identity,
            nonce,
            authentication,
        };
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(directory.join("header.cbor"))
            .map_err(persistence)?;
        write_frame(&mut file, &header)?;
        file.sync_all().map_err(persistence)?;
        let segment = private_options()
            .write(true)
            .create_new(true)
            .open(segment_path(directory, 0))
            .map_err(persistence)?;
        segment.sync_all().map_err(persistence)?;
        sync_directory(directory)?;
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    /// Validate all records without restoring any model history.
    pub fn open(directory: PathBuf, key: SecretKey) -> Result<Self> {
        let mut journal = Self::open_for_delete(directory, key)?;
        if journal.directory.join("pending.cbor").exists() {
            // Preserve the encrypted artifact. Startup never retries or replays it implicitly.
            return Err(failure(ErrorKind::Persistence));
        }
        journal.validate_segments()?;
        journal.check_capacity()?;
        Ok(journal)
    }

    pub fn open_for_delete(directory: PathBuf, key: SecretKey) -> Result<Self> {
        let metadata = fs::symlink_metadata(&directory).map_err(persistence)?;
        if !metadata.is_dir() {
            return Err(failure(ErrorKind::Persistence));
        }
        regular(&directory.join("writer.lock"))?;
        let lock = private_options()
            .read(true)
            .write(true)
            .open(directory.join("writer.lock"))
            .map_err(persistence)?;
        lock.try_lock_exclusive().map_err(persistence)?;
        regular(&directory.join("header.cbor"))?;
        let header: Header =
            read_frame(&mut File::open(directory.join("header.cbor")).map_err(persistence)?)
                .map_err(persistence)?;
        let aad = encode(&header.identity)?;
        let plain = XChaCha20Poly1305::new(key.bytes().into())
            .decrypt(
                XNonce::from_slice(&header.nonce),
                Payload {
                    msg: &header.authentication,
                    aad: &aad,
                },
            )
            .map_err(persistence)?;
        if plain != b"Teto journal v1" {
            return Err(failure(ErrorKind::Persistence));
        }
        let journal = Self {
            directory,
            _lock: lock,
            identity: header.identity,
            key,
            segment: 0,
            length: 0,
            next_record: 1,
            pending: None,
        };
        Ok(journal)
    }

    fn validate_segments(&mut self) -> Result<()> {
        let mut missing = false;
        for segment in 0..policy::SEGMENTS {
            let path = segment_path(&self.directory, segment);
            if !path.exists() {
                missing = true;
                continue;
            }
            if missing {
                return Err(failure(ErrorKind::Persistence));
            }
            regular(&path)?;
            let mut file = File::open(path).map_err(persistence)?;
            let length = file.metadata().map_err(persistence)?.len();
            if length > policy::SEGMENT_BYTES {
                return Err(failure(ErrorKind::Persistence));
            }
            while file.stream_position().map_err(persistence)? < length {
                let frame: Frame = read_frame(&mut file).map_err(persistence)?;
                self.validate_frame(&frame, segment)?;
                self.next_record = self
                    .next_record
                    .checked_add(1)
                    .ok_or_else(|| failure(ErrorKind::Persistence))?;
            }
            self.segment = segment;
            self.length = length;
        }
        if missing && !segment_path(&self.directory, 0).exists() {
            return Err(failure(ErrorKind::Persistence));
        }
        Ok(())
    }

    fn validate_frame(&self, frame: &Frame, segment: u32) -> Result<()> {
        let a = &frame.aad;
        if a.journal != self.identity.journal
            || a.key != self.identity.key
            || a.schema != 1
            || a.segment != segment
            || a.record != self.next_record
            || a.cipher_length as usize != frame.ciphertext.len()
            || a.plain_length.checked_add(16) != Some(a.cipher_length)
        {
            return Err(failure(ErrorKind::Persistence));
        }
        let aad = encode(a)?;
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new(self.key.bytes().into())
                .decrypt(
                    XNonce::from_slice(&frame.nonce),
                    Payload {
                        msg: &frame.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(persistence)?,
        );
        if plain.len() != a.plain_length as usize {
            return Err(failure(ErrorKind::Persistence));
        }
        Ok(())
    }

    pub(crate) fn check_capacity(&self) -> Result<()> {
        if self.pending.is_some() {
            return Err(failure(ErrorKind::Persistence));
        }
        let reserve = policy::FRAME_BYTES as u64 + 4;
        if policy::SEGMENT_BYTES - self.length < reserve && self.segment + 1 >= policy::SEGMENTS {
            return Err(failure(ErrorKind::JournalFull));
        }
        Ok(())
    }

    /// Reserve the maximum record, before the engine accepts user input.
    pub(crate) fn reserve(&mut self) -> Result<()> {
        self.check_capacity()?;
        if policy::SEGMENT_BYTES - self.length < policy::FRAME_BYTES as u64 + 4 {
            let next = self.segment + 1;
            let file = private_options()
                .write(true)
                .create_new(true)
                .open(segment_path(&self.directory, next))
                .map_err(persistence)?;
            file.sync_all().map_err(persistence)?;
            sync_directory(&self.directory)?;
            self.segment = next;
            self.length = 0;
        }
        Ok(())
    }

    pub(crate) fn append(&mut self, record: &FinalizedTurn) -> Result<()> {
        if self.pending.is_some() {
            return Err(failure(ErrorKind::Persistence));
        }
        self.pending = Some(
            self.prepare(record, random())
                .map_err(|error| error.with_pending_durable(false))?,
        );
        self.retry()
    }

    fn prepare(&self, record: &FinalizedTurn, nonce: Result<[u8; 24]>) -> Result<Pending> {
        let nonce = nonce?;
        let plain = encode(record)?;
        let plain_length = u32::try_from(plain.len()).map_err(persistence)?;
        let aad = Aad {
            journal: self.identity.journal,
            key: self.identity.key,
            schema: 1,
            segment: self.segment,
            record: self.next_record,
            plain_length,
            cipher_length: plain_length
                .checked_add(16)
                .ok_or_else(|| failure(ErrorKind::Persistence))?,
        };
        let aad_bytes = encode(&aad)?;
        let ciphertext = XChaCha20Poly1305::new(self.key.bytes().into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &aad_bytes,
                },
            )
            .map_err(persistence)?;
        let mut bytes = Zeroizing::new(Vec::new());
        write_frame(
            &mut *bytes,
            &Frame {
                aad,
                nonce,
                ciphertext,
            },
        )?;
        if self.length + bytes.len() as u64 > policy::SEGMENT_BYTES {
            return Err(failure(ErrorKind::JournalFull));
        }
        Ok(Pending {
            bytes,
            segment: self.segment,
            offset: self.length,
            durable: false,
        })
    }

    pub(crate) fn retry_record(&mut self, record: &FinalizedTurn) -> Result<()> {
        if self.pending.is_some() {
            self.retry()
        } else {
            self.append(record)
        }
    }

    /// Retry precisely the retained encrypted frame. Partial appends are replaced.
    pub(crate) fn retry(&mut self) -> Result<()> {
        self.persist_pending()?;
        self.flush_pending().map_err(|error| {
            error.with_pending_durable(self.pending.as_ref().is_some_and(|pending| pending.durable))
        })
    }
    fn persist_pending(&mut self) -> Result<()> {
        let pending = self
            .pending
            .as_mut()
            .ok_or_else(|| failure(ErrorKind::Persistence))?;
        if pending.durable && self.directory.join("pending.cbor").exists() {
            return Ok(());
        }
        let result = (|| {
            let mut file = private_options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(self.directory.join("pending.cbor"))
                .map_err(persistence)?;
            file.write_all(&pending.bytes)
                .and_then(|()| file.sync_all())
                .map_err(persistence)?;
            sync_directory(&self.directory)
        })();
        match result {
            Ok(()) => {
                pending.durable = true;
                Ok(())
            }
            Err(error) => Err(error.with_pending_durable(false)),
        }
    }
    fn flush_pending(&mut self) -> Result<()> {
        let Some(pending) = &self.pending else {
            return Err(failure(ErrorKind::Persistence));
        };
        let mut file = private_options()
            .write(true)
            .open(segment_path(&self.directory, pending.segment))
            .map_err(persistence)?;
        file.set_len(pending.offset).map_err(persistence)?;
        file.seek(SeekFrom::Start(pending.offset))
            .map_err(persistence)?;
        file.write_all(&pending.bytes)
            .and_then(|()| file.sync_all())
            .map_err(persistence)?;
        fs::remove_file(self.directory.join("pending.cbor")).map_err(persistence)?;
        sync_directory(&self.directory)?;
        self.length = pending.offset + pending.bytes.len() as u64;
        self.next_record = self
            .next_record
            .checked_add(1)
            .ok_or_else(|| failure(ErrorKind::Persistence))?;
        self.pending = None;
        Ok(())
    }

    /// Delete only this authenticated, exclusively locked journal after key deletion.
    pub fn delete(self) -> Result<()> {
        for segment in 0..policy::SEGMENTS {
            let path = segment_path(&self.directory, segment);
            if path.exists() {
                fs::remove_file(path).map_err(persistence)?;
            }
        }
        if self.directory.join("pending.cbor").exists() {
            fs::remove_file(self.directory.join("pending.cbor")).map_err(persistence)?;
        }
        fs::remove_file(self.directory.join("header.cbor")).map_err(persistence)?;
        fs::remove_file(self.directory.join("writer.lock")).map_err(persistence)?;
        fs::remove_dir(&self.directory).map_err(persistence)
    }
}
