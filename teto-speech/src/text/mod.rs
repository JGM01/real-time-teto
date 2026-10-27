//! English lexical analysis and phonemization contracts.
use crate::{
    error::Result,
    types::{ArpaPhone, EnglishPhone, PhonemeSequence, SpeechInput},
};

pub mod phonemize;
pub mod syllable;
pub mod xsampa;

/// Convert one bounded clause to an English phoneme sequence.
pub trait Phonemizer {
    /// Phonemize one clause.
    fn phonemize(&mut self, input: &SpeechInput) -> Result<PhonemeSequence>;
}

/// Add syllable spans to a phoneme sequence.
pub trait Syllabifier {
    /// Split words into syllables.
    fn syllabify(&mut self, sequence: &mut PhonemeSequence) -> Result<()>;
}

/// Predict phones for one out-of-dictionary word.
pub trait OovPredictor {
    /// Return the predicted phone sequence.
    fn predict(&mut self, word: &str) -> Result<Box<[EnglishPhone]>>;
}

/// Map one ARPAbet phone to a voicebank symbol.
pub trait XsampaMap {
    /// Return the X-SAMPA symbol for one phone.
    fn map(&self, phone: ArpaPhone) -> &'static str;
}
