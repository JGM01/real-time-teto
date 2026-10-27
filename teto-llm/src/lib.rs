//! Local inference, clause assembly, played-history and encrypted journaling.
#![forbid(unsafe_code)]

pub use teto_llm_protocol::*;
mod cache;
mod context;
mod engine;
pub mod journal;
mod mailbox;
mod native;
mod profile;
pub mod qualification;
mod text;
pub use engine::{Llm, Turn, TurnControl};
pub use profile::Profile;
