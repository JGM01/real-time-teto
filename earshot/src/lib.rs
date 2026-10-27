//! Offline activation, speech capture, and semantic endpoint detection.
//!
//! Earshot captures one spoken statement at a time. The microphone stays closed
//! while the service is idle. The two inference engines load and warm once.
//! A caller activates an attempt. Earshot emits `Listening` when capture is
//! ready, then exactly one `Finished` outcome. Receiving `Finished` releases
//! the next activation. File output and downstream processing belong to the
//! caller.
//!
//! # Outcomes
//!
//! `Complete` carries a statement created by a current positive semantic
//! decision. `Incomplete(SilenceLimit)` carries captured speech after five
//! seconds of quiet and reports that completion is unconfirmed. Both paths
//! deliver owned 16 kHz mono audio. `Incomplete(DurationLimit)` carries partial
//! audio. `NoSpeech`, `Cancelled`, and `Failed` carry no audio.
//!
//! # Policy
//!
//! The bundled policy is a development baseline: a 1024 ms minimum pause and a
//! 0.01 completion threshold. Qualification is pending. These values are not
//! qualified defaults.
//!
//! # Threading
//!
//! [`Earshot`] is the single event owner. It is [`Send`]. The crate does not
//! promise [`Sync`], so keep the event loop on one thread. [`Control`] is a
//! cloneable handle and can be used from other threads.
//!
//! # Resources
//!
//! Earshot uses no network, writes no files, and emits no telemetry. The
//! microphone is closed while the service is idle.
//!
//! # Feature flags
//!
//! - `serde`: derive serialization for [`ActivationId`], [`Timings`], and
//!   [`IncompleteReason`].
//! - `tooling`: a hidden seam for the audit harness. It is not part of the
//!   stable API and is not semver-checked.
//!
//! # Example
//!
//! ```no_run
//! use earshot::{Earshot, Event, IncompleteReason, Outcome};
//! # fn process_audio(_: earshot::Audio, _: bool) {}
//! # fn main() -> earshot::Result<()> {
//! let mut earshot = Earshot::open()?;
//! let control = earshot.control();
//! let id = control.activate()?;
//! loop {
//!     match earshot.recv()? {
//!         Event::Listening(_) => { /* Capture is ready. */ }
//!         Event::Finished { outcome, .. } => {
//!             let (audio, confirmed) = match outcome {
//!                 Outcome::Complete(statement) => (statement.into_audio(), true),
//!                 Outcome::Incomplete {
//!                     audio,
//!                     reason: IncompleteReason::SilenceLimit,
//!                     ..
//!                 } => (audio, false),
//!                 Outcome::Failed(error) => return Err(error),
//!                 _ => break,
//!             };
//!             process_audio(audio, confirmed);
//!             break;
//!         }
//!         _ => break,
//!     }
//! }
//! control.cancel(id);
//! earshot.close()
//! # }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod audio;
mod capture;
mod constants;
mod control;
mod coordinator;
mod endpoint;
mod error;
mod policy;
mod runtime;
mod session;
mod smart_turn;
mod trace;
mod turn;
mod types;
mod vad;

pub use audio::Audio;
pub use constants::SAMPLE_RATE_HZ;
pub use control::{Control, Earshot};
pub use error::{Error, Result};
pub use types::{ActivationId, Event, IncompleteReason, Outcome, Statement, Timings};

pub(crate) use types::ActivationId as Id;

#[cfg(feature = "tooling")]
#[doc(hidden)]
pub mod tooling;

const _: () = {
    fn is_send<T: Send>() {}
    fn is_sync<T: Sync>() {}
    let _ = (
        is_send::<Audio>,
        is_send::<ActivationId>,
        is_send::<Timings>,
        is_send::<Statement>,
        is_send::<Outcome>,
        is_send::<Event>,
        is_send::<Error>,
        is_send::<Control>,
        is_send::<Earshot>,
        is_sync::<Audio>,
        is_sync::<ActivationId>,
        is_sync::<Timings>,
        is_sync::<Control>,
    );
};
