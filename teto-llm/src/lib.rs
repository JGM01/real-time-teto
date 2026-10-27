//! Local inference, clause assembly, played history, and encrypted journaling.
#![forbid(unsafe_code)]

mod api;
#[cfg(feature = "measurement")]
pub mod cache_probe;
mod context;
pub mod journal;
mod mailbox;
#[cfg(feature = "measurement")]
pub mod measurement;
mod prefix_cache;
mod profile;
#[cfg(feature = "testing")]
pub mod testing;
mod text;
mod worker;

pub use api::{Llm, Turn, TurnControl};
pub use profile::Profile;
pub use teto_llm_protocol::*;
