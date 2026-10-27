//! Hidden tooling seam for the audit harness. Not part of the stable API.

use crate::{
    Earshot, Result, constants::SAMPLE_RATE_HZ, coordinator::Setup, policy::Policy as RuntimePolicy,
};

pub use crate::capture::{InputSource, ReplayPacing};
pub use crate::constants::TRACE_SAMPLES;
pub use crate::smart_turn::{MODEL_LICENSE, MODEL_NAME, MODEL_REVISION, MODEL_SHA256};
pub use crate::trace::{Disposition, Evaluation, Sink, Trace};

/// The endpoint policy for a tooling replay.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// The minimum pause in milliseconds.
    pub pause_ms: u64,
    /// The completion threshold in `[0.01, 0.99]`.
    pub threshold: f32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            pause_ms: 1_024,
            threshold: 0.01,
        }
    }
}

/// Start Earshot with a tooling source, policy, and trace sink.
///
/// The source is asked once per activation. It returns [`InputSource::Microphone`]
/// for live capture, or a replay source for diagnostics.
///
/// # Errors
///
/// Returns an engine or device startup error.
///
/// # Panics
///
/// Panics if the pause is zero or does not fit in samples, or if the threshold
/// is not finite.
pub fn start(
    source: impl FnMut() -> Result<InputSource> + Send + 'static,
    policy: Policy,
    traces: Option<Sink>,
) -> Result<Earshot> {
    assert!(policy.pause_ms > 0, "tooling pause must be positive");
    assert!(
        policy.threshold.is_finite(),
        "tooling threshold must be finite"
    );
    let pause_samples = policy
        .pause_ms
        .checked_mul(u64::from(SAMPLE_RATE_HZ))
        .and_then(|samples| samples.checked_div(1_000))
        .expect("tooling pause fits in samples");
    let mut setup = Setup::microphone();
    setup.policy = RuntimePolicy {
        pause_samples,
        threshold: policy.threshold,
        ..RuntimePolicy::default()
    };
    setup.source = Box::new(source);
    setup.traces = traces;
    Earshot::start(setup)
}
