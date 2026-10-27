//! Parent-side client for the private LLM worker.
//!
//! The crate contains no inference code and does not link llama.cpp.
#![forbid(unsafe_code)]

mod client;
mod error;
mod journal;
mod transport;
mod worker;

pub use client::{CancellationToken, Client, ClientOptions};
pub use error::ClientError;
pub use journal::{JournalAdmin, PreparedDelete};
pub use worker::WorkerLaunch;
