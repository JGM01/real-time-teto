//! Qualification and replay harness for Earshot.
//!
//! Replay goes through the production workers and coordinator. The harness adds
//! a fixed policy grid and bounded diagnostic traces. It does not change the
//! production policy.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod error;

pub use earshot::tooling::{
    Disposition, Evaluation, MODEL_LICENSE, MODEL_NAME, MODEL_REVISION, MODEL_SHA256, ReplayPacing,
    TRACE_SAMPLES, Trace,
};
pub use error::{Error, Result};

use earshot::{
    Earshot,
    tooling::{InputSource, Policy as ToolingPolicy, Sink},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The qualification pause grid in milliseconds. One source of truth.
pub const QUALIFICATION_PAUSES_MS: [u64; 5] = [1_024, 1_280, 1_536, 1_792, 2_048];

/// One candidate on the fixed qualification grid.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    /// The minimum pause in milliseconds. Must be on the grid.
    pub pause_ms: u64,
    /// The completion threshold in `[0.01, 0.99]`.
    pub threshold: f32,
}

impl Candidate {
    fn policy(self) -> Result<ToolingPolicy> {
        if !self.threshold.is_finite()
            || !(0.01..=0.99).contains(&self.threshold)
            || !QUALIFICATION_PAUSES_MS.contains(&self.pause_ms)
        {
            return Err(Error::Config("candidate is outside the fixed policy grid"));
        }
        Ok(ToolingPolicy {
            pause_ms: self.pause_ms,
            threshold: self.threshold,
        })
    }
}

/// Missing diagnostic attempts are counted, and mark the next received trace
/// incomplete.
pub struct TraceReceiver {
    receiver: crossbeam_channel::Receiver<Trace>,
    omitted: Arc<AtomicU64>,
    observed: u64,
}

impl TraceReceiver {
    /// Receive one completed trace, if one is ready.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Earshot`] with `Closed` after the owner closes.
    pub fn try_recv(&mut self) -> Result<Option<Trace>> {
        match self.receiver.try_recv() {
            Ok(mut trace) => {
                let total = self.omitted.load(Ordering::Acquire);
                trace.omitted_traces = total - self.observed;
                self.observed = total;
                trace.complete &= trace.omitted_traces == 0;
                Ok(Some(trace))
            }
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(_) => Err(Error::Earshot(earshot::Error::Closed)),
        }
    }
}

fn traced(
    source: impl FnMut() -> earshot::Result<InputSource> + Send + 'static,
    policy: ToolingPolicy,
) -> Result<(Earshot, TraceReceiver)> {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let omitted = Arc::new(AtomicU64::new(0));
    let sink = Sink {
        sender,
        omitted: omitted.clone(),
    };
    let owner = earshot::tooling::start(source, policy, Some(sink))?;
    Ok((
        owner,
        TraceReceiver {
            receiver,
            omitted,
            observed: 0,
        },
    ))
}

/// Open the default microphone with diagnostic capture enabled.
///
/// # Errors
///
/// Returns an engine or device startup error.
pub fn open_traced() -> Result<(Earshot, TraceReceiver)> {
    traced(|| Ok(InputSource::Microphone), ToolingPolicy::default())
}

/// Replay one file through the production workers and coordinator.
///
/// # Errors
///
/// Returns a configuration or startup error.
pub fn open_replay(
    path: impl AsRef<Path>,
    pacing: ReplayPacing,
    candidate: Candidate,
) -> Result<(Earshot, TraceReceiver)> {
    let path = path.as_ref().to_owned();
    traced(
        move || Ok(InputSource::file(path.clone(), pacing)),
        candidate.policy()?,
    )
}

/// Activate each file once in order while retaining the warmed engines.
///
/// # Errors
///
/// Returns a configuration or startup error.
pub fn open_playlist(
    paths: Vec<PathBuf>,
    pacing: ReplayPacing,
    candidate: Candidate,
) -> Result<(Earshot, TraceReceiver)> {
    if paths.is_empty() || paths.len() > 512 {
        return Err(Error::Config("playlist requires 1..=512 files"));
    }
    let mut paths = paths.into_iter();
    traced(
        move || {
            paths
                .next()
                .map(|path| InputSource::file(path, pacing))
                .ok_or(earshot::Error::SourceEnded)
        },
        candidate.policy()?,
    )
}

/// Replay owned 16 kHz mono samples for signal and performance audits.
///
/// # Errors
///
/// Returns [`Error::InvalidAudio`] for empty, non-finite, or oversized input,
/// or a configuration or startup error.
pub fn open_samples(
    samples: Vec<f32>,
    pacing: ReplayPacing,
    candidate: Candidate,
) -> Result<(Earshot, TraceReceiver)> {
    if samples.is_empty()
        || samples.len() > TRACE_SAMPLES
        || !samples.iter().all(|sample| sample.is_finite())
    {
        return Err(Error::InvalidAudio);
    }
    let samples: Arc<[f32]> = Arc::from(samples);
    traced(
        move || Ok(InputSource::samples(samples.clone(), pacing)),
        candidate.policy()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_diagnostic_consumers_observe_dropped_trace_counts() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let omitted = Arc::new(AtomicU64::new(0));
        let sink = Sink {
            sender,
            omitted: omitted.clone(),
        };
        let mut receiver = TraceReceiver {
            receiver,
            omitted,
            observed: 0,
        };
        for _ in 0..3 {
            sink.send(Trace {
                model_sha256: MODEL_SHA256,
                activation_id: 1,
                samples: Vec::new(),
                complete: true,
                captured_samples: 0,
                timings: earshot::Timings::default(),
                outcome: "cancelled",
                evaluations: Vec::new(),
                omitted_evaluations: 0,
                minimum_pause_samples: 1_024,
                completion_threshold: 0.01,
                omitted_traces: 0,
            });
        }
        let trace = receiver.try_recv().unwrap().unwrap();
        assert_eq!(trace.omitted_traces, 2);
        assert!(!trace.complete);
    }

    #[test]
    fn candidate_grid_has_one_definition() {
        assert_eq!(QUALIFICATION_PAUSES_MS.len(), 5);
        for pause_ms in QUALIFICATION_PAUSES_MS {
            Candidate {
                pause_ms,
                threshold: 0.5,
            }
            .policy()
            .unwrap();
        }
        assert!(
            Candidate {
                pause_ms: 256,
                threshold: 0.5
            }
            .policy()
            .is_err()
        );
    }
}
