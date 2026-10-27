use std::io;

use teto_llm_protocol::{Error, ErrorKind, failure};

/// A parent-side client failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The worker reported a domain error.
    #[error("worker error: {0}")]
    Remote(Error),
    /// The pipe or frame layer failed.
    #[error("transport error: {0}")]
    Transport(Error),
    /// The operation exceeded its deadline.
    #[error("{operation} timed out")]
    Timeout { operation: &'static str },
    /// The worker process exited.
    #[error("worker exited with {status:?}")]
    Exited { status: Option<i32> },
    /// A local I/O operation failed.
    #[error("{operation}: {kind:?}")]
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
    /// The peer sent an invalid frame or response.
    #[error("protocol error")]
    Protocol,
    /// The sequence number cannot advance.
    #[error("sequence exhausted")]
    SequenceExhausted,
    /// The client is closed or unusable.
    #[error("client closed")]
    Closed,
    /// A timed-out operation may have reached the worker.
    #[error("{operation} outcome is unknown")]
    OutcomeUnknown { operation: &'static str },
}

impl ClientError {
    /// Convert the failure into the bounded protocol error.
    pub fn into_protocol(self) -> Error {
        match self {
            Self::Remote(error) | Self::Transport(error) => error,
            Self::Protocol => failure(ErrorKind::Protocol),
            Self::Timeout { .. }
            | Self::Exited { .. }
            | Self::Io { .. }
            | Self::SequenceExhausted
            | Self::Closed
            | Self::OutcomeUnknown { .. } => failure(ErrorKind::Worker),
        }
    }
}
