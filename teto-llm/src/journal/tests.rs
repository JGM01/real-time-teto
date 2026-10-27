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
        partial: generation_stop != GenerationStop::Eos || playback_stop != PlaybackStop::Complete,
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
fn existing_journal_missing_key() {
    journal_open_failure();
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
fn startup_never_creates_or_replaces_key() {
    explicit_initialization_only();
    existing_key_missing_journal();
}
#[test]
fn journal_delete_and_key_loss() {
    let (_root, dir, key) = setup();
    let journal = Journal::open_for_delete(dir.clone(), key).unwrap();
    journal.delete().unwrap();
    assert!(!dir.exists());
}
#[test]
fn per_turn_delete_unsupported() {
    assert!(!include_str!("../journal.rs").contains("fn delete_turn"));
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
    assert_eq!(failed.pending_durable(), Some(true));
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
fn failure_never_disables_journal() {
    write_failure_stops_turns();
}
#[test]
fn no_plaintext_fallback() {
    journal_aead_roundtrip_and_tamper();
}
#[test]
fn journal_not_replayed() {
    let (_root, dir, key) = setup();
    let mut journal = Journal::open(dir.clone(), key).unwrap();
    append_one(&mut journal);
    drop(journal);
    assert_eq!(
        Journal::open(dir, SecretKey::new([17; 32]))
            .unwrap()
            .next_record,
        2
    );
    assert!(!include_str!("../native.rs").contains("journal.history"));
}
#[test]
fn token_limit_journal_record() {
    let record = record(GenerationStop::TokenLimit, PlaybackStop::Interrupted);
    let saved: FinalizedTurn = ciborium::from_reader(encode(&record).unwrap().as_slice()).unwrap();
    assert_eq!(saved.generation_stop, GenerationStop::TokenLimit);
    assert_eq!(saved.playback_stop, PlaybackStop::Interrupted);
    assert!(saved.partial);
}
#[test]
fn interrupted_journal_record() {
    token_limit_journal_record();
}
#[test]
fn diagnostics_are_not_journal() {
    let saved = record(GenerationStop::Eos, PlaybackStop::Complete);
    assert!(!format!("{saved:?}").contains("SENTINEL"));
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
