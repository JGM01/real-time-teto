use teto_speech::{
    diagnostics::{Diagnostic, SpeechDiagnostics},
    error::SpeechError,
    limits::*,
    types::*,
};
fn input() -> SpeechInput {
    SpeechInput::new(SpeechKey::new(0, 1, 0).unwrap(), "a").unwrap()
}
fn sequence() -> PhonemeSequence {
    PhonemeSequence::new(
        input(),
        vec![EnglishPhone::parse("AH1").unwrap()],
        vec![LexicalWord {
            text: ByteSpan::new(0, 1).unwrap(),
            phones: IndexSpan::new(0, 1).unwrap(),
            syllables: IndexSpan::new(0, 1).unwrap(),
            part_of_speech: PartOfSpeech::Unknown,
            source: PronunciationSource::Lexicon,
            status: PronunciationStatus::Selected,
        }],
        vec![Syllable {
            phones: IndexSpan::new(0, 1).unwrap(),
            onset: IndexSpan::new(0, 0).unwrap(),
            nucleus: IndexSpan::new(0, 1).unwrap(),
            coda: IndexSpan::new(1, 1).unwrap(),
            stress: Stress::Primary,
        }],
        vec![],
        [0; 32],
    )
    .unwrap()
}
#[test]
fn speech_input_rejects_empty_controls_and_oversized() {
    for text in ["", "   ", "hi\nthere", "hi\tthere", "hi\0"] {
        assert!(SpeechInput::new(input().key(), text).is_err());
    }
    assert!(SpeechInput::new(input().key(), "a".repeat(MAX_INPUT_SCALARS)).is_ok());
    assert!(SpeechInput::new(input().key(), "a".repeat(MAX_INPUT_SCALARS + 1)).is_err());
    assert!(SpeechKey::new(0, 0, 0).is_err());
    assert!(SpeechKey::new(0, 1, MAX_CLAUSES).is_err());
}
#[test]
fn speech_debug_redacts_text_and_diagnostics() {
    let input = SpeechInput::new(input().key(), "private sentinel phrase").unwrap();
    assert!(!format!("{input:?}").contains("sentinel"));
    let mut diagnostics = SpeechDiagnostics::default();
    for _ in 0..MAX_DIAGNOSTICS + 17 {
        diagnostics.record(Diagnostic::Oov {
            word: ByteSpan::new(0, 7).unwrap(),
        });
    }
    assert_eq!(diagnostics.events().len(), MAX_DIAGNOSTICS);
    assert_eq!(diagnostics.counts()[0], (MAX_DIAGNOSTICS + 17) as u64);
    assert!(!format!("{diagnostics:?}").contains("sentinel"));
}
#[test]
fn lexical_stress_rejects_invalid_symbols() {
    for token in ["B1", "AA3", "XX", "", "あ"] {
        assert!(EnglishPhone::parse(token).is_err());
    }
    assert_eq!(
        EnglishPhone::parse("UW").unwrap().stress(),
        Some(Stress::Unknown)
    );
    assert!(EnglishPhone::new(ArpaPhone::B, Some(Stress::Primary)).is_err());
    assert!(EnglishPhone::new(ArpaPhone::Aa, None).is_err());
}
#[test]
fn prosody_rejects_missing_duplicate_and_reversed_timings() {
    let sequence = sequence();
    assert!(ProsodyPlan::new(&sequence, 100, vec![], vec![]).is_err());
    for (target_phone, start_us, end_us) in [(1, 0, 100), (0, 100, 99), (0, 0, 101)] {
        assert!(
            ProsodyPlan::new(
                &sequence,
                100,
                vec![PhoneTiming {
                    target_phone,
                    start_us,
                    end_us
                }],
                vec![]
            )
            .is_err()
        );
    }
    let timing = PhoneTiming {
        target_phone: 0,
        start_us: 0,
        end_us: 100,
    };
    assert!(
        ProsodyPlan::new(
            &sequence,
            100,
            vec![timing],
            vec![ProsodyPoint {
                at_us: 0,
                f0_millihz: Some(0),
                gain_q16: 65536
            }]
        )
        .is_err()
    );
    assert!(ProsodyPlan::new(&sequence, 100, vec![timing], vec![]).is_ok());
}
#[test]
fn phoneme_partitions_reject_orphan_and_invalid_byte_spans() {
    let sequence = sequence();
    let word = sequence.words()[0].clone();
    let syllable = sequence.syllables()[0];
    for text in [ByteSpan::new(0, 2).unwrap(), ByteSpan::new(1, 1).unwrap()] {
        let mut word = word.clone();
        word.text = text;
        assert!(
            PhonemeSequence::new(
                input(),
                sequence.phones().to_vec(),
                vec![word],
                vec![syllable],
                vec![],
                [0; 32]
            )
            .is_err()
        );
    }
    let mut word = word;
    word.phones = IndexSpan::new(0, 0).unwrap();
    assert!(
        PhonemeSequence::new(
            input(),
            sequence.phones().to_vec(),
            vec![word],
            vec![syllable],
            vec![],
            [0; 32]
        )
        .is_err()
    );
    assert!(matches!(IndexSpan::new(2, 1), Err(SpeechError::Bounds)));
}
proptest::proptest! {
    #[test]
    fn oto_decimal_matches_integer_microseconds(whole in 0i64..1_000_000, fraction in 0i64..1000,negative in proptest::bool::ANY) {
        let text=format!("{}{whole}.{fraction:03}",if negative {"-"}else{""});
        let expected=(whole*1000+fraction)*if negative {-1}else{1};
        proptest::prop_assert_eq!(teto_speech::voicebank::oto::decimal_us(&text).unwrap(),expected);
    }
}
