//! English speech contracts for the Teto pipeline.
//!
//! The default build contains checked contracts and planning logic. Asset and
//! device backends are optional features.
#![forbid(unsafe_code)]

pub mod audio;
pub mod diagnostics;
pub mod error;
pub mod limits;
mod plan;
pub mod prosody;
mod sequence;
pub mod text;
pub mod types;
pub mod voicebank;

#[cfg(feature = "llm-adapter")]
pub mod adapter;

#[cfg(feature = "testing")]
pub mod testing;
#[cfg(all(test, feature = "cmudict", not(feature = "testing")))]
mod testing;
