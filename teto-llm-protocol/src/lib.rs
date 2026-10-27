//! Native-free clause values and private parent/worker transport.
#![forbid(unsafe_code)]

pub mod measurement;
pub mod policy;
mod types;
mod wire;
pub use types::*;
pub use wire::*;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable error categories. Diagnostic text never contains response text or keys.
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

/// Whether an encrypted pending record reached durable storage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingDurability {
    #[default]
    Unknown,
    Durable,
    NotDurable,
}

/// An opaque, redacted subsystem failure.
#[derive(Clone, Serialize, Deserialize)]
pub struct Error {
    kind: ErrorKind,
    pending: PendingDurability,
}
impl Error {
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            pending: PendingDurability::Unknown,
        }
    }
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
    pub fn with_pending_durable(mut self, durable: bool) -> Self {
        self.pending = if durable {
            PendingDurability::Durable
        } else {
            PendingDurability::NotDurable
        };
        self
    }
    pub fn pending_durability(&self) -> PendingDurability {
        self.pending
    }
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LLM {:?} failure", self.kind)?;
        if self.kind == ErrorKind::JournalFull {
            formatter.write_str(
                "; select or initialize another journal, or restart in explicit no-journal mode",
            )?;
        }
        match self.pending {
            PendingDurability::Unknown => (),
            PendingDurability::Durable => {
                formatter.write_str("; admission stopped; encrypted pending record is durable")?
            }
            PendingDurability::NotDurable => formatter
                .write_str("; admission stopped; encrypted pending record is not durable")?,
        }
        Ok(())
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub fn failure(kind: ErrorKind) -> Error {
    Error::new(kind)
}
