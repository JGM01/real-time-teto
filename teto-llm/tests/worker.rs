//! Real private worker protocol tests; no native model is needed for journal setup.
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use teto_llm::*;

struct Worker {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
}
impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_teto-llm-worker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: child.stdout.take().unwrap(),
            child,
        }
    }
    fn call(&mut self, request: Request) -> Response {
        write_frame(&mut self.input, &request).unwrap();
        read_frame(&mut self.output).unwrap()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn oversize_config_prevents_native_load() {
    let mut worker = Worker::spawn();
    let config = OpenConfig {
        assets: "unused".into(),
        profile: ProfileId::desktop(),
        output_language: OutputLanguage::English,
        system_prompt: "a".repeat(policy::SYSTEM_BYTES + 1),
        seed: 0,
        journal: JournalAccess::Disabled,
    };
    assert!(
        matches!(worker.call(Request::Open(config)), Response::Error(error) if error.kind()==ErrorKind::Config)
    );
}
#[test]
fn key_required_by_library() {
    let mut worker = Worker::spawn();
    let invalid = serde_json::json!({"Open": {"assets": "/unused", "profile": policy::PROFILE_ID,
        "output_language":"en", "system_prompt":"", "seed":0,
        "journal":{"Encrypted":{"directory":"/unused"}}}});
    write_frame(&mut worker.input, &invalid).unwrap();
    assert!(
        matches!(read_frame::<Response>(&mut worker.output).unwrap(), Response::Error(error) if error.kind()==ErrorKind::Protocol)
    );
}
#[test]
fn explicit_no_journal_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut worker = Worker::spawn();
    let config = OpenConfig {
        assets: root.path().into(),
        profile: ProfileId::desktop(),
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 0,
        journal: JournalAccess::Disabled,
    };
    assert!(
        matches!(worker.call(Request::Open(config)), Response::Error(error) if error.kind()==ErrorKind::Profile)
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn worker_journal_init_inspect_delete() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let identity = JournalIdentity {
        journal: [1; 16],
        key: [2; 16],
    };
    let mut worker = Worker::spawn();
    assert!(matches!(
        worker.call(Request::Initialize {
            directory: directory.clone(),
            identity,
            key: SecretKey::new([7; 32])
        }),
        Response::Ready
    ));
    assert!(worker.child.wait().unwrap().success());
    let mut worker = Worker::spawn();
    assert!(
        matches!(worker.call(Request::Inspect(directory.clone())), Response::Identity(value) if value == identity)
    );
    assert!(worker.child.wait().unwrap().success());
    let mut worker = Worker::spawn();
    assert!(matches!(
        worker.call(Request::PrepareDelete {
            directory: directory.clone(),
            key: SecretKey::new([7; 32])
        }),
        Response::Ready
    ));
    assert!(matches!(worker.call(Request::Delete), Response::Closed));
    assert!(worker.child.wait().unwrap().success());
    assert!(!directory.exists());
}
#[test]
fn fragmented_frames_and_child_crash() {
    let mut worker = Worker::spawn();
    let mut frame = Vec::new();
    write_frame(&mut frame, &Request::Close).unwrap();
    for byte in frame {
        worker.input.write_all(&[byte]).unwrap();
    }
    assert!(
        matches!(read_frame::<Response>(&mut worker.output).unwrap(), Response::Error(error) if error.kind()==ErrorKind::Protocol)
    );
    let mut worker = Worker::spawn();
    worker.child.kill().unwrap();
    worker.child.wait().unwrap();
    let mut buffer = [0];
    assert_eq!(worker.output.read(&mut buffer).unwrap(), 0);
}
#[test]
fn oversized_worker_frame_rejected() {
    let mut worker = Worker::spawn();
    worker
        .input
        .write_all(&(policy::FRAME_BYTES as u32 + 1).to_be_bytes())
        .unwrap();
    assert!(
        matches!(read_frame::<Response>(&mut worker.output).unwrap(), Response::Error(error) if error.kind()==ErrorKind::Protocol)
    );
}
#[test]
fn native_and_audio_isolation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let output = Command::new("cargo")
        .args(["tree", "--offline", "-p", "teto-cli", "--prefix", "none"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let parent = String::from_utf8(output.stdout).unwrap();
    assert!(!parent.contains("llama-cpp"));
    let output = Command::new("cargo")
        .args(["tree", "--offline", "-p", "teto-llm", "--prefix", "none"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let child = String::from_utf8(output.stdout).unwrap();
    assert!(!child.contains("whisper"));
    assert!(!child.contains("earshot"));
    assert!(!child.contains("cpal"));
}
