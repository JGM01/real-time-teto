//! Local voicebank import and CVVC selection.
pub mod alias;
pub mod cvvc;
pub mod id;
pub use id::{AliasIndex, AliasInventoryId, OtoRowId, PitchId, SourceId};
pub mod index;
pub mod limits;
pub mod oto;
#[cfg(feature = "voicebank-wav")]
mod read;
