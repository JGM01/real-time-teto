//! Prosody predictor trait.
use crate::{
    error::Result,
    types::{PhonemeSequence, ProsodyPlan},
};

/// Predict duration, F0 and gain for one clause.
pub trait ProsodyPredictor {
    /// Build a validated prosody plan.
    fn predict(&mut self, sequence: &PhonemeSequence) -> Result<ProsodyPlan>;
}
