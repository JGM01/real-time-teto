//! Convert an LLM clause into a speech input.
use teto_llm_protocol::Clause;

use crate::{
    error::Result,
    types::{SpeechInput, SpeechKey},
};

/// Build a speech input from one clause.
pub fn from_clause(clause: &Clause, epoch: u64) -> Result<SpeechInput> {
    let key = SpeechKey::new(epoch, clause.turn().get(), clause.index().get())?;
    SpeechInput::new(key, clause.text())
}
