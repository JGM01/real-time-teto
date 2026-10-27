use std::{fmt, num::NonZeroU64, sync::Arc};

use crate::error::{Result, SpeechError};

pub use crate::limits::*;
pub use crate::plan::{PlannedUnit, ProsodyPlan, Realization, RealizationPart, UnitPlan};
pub use crate::sequence::PhonemeSequence;
pub use crate::voicebank::manifest::{
    Alias, OtoEntry, PitchGroup, Source, SourceId, VoicebankManifest,
};

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
    pub fn new(key: SpeechKey, text: impl AsRef<str>) -> Result<Self> {
        let text = text.as_ref();
        SpeechKey::new(key.epoch, key.turn.get(), key.clause)?;
        if text.trim().is_empty()
            || text.chars().count() > MAX_INPUT_SCALARS
            || text.chars().any(char::is_control)
        {
            return Err(SpeechError::InvalidInput);
        }
        Ok(Self {
            key,
            text: Arc::from(text),
        })
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
    pub(crate) start: u32,
    /// One past the last byte.
    pub(crate) end: u32,
}

impl ByteSpan {
    pub fn start(self) -> u32 {
        self.start
    }
    pub fn end(self) -> u32 {
        self.end
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
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
    pub(crate) start: u32,
    /// One past the last index.
    pub(crate) end: u32,
}

impl IndexSpan {
    pub fn start(self) -> u32 {
        self.start
    }
    pub fn end(self) -> u32 {
        self.end
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
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
    pub(crate) phone: ArpaPhone,
    pub(crate) stress: Option<Stress>,
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
