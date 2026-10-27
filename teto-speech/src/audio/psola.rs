//! PSOLA rendering seam.
use crate::{
    audio::{AudioSink, Renderer},
    error::Result,
    types::UnitPlan,
};

/// Render voicebank units with TD-PSOLA.
pub struct PsolaRenderer;

impl Renderer for PsolaRenderer {
    fn render(&mut self, _plan: &UnitPlan, _sink: &mut dyn AudioSink) -> Result<()> {
        todo!("render voicebank units with TD-PSOLA")
    }
}
