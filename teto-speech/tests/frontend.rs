#![cfg(feature = "cmudict")]
use teto_speech::{
    diagnostics::SpeechDiagnostics,
    text::{
        lexicon::Lexicon,
        phonemize::{CmudictPhonemizer, OovBackend},
    },
    types::*,
};
fn frontend(data: &[u8]) -> CmudictPhonemizer {
    CmudictPhonemizer::new(
        Lexicon::parse(data).unwrap(),
        Lexicon::parse(b"").unwrap(),
        OovBackend::Disabled,
    )
}
fn input(text: &str) -> SpeechInput {
    SpeechInput::new(SpeechKey::new(0, 1, 0).unwrap(), text).unwrap()
}
#[test]
fn lexicon_preserves_alternatives_and_override_precedence() {
    let data = b"read R EH1 D # present\nread(2) R IY1 D\nread(3) R IY1 D\n";
    let lexicon = Lexicon::parse(data).unwrap();
    assert_eq!(lexicon.lookup("read").unwrap().1, 2);
    let mut f = CmudictPhonemizer::new(
        lexicon,
        Lexicon::parse(b"read R IY1 D\n").unwrap(),
        OovBackend::Disabled,
    );
    let sequence = f
        .phonemize(&input("Read."), &mut SpeechDiagnostics::default())
        .unwrap();
    assert_eq!(sequence.words()[0].source, PronunciationSource::Override);
    assert_eq!(sequence.phones()[1].phone(), ArpaPhone::Iy);
    assert!(Lexicon::parse(b"read R EH1 D\nread R IY1 D\n").is_err());
    assert!(Lexicon::parse(b"bad B1 AE1 D\n").is_err());
}
#[test]
fn frontend_preserves_utf8_word_spans_and_marks_ambiguity() {
    let mut f = frontend(b"can't K AE1 N T\nread R EH1 D\nread(2) R IY1 D\n");
    let input = input("Can’t read?");
    let mut diagnostics = SpeechDiagnostics::default();
    let sequence = f.phonemize(&input, &mut diagnostics).unwrap();
    sequence.validate().unwrap();
    assert_eq!(&input.text()[sequence.words()[0].text.range()], "Can’t");
    assert_eq!(sequence.words()[1].status, PronunciationStatus::Ambiguous);
    assert_eq!(sequence.breaks().last().unwrap().kind, BreakKind::Question);
    assert_eq!(diagnostics.counts()[1], 1);
}
#[test]
fn frontend_rejects_unsupported_text_without_expansion() {
    let mut f = frontend(b"hello HH AH0 L OW1\n");
    for text in ["hello 12", "hello 😊", "hello café"] {
        assert!(
            f.phonemize(&input(text), &mut SpeechDiagnostics::default())
                .is_err()
        );
    }
    assert!(
        f.phonemize(&input("unknown"), &mut SpeechDiagnostics::default())
            .is_err()
    );
}
#[test]
fn embedded_dictionary_and_cluster_syllables_validate() {
    let mut f = CmudictPhonemizer::embedded(OovBackend::Disabled).unwrap();
    for text in [
        "strengths",
        "judgment",
        "church",
        "singing",
        "through",
        "vision",
        "world",
        "yellow",
        "we'll",
        "beautiful",
        "syllable",
        "rhythm",
    ] {
        let sequence = f
            .phonemize(&input(text), &mut SpeechDiagnostics::default())
            .unwrap();
        sequence.validate().unwrap();
        assert_eq!(
            sequence
                .syllables()
                .iter()
                .map(|s| s.phones.len())
                .sum::<u32>(),
            sequence.phones().len() as u32
        );
    }
}
proptest::proptest! {
    #[test]
    fn syllable_partition_preserves_every_phone(n in 1usize..22, selector in proptest::collection::vec(0usize..5,25)) {
        let mut data=String::from("test");
        for choice in selector.iter().take(n) {data.push_str([" B AA1 T"," NG IY0"," TH AW2 S"," CH ER0 L"," ZH EH0 W"][*choice]);}
        let mut f=frontend(data.as_bytes());let s=f.phonemize(&input("test"),&mut SpeechDiagnostics::default()).unwrap();
        proptest::prop_assert!(s.validate().is_ok());
        proptest::prop_assert_eq!(s.syllables().iter().map(|s|s.phones.len()).sum::<u32>(),s.phones().len() as u32);
    }
}
