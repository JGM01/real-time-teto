//! Optional bounded evidence. The application, not Earshot, persists these values.

#[cfg(feature = "tooling")]
mod enabled;
#[cfg(feature = "tooling")]
pub(crate) use enabled::{
    CaptureTrace, Log, TraceArena, mark_accepted, mark_discarded, mark_stale,
};
#[cfg(feature = "tooling")]
pub use enabled::{Disposition, Evaluation, Sink, Trace};

#[cfg(not(feature = "tooling"))]
mod disabled;
#[cfg(not(feature = "tooling"))]
pub(crate) use disabled::{
    CaptureTrace, Log, TraceArena, mark_accepted, mark_discarded, mark_stale,
};
