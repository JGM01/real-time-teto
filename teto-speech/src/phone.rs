//! English phone, word and syllable types.
use crate::span::{ByteSpan, IndexSpan};

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
    pub nucleus: u32,
    pub stress: Stress,
}

impl Syllable {
    /// Return the onset span.
    pub fn onset(self) -> IndexSpan {
        IndexSpan {
            start: self.phones.start,
            end: self.nucleus,
        }
    }

    /// Return the nucleus span.
    pub fn nucleus_span(self) -> IndexSpan {
        IndexSpan {
            start: self.nucleus,
            end: self.nucleus + 1,
        }
    }

    /// Return the coda span.
    pub fn coda(self) -> IndexSpan {
        IndexSpan {
            start: self.nucleus + 1,
            end: self.phones.end,
        }
    }
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
    pub break_kind: BreakKind,
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
