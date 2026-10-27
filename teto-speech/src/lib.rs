//! English speech contracts for the Teto pipeline.
//!
//! The default build contains checked contracts and planning logic. Asset
//! backends are optional features.
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::todo,
        clippy::unimplemented
    )
)]

pub mod diagnostics;
pub mod error;
pub mod input;
pub mod phone;
pub mod plan;
pub mod sequence;
pub mod span;
pub mod text;
pub mod voicebank;

#[cfg(feature = "llm-adapter")]
pub mod adapter;

#[cfg(feature = "testing")]
pub mod testing;
#[cfg(all(test, feature = "cmudict", not(feature = "testing")))]
mod testing;
