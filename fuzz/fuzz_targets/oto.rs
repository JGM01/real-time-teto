#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _ = teto_speech::voicebank::oto::visit_rows(bytes, |_, _| Ok(()));
});
