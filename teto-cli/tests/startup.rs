#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn missing_model_exits_with_an_error() {
    let path = std::env::temp_dir().join(format!("teto-cli-missing-model-{}", std::process::id()));
    assert!(!path.exists(), "the missing-model fixture must not exist");

    let output = Command::new(env!("CARGO_BIN_EXE_teto-cli"))
        .env("TETO_MODE", "transcribe")
        .env("TETO_WHISPER_MODEL", &path)
        .output()
        .expect("the built CLI must start");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).expect("the startup error must be UTF-8");
    assert!(error.contains("cannot load Whisper model"));
    assert!(error.contains(&path.display().to_string()));
}

#[test]
fn invalid_mode_does_not_load_asr() {
    let output = Command::new(env!("CARGO_BIN_EXE_teto-cli"))
        .env("TETO_MODE", "invalid")
        .env_remove("TETO_WHISPER_MODEL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Whisper"));
}

#[test]
fn text_mode_requires_no_whisper_configuration() {
    let output = Command::new(env!("CARGO_BIN_EXE_teto-cli"))
        .env("TETO_MODE", "text")
        .env("TETO_LLM_ASSETS", "/unused")
        .env("TETO_JOURNAL", "off")
        .env("TETO_LLM_WORKER", "/does/not/exist/teto-worker")
        .env_remove("TETO_WHISPER_MODEL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Worker"));
    assert!(!stderr.contains("Whisper"));
}
