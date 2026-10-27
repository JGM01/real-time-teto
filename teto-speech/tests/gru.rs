#![cfg(feature = "g2p-gru")]
use teto_speech::{error::SpeechError, text::gru::G2pEnGru};
#[test]
#[ignore = "requires pinned checkpoint and independent NumPy oracle"]
fn gru_matches_numpy_tokens() {
    let checkpoint = std::env::var_os("TETO_G2P_CHECKPOINT").unwrap();
    let oracle = std::env::var_os("TETO_G2P_ORACLE").unwrap();
    let mut gru = G2pEnGru::load(std::path::Path::new(&checkpoint)).unwrap();
    let oracle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(oracle).unwrap()).unwrap();
    for record in oracle["records"].as_array().unwrap() {
        let word = record["word"].as_str().unwrap();
        let result = gru.decode(word);
        if !record["eos"].as_bool().unwrap() {
            assert!(matches!(result, Err(SpeechError::DecoderLimit)), "{word}");
            continue;
        }
        let expected: Vec<u8> = record["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_u64().unwrap() as u8)
            .collect();
        if expected.is_empty() || expected.iter().any(|t| *t < 4) {
            assert!(result.is_err(), "{word}");
            continue;
        }
        assert_eq!(result.unwrap(), expected, "{word}");
    }
    assert!(gru.decode(&"a".repeat(65)).is_err());
    assert!(gru.decode("").is_err());
    assert!(gru.decode("é").is_err());
}

#[test]
fn gru_rejects_unverified_and_oversized_assets() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"invalid").unwrap();
    assert!(matches!(
        G2pEnGru::load(file.path()),
        Err(SpeechError::ModelMismatch)
    ));
    file.as_file()
        .set_len(teto_speech::text::limits::MAX_MODEL_BYTES + 1)
        .unwrap();
    assert!(matches!(
        G2pEnGru::load(file.path()),
        Err(SpeechError::LimitExceeded)
    ));
}
