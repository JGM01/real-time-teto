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
            || a.schema != 2
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
        let pending = self
            .prepare(record, random())
            .map_err(|error| error.with_pending_durable(false))?;
        self.pending = Some(pending);
        self.retry()
    }

    fn prepare(&self, record: &FinalizedTurn, nonce: Result<[u8; 24]>) -> Result<Pending> {
        let nonce = nonce?;
        let plain = encode(record)?;
        if plain.len() > policy::RECORD_BYTES {
            return Err(failure(ErrorKind::Persistence));
        }
        let plain_length = u32::try_from(plain.len()).map_err(persistence)?;
        let aad = Aad {
            journal: self.identity.journal,
            key: self.identity.key,
            schema: 2,
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
        })
    }

    pub(crate) fn retry_record(&mut self, record: &FinalizedTurn) -> Result<()> {
        if self.pending.is_some() {
            self.retry()
        } else {
            self.append(record)
        }
    }

    /// Retry the retained encrypted frame. A failed segment write spills it.
    pub(crate) fn retry(&mut self) -> Result<()> {
        match self.flush_pending() {
            Ok(()) => Ok(()),
            Err(error) => {
                let durable = self.spill_pending().is_ok();
                Err(error.with_pending_durable(durable))
            }
        }
    }

    fn flush_pending(&mut self) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Err(failure(ErrorKind::Persistence));
        };
        let result = (|| {
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
            sync_directory(&self.directory)
        })();
        match result {
            Ok(()) => {
                self.length = pending.offset + pending.bytes.len() as u64;
                self.next_record = self
                    .next_record
                    .checked_add(1)
                    .ok_or_else(|| failure(ErrorKind::Persistence))?;
                let pending_path = self.directory.join("pending.cbor");
                if pending_path.exists() {
                    let _ = fs::remove_file(&pending_path);
                    let _ = sync_directory(&self.directory);
                }
                Ok(())
            }
            Err(error) => {
                self.pending = Some(pending);
                Err(error)
            }
        }
    }

    fn spill_pending(&mut self) -> Result<()> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| failure(ErrorKind::Persistence))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn setup() -> (tempfile::TempDir, PathBuf, SecretKey) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("journal");
        let key = SecretKey::new([17; 32]);
        Journal::initialize(
            &dir,
            Identity {
                journal: [1; 16],
                key: [2; 16],
            },
            &key,
        )
        .unwrap();
        (root, dir, key)
    }
    fn record(generation_stop: GenerationStop, playback_stop: PlaybackStop) -> FinalizedTurn {
        FinalizedTurn {
            schema_version: 1,
            policy_version: policy::VERSION,
            session: [9; 16],
            turn: TurnId::new(1).unwrap(),
            profile: policy::PROFILE_ID.into(),
            user: UserInput::new("USER_SENTINEL".into(), true).unwrap(),
            assistant: vec!["PLAYED_SENTINEL".into()],
            generation_stop,
            playback_stop,
            partial: generation_stop != GenerationStop::Eos
                || playback_stop != PlaybackStop::Complete,
            receipt_source: ReceiptSource::Display,
            metrics: TurnMetrics::default(),
        }
    }
    fn append_one(journal: &mut Journal) {
        journal.reserve().unwrap();
        journal
            .append(&record(GenerationStop::Eos, PlaybackStop::Complete))
            .unwrap();
    }
    #[test]
    fn journal_aead_roundtrip_and_tamper() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        append_one(&mut journal);
        drop(journal);
        let path = segment_path(&dir, 0);
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.windows(13).any(|w| w == b"USER_SENTINEL"));
        assert!(!bytes.windows(15).any(|w| w == b"PLAYED_SENTINEL"));
        drop(Journal::open(dir.clone(), SecretKey::new([17; 32])).unwrap());
        let mut frame: Frame = read_frame(&mut bytes.as_slice()).unwrap();
        frame.ciphertext[0] ^= 1;
        let mut modified = Vec::new();
        write_frame(&mut modified, &frame).unwrap();
        fs::write(path, modified).unwrap();
        assert!(Journal::open(dir, SecretKey::new([17; 32])).is_err());
    }
    #[test]
    fn journal_contains_user_and_played_prefix() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        append_one(&mut journal);
        let frame: Frame = read_frame(&mut File::open(segment_path(&dir, 0)).unwrap()).unwrap();
        let plain = XChaCha20Poly1305::new(journal.key.bytes().into())
            .decrypt(
                XNonce::from_slice(&frame.nonce),
                Payload {
                    msg: &frame.ciphertext,
                    aad: &encode(&frame.aad).unwrap(),
                },
            )
            .unwrap();
        let saved: FinalizedTurn = ciborium::from_reader(plain.as_slice()).unwrap();
        assert_eq!(saved.user.text(), "USER_SENTINEL");
        assert_eq!(saved.assistant, ["PLAYED_SENTINEL"]);
        assert!(saved.user.truncated());
    }
    #[test]
    fn nonce_frames_rng_failure_and_retry() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        assert!(
            journal
                .prepare(
                    &record(GenerationStop::Eos, PlaybackStop::Complete),
                    Err(failure(ErrorKind::Persistence))
                )
                .is_err()
        );
        assert_eq!(journal.next_record, 1);
        assert!(journal.pending.is_none());
        append_one(&mut journal);
        append_one(&mut journal);
        let mut file = File::open(segment_path(&dir, 0)).unwrap();
        let a: Frame = read_frame(&mut file).unwrap();
        let b: Frame = read_frame(&mut file).unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_eq!(a.nonce.len(), 24);
        assert_eq!(a.aad.record, 1);
        assert_eq!(b.aad.record, 2);
        assert!(journal.retry().is_err());
    }
    #[test]
    fn second_writer_rejected() {
        let (_root, dir, key) = setup();
        let _journal = Journal::open(dir.clone(), key).unwrap();
        assert!(Journal::open(dir, SecretKey::new([17; 32])).is_err());
    }
    #[test]
    fn journal_open_failure() {
        let (_root, dir, _) = setup();
        assert!(Journal::open(dir, SecretKey::new([18; 32])).is_err());
    }
    #[test]
    fn existing_key_missing_journal() {
        let root = tempfile::tempdir().unwrap();
        assert!(Journal::open(root.path().join("missing"), SecretKey::new([17; 32])).is_err());
    }
    #[test]
    fn explicit_initialization_only() {
        let (_root, dir, key) = setup();
        let old = fs::read(dir.join("header.cbor")).unwrap();
        assert!(
            Journal::initialize(
                &dir,
                Identity {
                    journal: [3; 16],
                    key: [4; 16]
                },
                &key
            )
            .is_err()
        );
        assert_eq!(fs::read(dir.join("header.cbor")).unwrap(), old);
    }
    #[test]
    fn journal_delete_and_key_loss() {
        let (_root, dir, key) = setup();
        let journal = Journal::open_for_delete(dir.clone(), key).unwrap();
        journal.delete().unwrap();
        assert!(!dir.exists());
    }
    #[test]
    fn journal_capacity_preserves_records() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir, key).unwrap();
        journal.segment = policy::SEGMENTS - 1;
        journal.length = policy::SEGMENT_BYTES;
        assert_eq!(
            journal.reserve().unwrap_err().kind(),
            ErrorKind::JournalFull
        );
        assert!(journal.directory.join("header.cbor").exists());
    }
    #[test]
    #[ignore = "writes sixty-four megabytes with durable appends"]
    fn journal_full_startup_and_runtime() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        let mut value = record(GenerationStop::Eos, PlaybackStop::Complete);
        value.user = UserInput::new("u".repeat(policy::INPUT_BYTES), false).unwrap();
        value.assistant = vec!["a".repeat(128); policy::TURN_CLAUSES];
        value.assistant[0] = "a".repeat(policy::FIRST_CHARS);
        let mut full = false;
        for _ in 0..1024 {
            match journal.reserve() {
                Ok(()) => journal.append(&value).unwrap(),
                Err(error) => {
                    assert_eq!(error.kind(), ErrorKind::JournalFull);
                    full = true;
                    break;
                }
            }
        }
        assert!(full, "bounded fixture must fill all eight segments");
        let before: Vec<_> = (0..policy::SEGMENTS)
            .map(|index| fs::read(segment_path(&dir, index)).unwrap())
            .collect();
        assert!(
            before
                .iter()
                .all(|bytes| bytes.len() as u64 <= policy::SEGMENT_BYTES)
        );
        drop(journal);
        assert_eq!(
            Journal::open(dir.clone(), SecretKey::new([17; 32]))
                .err()
                .unwrap()
                .kind(),
            ErrorKind::JournalFull
        );
        for (index, bytes) in before.iter().enumerate() {
            assert_eq!(&fs::read(segment_path(&dir, index as u32)).unwrap(), bytes);
        }
    }
    #[test]
    fn write_failure_stops_turns() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        journal.reserve().unwrap();
        fs::rename(segment_path(&dir, 0), dir.join("temporary")).unwrap();
        let failed = journal
            .append(&record(GenerationStop::Eos, PlaybackStop::Complete))
            .unwrap_err();
        assert_eq!(failed.pending_durability(), PendingDurability::Durable);
        assert!(dir.join("pending.cbor").is_file());
        let pending = journal.pending.as_ref().unwrap().bytes.to_vec();
        assert!(journal.reserve().is_err());
        fs::rename(dir.join("temporary"), segment_path(&dir, 0)).unwrap();
        journal.retry().unwrap();
        assert!(!dir.join("pending.cbor").exists());
        assert_eq!(fs::read(segment_path(&dir, 0)).unwrap(), pending);
        assert!(journal.pending.is_none());
        assert_eq!(journal.next_record, 2);
    }
    #[test]
    fn token_limit_journal_record() {
        let record = record(GenerationStop::TokenLimit, PlaybackStop::Interrupted);
        let saved: FinalizedTurn =
            ciborium::from_reader(encode(&record).unwrap().as_slice()).unwrap();
        assert_eq!(saved.generation_stop, GenerationStop::TokenLimit);
        assert_eq!(saved.playback_stop, PlaybackStop::Interrupted);
        assert!(saved.partial);
    }
    #[test]
    fn truncated_frame_fails_closed() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        append_one(&mut journal);
        drop(journal);
        let path = segment_path(&dir, 0);
        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();
        bytes.pop();
        fs::write(path, bytes).unwrap();
        assert!(Journal::open(dir, SecretKey::new([17; 32])).is_err());
    }

    #[test]
    fn earlier_byte_array_frames_remain_readable() {
        #[derive(Serialize)]
        struct EarlierFrame<'a> {
            aad: &'a Aad,
            nonce: [u8; 24],
            ciphertext: &'a [u8],
        }
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        append_one(&mut journal);
        drop(journal);
        let path = segment_path(&dir, 0);
        let frame: Frame = read_frame(&mut File::open(&path).unwrap()).unwrap();
        let old = EarlierFrame {
            aad: &frame.aad,
            nonce: frame.nonce,
            ciphertext: &frame.ciphertext,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &old).unwrap();
        fs::write(path, bytes).unwrap();
        drop(Journal::open(dir, SecretKey::new([17; 32])).unwrap());
    }

    #[test]
    fn every_truncated_segment_prefix_fails_closed() {
        let (_root, dir, key) = setup();
        let mut journal = Journal::open(dir.clone(), key).unwrap();
        append_one(&mut journal);
        drop(journal);
        let path = segment_path(&dir, 0);
        let bytes = fs::read(&path).unwrap();
        for cut in 1..bytes.len() {
            fs::write(&path, &bytes[..cut]).unwrap();
            assert!(
                Journal::open(dir.clone(), SecretKey::new([17; 32])).is_err(),
                "cut {cut}"
            );
        }
        fs::write(&path, &bytes).unwrap();
        drop(Journal::open(dir, SecretKey::new([17; 32])).unwrap());
    }
}
