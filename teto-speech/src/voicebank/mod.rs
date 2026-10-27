//! Voicebank import and unit selection contracts.
use std::path::Path;

use crate::{
    error::Result,
    types::{PhonemeSequence, ProsodyPlan, UnitPlan, VoicebankManifest},
};

pub mod alias;
pub mod manifest;
pub mod oto;
pub mod select;

/// Import a voicebank manifest from a local directory.
pub trait OtoImport {
    /// Load and validate the bank.
    fn load(&mut self, path: &Path) -> Result<VoicebankManifest>;
}

/// Select OTO units for one phoneme sequence.
pub trait AliasPlanner {
    /// Build a unit plan.
    fn plan(
        &mut self,
        sequence: &PhonemeSequence,
        prosody: &ProsodyPlan,
        bank: &VoicebankManifest,
    ) -> Result<UnitPlan>;
}
