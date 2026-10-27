//! Render and output contracts.
use crate::{
    error::Result,
    types::{AudioChunk, UnitPlan},
};

pub mod psola;
pub mod ring;
pub mod sink;

/// Receive rendered PCM blocks.
pub trait AudioSink {
    /// Push one block.
    fn push(&mut self, chunk: &AudioChunk) -> Result<()>;
}

/// Render one unit plan into an audio sink.
pub trait Renderer {
    /// Render one unit plan.
    fn render(&mut self, plan: &UnitPlan, sink: &mut dyn AudioSink) -> Result<()>;
}
