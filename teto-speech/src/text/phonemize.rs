//! CMUdict phonemization and OOV prediction seams.
use crate::{
    error::Result,
    text::{OovPredictor, Phonemizer},
    types::{EnglishPhone, PhonemeSequence, SpeechInput},
};

/// CMUdict lookup with OOV fallback.
pub struct CmudictPhonemizer;

impl Phonemizer for CmudictPhonemizer {
    fn phonemize(&mut self, _input: &SpeechInput) -> Result<PhonemeSequence> {
        todo!("load CMUdict and build the phoneme sequence")
    }
}

/// The g2p_en OOV network.
pub struct G2pEnOnnx;

impl OovPredictor for G2pEnOnnx {
    fn predict(&mut self, _word: &str) -> Result<Box<[EnglishPhone]>> {
        todo!("run the g2p_en OOV network")
    }
}
