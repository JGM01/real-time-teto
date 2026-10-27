#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 4096 {
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "teto-fuzz-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::create_dir_all(&root).is_err() {
        return;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    if let Ok(mut writer) = hound::WavWriter::create(root.join("unit.wav"), spec) {
        for _ in 0..4410 {
            let _ = writer.write_sample(0i16);
        }
        let _ = writer.finalize();
    }
    if std::fs::write(root.join("oto.ini"), bytes).is_ok() {
        let _ = teto_speech::voicebank::oto::import_voicebank(&root);
    }
    let _ = std::fs::remove_dir_all(root);
});
