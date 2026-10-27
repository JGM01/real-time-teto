//! Failures from audit configuration and replay.

/// Failures from audit configuration, replay, or Earshot itself.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Earshot failed.
    #[error(transparent)]
    Earshot(#[from] earshot::Error),
    /// An audit value is outside its permitted range.
    #[error("Invalid audit configuration: {0}")]
    Config(&'static str),
    /// Replay audio is empty, non-finite, or too long.
    #[error("Audit audio is invalid")]
    InvalidAudio,
}

/// A result that uses the audit error type.
pub type Result<T> = std::result::Result<T, Error>;
