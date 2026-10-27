//! Teto English CVVC unit selection seam.
use crate::{
    error::Result,
    types::{PhonemeSequence, ProsodyPlan, UnitPlan, VoicebankManifest},
    voicebank::AliasPlanner,
};

/// Select CVVC aliases for the Teto English bank.
pub struct TetoCvvcPlanner;

impl AliasPlanner for TetoCvvcPlanner {
    fn plan(
        &mut self,
        _sequence: &PhonemeSequence,
        _prosody: &ProsodyPlan,
        _bank: &VoicebankManifest,
    ) -> Result<UnitPlan> {
        todo!("select CVVC aliases and OTO rows")
    }
}
