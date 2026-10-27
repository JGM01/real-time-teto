//! English speech contracts for the Teto pipeline.
//!
//! The default build contains types and traits only. Model, voicebank and
//! device backends are optional features.
#![forbid(unsafe_code)]

pub mod audio;
pub mod error;
pub mod prosody;
pub mod text;
pub mod types;
pub mod voicebank;

#[cfg(feature = "llm-adapter")]
pub mod adapter;
