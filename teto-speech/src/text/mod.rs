//! English lexical analysis and phonemization.
#[cfg(feature = "g2p-gru")]
pub mod gru;
#[cfg(feature = "cmudict")]
pub mod lexicon;
pub mod phonemize;
mod phones;
#[cfg(feature = "cmudict")]
pub mod syllable;
pub mod xsampa;
