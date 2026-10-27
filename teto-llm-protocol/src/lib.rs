//! Native-free clause values and private parent/worker transport.
#![forbid(unsafe_code)]

pub mod policy;
mod types;
mod wire;
pub use types::*;
pub use wire::*;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable categories. Diagnostic messages never contain response text or keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Config,
    ContextBudget,
    PlaybackPending,
    Profile,
    Generation,
    Normalization,
    JournalFull,
    Persistence,
    Worker,
    Protocol,
    Exhausted,
    Receipt,
    Cancelled,
}

/// An opaque, redacted subsystem failure.
#[derive(Clone, Serialize, Deserialize)]
pub struct Error {
    kind: ErrorKind,
    pending_durable: Option<bool>,
}
impl Error {
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            pending_durable: None,
        }
    }
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
    pub fn with_pending_durable(mut self, durable: bool) -> Self {
        self.pending_durable = Some(durable);
        self
    }
    pub fn pending_durable(&self) -> Option<bool> {
        self.pending_durable
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LLM {:?} failure", self.kind)?;
        if self.kind == ErrorKind::JournalFull {
            f.write_str(
                "; select or initialize another journal, or restart in explicit no-journal mode",
            )?;
        }
        if let Some(durable) = self.pending_durable {
            write!(
                f,
                "; admission stopped; encrypted pending record durable={durable}"
            )?;
        }
        Ok(())
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub fn failure(kind: ErrorKind) -> Error {
    Error::new(kind)
}
