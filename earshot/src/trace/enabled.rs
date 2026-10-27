//! Bounded diagnostic evidence, compiled only for tooling.

use crate::{Timings, constants::*};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// The disposition of one model evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    /// The evaluation is current but not yet used.
    Pending,
    /// The probability was below the policy threshold.
    BelowThreshold,
    /// Speech resumed before the evaluation became usable.
    Stale,
    /// The evaluation completed the statement.
    Accepted,
    /// The capture ended before the evaluation became usable.
    Discarded,
}

/// One model evaluation on the capture timeline.
#[derive(Debug, Serialize)]
pub struct Evaluation {
    /// The monotonic request number.
    pub request: u64,
    /// The speech revision that the request scored.
    pub revision: u64,
    /// The last sample in the model input.
    pub input_end_sample: u64,
    /// The model probability.
    pub probability: f32,
    /// Time spent in preprocessing and inference.
    pub processing_ms: f64,
    /// What the coordinator did with the evaluation.
    pub disposition: Disposition,
}

/// Bounded diagnostic evidence for one activation.
#[derive(Debug, Serialize)]
pub struct Trace {
    /// The SHA-256 of the bundled model.
    pub model_sha256: &'static str,
    /// The activation that produced this trace.
    pub activation_id: u64,
    /// Captured samples, bounded by the trace capacity.
    pub samples: Vec<f32>,
    /// Whether every diagnostic value survived the bounded channels.
    pub complete: bool,
    /// Total captured samples, including samples omitted from `samples`.
    pub captured_samples: u64,
    /// Timing values for the activation.
    pub timings: Timings,
    /// The terminal outcome name.
    pub outcome: &'static str,
    /// Recorded model evaluations, bounded by the trace capacity.
    pub evaluations: Vec<Evaluation>,
    /// Number of evaluations the log dropped.
    pub omitted_evaluations: u64,
    /// The minimum pause in samples.
    pub minimum_pause_samples: u64,
    /// The completion threshold.
    pub completion_threshold: f32,
    /// Number of traces that a slow consumer dropped.
    pub omitted_traces: u64,
}

/// A bounded sink for completed traces.
pub struct Sink {
    /// The bounded trace channel.
    pub sender: crossbeam_channel::Sender<Trace>,
    /// The count of traces that a slow consumer dropped.
    pub omitted: Arc<AtomicU64>,
}

impl Sink {
    /// Offer one completed trace without blocking.
    pub fn send(&self, trace: Trace) {
        if self.sender.try_send(trace).is_err() {
            self.omitted.fetch_add(1, Ordering::Release);
        }
    }
}

/// The exact capture samples and completeness for one activation.
pub(crate) struct CaptureTrace {
    pub samples: Vec<f32>,
    pub complete: bool,
    pub captured_samples: u64,
}

/// A reusable bounded buffer for capture samples.
pub(crate) struct TraceArena {
    samples: Vec<f32>,
    complete: bool,
    captured_samples: u64,
}

impl TraceArena {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            complete: true,
            captured_samples: 0,
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.complete = true;
        self.captured_samples = 0;
    }

    pub fn append(&mut self, samples: &[f32]) {
        self.captured_samples += samples.len() as u64;
        let remaining = TRACE_SAMPLES - self.samples.len();
        let count = remaining.min(samples.len());
        self.samples.extend_from_slice(&samples[..count]);
        self.complete &= count == samples.len();
    }

    pub fn fail(&mut self) {
        self.complete = false;
    }

    /// Copy the capture into an exact-size value and reuse the arena.
    pub fn finish(&mut self) -> CaptureTrace {
        let captured = CaptureTrace {
            samples: self.samples.clone(),
            complete: self.complete,
            captured_samples: self.captured_samples,
        };
        self.reset();
        captured
    }
}

/// One activation's model evaluation log.
pub(crate) struct Log {
    pub evaluations: Vec<Evaluation>,
    pub omitted: u64,
}

impl Log {
    pub fn new() -> Self {
        Self {
            evaluations: Vec::with_capacity(TRACE_CAPACITY),
            omitted: 0,
        }
    }

    /// Record one evaluation. Return its index, or `None` when the log is full.
    pub fn record(
        &mut self,
        request: u64,
        revision: u64,
        input_end_sample: u64,
        probability: f32,
        processing_ms: f64,
        positive: bool,
    ) -> Option<usize> {
        if self.evaluations.len() == TRACE_CAPACITY {
            self.omitted += 1;
            return None;
        }
        let index = self.evaluations.len();
        self.evaluations.push(Evaluation {
            request,
            revision,
            input_end_sample,
            probability,
            processing_ms,
            disposition: if positive {
                Disposition::Pending
            } else {
                Disposition::BelowThreshold
            },
        });
        Some(index)
    }

    pub fn mark(&mut self, index: Option<usize>, disposition: Disposition) {
        if let Some(index) = index {
            self.evaluations[index].disposition = disposition;
        }
    }
}

/// Mark current positive evidence as accepted.
pub(crate) fn mark_accepted(log: &mut Log, index: Option<usize>) {
    log.mark(index, Disposition::Accepted);
}

/// Mark evidence invalidated by resumed speech.
pub(crate) fn mark_stale(log: &mut Log, index: Option<usize>) {
    log.mark(index, Disposition::Stale);
}

/// Mark evidence abandoned by a non-complete finish.
pub(crate) fn mark_discarded(log: &mut Log, index: Option<usize>) {
    log.mark(index, Disposition::Discarded);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_audio_and_evidence_are_bounded_and_reused() {
        let mut arena = TraceArena::new(TRACE_SAMPLES);
        arena.append(&vec![0.2; TRACE_SAMPLES]);
        arena.append(&[0.3; 512]);
        let captured = arena.finish();
        assert_eq!(captured.samples.len(), TRACE_SAMPLES);
        assert_eq!(captured.captured_samples, TRACE_SAMPLES as u64 + 512);
        assert!(!captured.complete);
        assert!(arena.samples.is_empty());
        assert!(arena.complete);

        let mut log = Log::new();
        for request in 0..TRACE_CAPACITY + 1 {
            log.record(request as u64, 1, 0, 0.1, 0.0, false);
        }
        assert_eq!(log.evaluations.len(), TRACE_CAPACITY);
        assert_eq!(log.omitted, 1);
    }

    #[test]
    fn dispositions_serialize_with_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Disposition::BelowThreshold).unwrap(),
            "\"below-threshold\""
        );
    }
}
