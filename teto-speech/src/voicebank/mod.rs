//! Local voicebank import and CVVC selection.
pub mod alias;
pub mod manifest;
pub mod oto;
#[cfg(feature = "voicebank-wav")]
mod read;
pub mod select;
