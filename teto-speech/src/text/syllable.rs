//! Syllable segmentation seam.
use crate::{error::Result, text::Syllabifier, types::PhonemeSequence};

/// Split words into syllables and fill syllable spans.
pub struct SyllableSplitter;

impl Syllabifier for SyllableSplitter {
    fn syllabify(&mut self, _sequence: &mut PhonemeSequence) -> Result<()> {
        todo!("split words into syllables")
    }
}
