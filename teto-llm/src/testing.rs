//! Fuzz and property helpers. Enable the `testing` feature.
use crate::text::TextPipeline;
use crate::*;

/// Run bytes through the text pipeline and return the clauses.
///
/// # Errors
/// Returns the normalization error reported by the pipeline.
pub fn render_text(bytes: &[u8]) -> Result<Vec<Clause>> {
    let mut pipeline = TextPipeline::new(TurnId::new(1)?);
    let mut clauses = Vec::new();
    pipeline.push(bytes, &mut |clause| {
        clauses.push(clause);
        Ok(())
    })?;
    pipeline.finish(true, &mut |clause| {
        clauses.push(clause);
        Ok(())
    })?;
    Ok(clauses)
}
