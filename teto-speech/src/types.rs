use std::{fmt, num::NonZeroU64, sync::Arc};

use crate::error::{Result, SpeechError};

/// Maximum input Unicode scalar values in one clause.
pub const MAX_INPUT_SCALARS: usize = 240;
/// Maximum phones in one clause.
pub const MAX_PHONES: usize = 512;
/// Maximum words in one clause.
pub const MAX_WORDS: usize = 64;
/// Maximum syllables in one clause.
pub const MAX_SYLLABLES: usize = 256;
/// Maximum breaks in one clause.
pub const MAX_BREAKS: usize = 64;
/// Maximum clause index.
pub const MAX_CLAUSES: u32 = 512;

/// Identity of one speech item across every stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpeechKey {
    /// Conversation or process generation.
    pub epoch: u64,
    /// Turn identifier.
    pub turn: NonZeroU64,
    /// Clause index inside the turn.
    pub clause: u32,
}

impl SpeechKey {
    /// Create a key.
    pub fn new(epoch: u64, turn: u64, clause: u32) -> Result<Self> {
        let turn = NonZeroU64::new(turn).ok_or(SpeechError::InvalidInput)?;
        if clause >= MAX_CLAUSES {
            return Err(SpeechError::Bounds);
        }
        Ok(Self {
            epoch,
            turn,
            clause,
        })
    }
}

/// One bounded English clause.
#[derive(Clone)]
pub struct SpeechInput {
    key: SpeechKey,
    text: Arc<str>,
}

impl SpeechInput {
    /// Create an input from normalized clause text.
    pub fn new(key: SpeechKey, text: impl Into<Arc<str>>) -> Result<Self> {
        let text = text.into();
        if text.is_empty() || text.chars().count() > MAX_INPUT_SCALARS || text.contains('\0') {
            return Err(SpeechError::InvalidInput);
        }
        Ok(Self { key, text })
    }

    /// Return the key.
    pub fn key(&self) -> SpeechKey {
        self.key
    }

    /// Return the text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for SpeechInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpeechInput")
            .field("key", &self.key)
            .field("text", &"[redacted]")
            .finish()
    }
}

/// One half-open byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan {
    /// First byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

impl ByteSpan {
    /// Create a span.
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start > end {
            return Err(SpeechError::Bounds);
        }
        Ok(Self { start, end })
    }
}

/// One half-open index span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexSpan {
    /// First index.
    pub start: u32,
    /// One past the last index.
    pub end: u32,
}

impl IndexSpan {
    /// Create a span.
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start > end {
            return Err(SpeechError::Bounds);
        }
        Ok(Self { start, end })
    }

    /// Return the span length.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Return true when the span is empty.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// ARPAbet phone symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArpaPhone {
    Aa,
    Ae,
    Ah,
    Ao,
    Aw,
    Ay,
    B,
    Ch,
    D,
    Dh,
    Eh,
    Er,
    Ey,
    F,
    G,
    Hh,
    Ih,
    Iy,
    Jh,
    K,
    L,
    M,
    N,
    Ng,
    Ow,
    Oy,
    P,
    R,
    S,
    Sh,
    T,
    Th,
    Uh,
    Uw,
    V,
    W,
    Y,
    Z,
    Zh,
}

/// Lexical stress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stress {
    Unstressed,
    Primary,
    Secondary,
    Unknown,
}

/// One English phone with optional stress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnglishPhone {
    pub phone: ArpaPhone,
    pub stress: Option<Stress>,
}

/// One syllable and its phone spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syllable {
    pub phones: IndexSpan,
    pub onset: IndexSpan,
    pub nucleus: IndexSpan,
    pub coda: IndexSpan,
    pub stress: Stress,
}

/// Part of speech used for homograph selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartOfSpeech {
    Noun,
    Verb,
    Adjective,
    Adverb,
    Other,
    Unknown,
}

/// Source of one pronunciation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PronunciationSource {
    Override,
    Lexicon,
    OovModel,
    Spelling,
}

/// Confidence of one pronunciation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PronunciationStatus {
    Selected,
    Ambiguous,
    Estimated,
}

/// One lexical word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexicalWord {
    pub text: ByteSpan,
    pub phones: IndexSpan,
    pub syllables: IndexSpan,
    pub part_of_speech: PartOfSpeech,
    pub source: PronunciationSource,
    pub status: PronunciationStatus,
}

/// A phrase or sentence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    Word,
    MinorPhrase,
    Sentence,
    Question,
    Exclamation,
}

/// A boundary after one phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBreak {
    pub after_phone: u32,
    pub kind: BreakKind,
}

/// English phones with word, syllable and break structure.
#[derive(Debug, Clone)]
pub struct PhonemeSequence {
    pub input: SpeechInput,
    pub phones: Box<[EnglishPhone]>,
    pub words: Box<[LexicalWord]>,
    pub syllables: Box<[Syllable]>,
    pub breaks: Box<[TextBreak]>,
    pub frontend_revision: [u8; 32],
}

impl PhonemeSequence {
    /// Validate all spans and bounds.
    pub fn validate(&self) -> Result<()> {
        if self.phones.len() > MAX_PHONES
            || self.words.len() > MAX_WORDS
            || self.syllables.len() > MAX_SYLLABLES
            || self.breaks.len() > MAX_BREAKS
        {
            return Err(SpeechError::Bounds);
        }
        let phones = self.phones.len() as u32;
        let syllables = self.syllables.len() as u32;
        for word in &self.words {
            check_span(word.phones, phones)?;
            check_span(word.syllables, syllables)?;
        }
        for syllable in &self.syllables {
            check_span(syllable.phones, phones)?;
            check_span(syllable.onset, phones)?;
            check_span(syllable.nucleus, phones)?;
            check_span(syllable.coda, phones)?;
        }
        for item in &self.breaks {
            if item.after_phone >= phones {
                return Err(SpeechError::Bounds);
            }
        }
        Ok(())
    }
}

fn check_span(span: IndexSpan, len: u32) -> Result<()> {
    if span.end > len {
        return Err(SpeechError::Bounds);
    }
    Ok(())
}

/// Voicebank alias inventory identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AliasInventoryId(pub [u8; 32]);

/// One alias identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AliasId(pub u32);

/// One recorded pitch group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PitchId(pub u32);

/// One voicebank unit identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitId(pub u32);

/// One alias row in a voicebank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub id: AliasId,
    pub text: Box<str>,
    pub phones: Box<[ArpaPhone]>,
    pub pitch: Option<PitchId>,
}

/// One OTO timing row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtoEntry {
    pub alias: AliasId,
    pub source: u32,
    pub offset_us: u64,
    pub preutterance_us: u64,
    pub overlap_us: u64,
    pub cutoff_us: i64,
    pub consonant_velocity: Option<f32>,
    pub pitch: Option<PitchId>,
}

/// A versioned imported voicebank.
#[derive(Debug, Clone)]
pub struct VoicebankManifest {
    pub revision: [u8; 32],
    pub inventory: AliasInventoryId,
    pub aliases: Box<[Alias]>,
    pub oto: Box<[OtoEntry]>,
    pub pitches: Box<[PitchId]>,
}

/// One phone timing on the target timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhoneTiming {
    pub target_phone: u32,
    pub start_us: u64,
    pub end_us: u64,
}

/// One F0 and gain point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProsodyPoint {
    pub at_us: u64,
    pub f0_millihz: Option<u32>,
    pub gain_q16: u32,
}

/// Duration and prosody for one clause.
#[derive(Debug, Clone)]
pub struct ProsodyPlan {
    pub key: SpeechKey,
    pub duration_us: u64,
    pub timings: Box<[PhoneTiming]>,
    pub contour: Box<[ProsodyPoint]>,
}

/// One selected voicebank unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedUnit {
    pub unit: UnitId,
    pub alias: AliasId,
    pub target_phones: IndexSpan,
    pub start_us: u64,
    pub end_us: u64,
    pub pitch: Option<PitchId>,
}

/// One selected unit plan.
#[derive(Debug, Clone)]
pub struct UnitPlan {
    pub key: SpeechKey,
    pub bank_revision: [u8; 32],
    pub units: Box<[PlannedUnit]>,
    pub prosody: ProsodyPlan,
}

/// Device sample format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

/// One rendered PCM block.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub key: SpeechKey,
    pub sample_rate_hz: u32,
    pub start_frame: u64,
    pub mono_frames: Box<[f32]>,
    pub final_chunk: bool,
}

/// One stereo output frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputFrame {
    pub channels: [f32; 2],
}

/// Evidence that playback completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionEvidence {
    DevicePlayed,
    PredictedDeviceTime,
}

/// One playback completion event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackCompletion {
    pub key: SpeechKey,
    pub last_device_frame_exclusive: u64,
    pub evidence: CompletionEvidence,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SpeechKey {
        SpeechKey::new(0, 1, 0).unwrap()
    }

    #[test]
    fn speech_input_rejects_empty_and_oversized() {
        assert!(SpeechInput::new(key(), "").is_err());
        let long = "a".repeat(MAX_INPUT_SCALARS + 1);
        assert!(SpeechInput::new(key(), long).is_err());
        assert!(SpeechInput::new(key(), "hello").is_ok());
    }

    #[test]
    fn speech_key_rejects_zero_turn() {
        assert!(SpeechKey::new(0, 0, 0).is_err());
        assert!(SpeechKey::new(0, 1, MAX_CLAUSES).is_err());
    }

    #[test]
    fn spans_reject_reversed_bounds() {
        assert!(IndexSpan::new(2, 1).is_err());
        assert!(ByteSpan::new(2, 1).is_err());
        assert!(IndexSpan::new(1, 1).unwrap().is_empty());
    }

    #[test]
    fn phoneme_sequence_rejects_out_of_range_spans() {
        let input = SpeechInput::new(key(), "hello").unwrap();
        let sequence = PhonemeSequence {
            input,
            phones: Box::new([]),
            words: Box::new([LexicalWord {
                text: ByteSpan::new(0, 5).unwrap(),
                phones: IndexSpan::new(0, 1).unwrap(),
                syllables: IndexSpan::new(0, 0).unwrap(),
                part_of_speech: PartOfSpeech::Unknown,
                source: PronunciationSource::Lexicon,
                status: PronunciationStatus::Selected,
            }]),
            syllables: Box::new([]),
            breaks: Box::new([]),
            frontend_revision: [0; 32],
        };
        assert!(matches!(sequence.validate(), Err(SpeechError::Bounds)));
    }

    #[test]
    fn input_debug_redacts_text() {
        let input = SpeechInput::new(key(), "secret words").unwrap();
        assert!(!format!("{input:?}").contains("secret"));
    }
}
