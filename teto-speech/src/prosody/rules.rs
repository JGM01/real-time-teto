//! Rule-based prosody seam.
use crate::{
    error::Result,
    prosody::predictor::ProsodyPredictor,
    types::{PhonemeSequence, ProsodyPlan},
};

/// Rule-based duration, pitch and gain.
pub struct RuleBasedProsody;

impl ProsodyPredictor for RuleBasedProsody {
    fn predict(&mut self, _sequence: &PhonemeSequence) -> Result<ProsodyPlan> {
        todo!("predict rule-based prosody")
    }
}
