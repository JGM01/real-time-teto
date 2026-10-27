#![no_main]
use libfuzzer_sys::fuzz_target;
use teto_speech::{
    diagnostics::SpeechDiagnostics,
    text::{
        lexicon::Lexicon,
        phonemize::{CmudictPhonemizer, OovBackend},
    },
    types::*,
    voicebank::select::TetoCvvcPlanner,
};
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 4096 {
        return;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let mut lines = text.lines();
    let Some(pronunciation) = lines.next() else {
        return;
    };
    let aliases: Vec<_> = lines.take(64).collect();
    let Ok(bank) = teto_speech::testing::manifest(&aliases) else {
        return;
    };
    let dictionary = format!("test {pronunciation}");
    let Ok(lexicon) = Lexicon::parse(dictionary.as_bytes()) else {
        return;
    };
    let mut frontend =
        CmudictPhonemizer::new(lexicon, Lexicon::parse(b"").unwrap(), OovBackend::Disabled);
    let input = SpeechInput::new(SpeechKey::new(0, 1, 0).unwrap(), "test").unwrap();
    let mut diagnostics = SpeechDiagnostics::default();
    let Ok(sequence) = frontend.phonemize(&input, &mut diagnostics) else {
        return;
    };
    if bytes.len() >= 2
        && let Ok(span) = IndexSpan::new(bytes[0] as u32, bytes[1] as u32)
    {
        let mut words = sequence.words().to_vec();
        words[0].phones = span;
        let _ = PhonemeSequence::new(
            input.clone(),
            sequence.phones().to_vec(),
            words,
            sequence.syllables().to_vec(),
            sequence.breaks().to_vec(),
            [0; 32],
        );
    }
    let timings = sequence
        .phones()
        .iter()
        .enumerate()
        .map(|(i, _)| PhoneTiming {
            target_phone: i as u32,
            start_us: i as u64 * 100000,
            end_us: (i as u64 + 1) * 100000,
        })
        .collect();
    let prosody = ProsodyPlan::new(
        &sequence,
        sequence.phones().len() as u64 * 100000,
        timings,
        vec![],
    )
    .unwrap();
    if let Ok(plan) = TetoCvvcPlanner::default().plan(&sequence, prosody, &bank, &mut diagnostics) {
        plan.validate(&sequence, &bank).unwrap();
    }
});
