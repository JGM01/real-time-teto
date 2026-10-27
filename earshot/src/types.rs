//! Public values for activation, delivery, and timing.
//!
//! Constructors enforce the audio and lifecycle contracts.

use crate::{Audio, Error};
#[cfg(feature = "serde")]
use serde::Serialize;
use std::fmt;

/// Monotonic ID scoped to one Earshot owner. Only accepted activations create IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ActivationId(pub(crate) u64);

impl ActivationId {
    /// Return the numeric ID.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ActivationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Sample positions use the activation's 16 kHz timeline. Durations are monotonic.
///
/// The type is non-exhaustive: new timing fields can appear in a minor release.
/// Use [`Timings::default`] when constructing a value from outside the crate.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[non_exhaustive]
pub struct Timings {
    /// Time from activation to capture readiness.
    pub activation_to_ready_ms: f64,
    /// First speech sample, when speech was detected.
    pub onset_sample: Option<u64>,
    /// Last speech sample, when speech was detected.
    pub last_speech_sample: Option<u64>,
    /// Sample at which the coordinator made its final decision.
    pub decision_sample: u64,
    /// Time from readiness to the final decision.
    pub decision_after_ready_ms: f64,
    /// Time from the final decision to the end of capture.
    pub capture_stop_ms: f64,
    /// Number of scored voice activity frames.
    pub vad_frames: u64,
    /// Slowest voice activity frame, in milliseconds.
    pub vad_max_ms: f64,
    /// Number of semantic endpoint calls.
    pub endpoint_calls: u64,
    /// Slowest semantic endpoint call, in milliseconds.
    pub endpoint_max_ms: f64,
}

/// Owned statement created only by a current positive semantic decision.
#[derive(Debug)]
pub struct Statement {
    id: ActivationId,
    audio: Audio,
    timings: Timings,
}

impl Statement {
    pub(crate) fn detected(id: ActivationId, audio: Audio, timings: Timings) -> Self {
        assert!(!audio.is_empty(), "a statement owns confirmed speech");
        Self { id, audio, timings }
    }

    /// Return the activation that created this statement.
    #[must_use]
    pub fn activation_id(&self) -> ActivationId {
        self.id
    }

    /// Borrow the owned audio.
    #[must_use]
    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    /// Borrow the timing values.
    #[must_use]
    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    /// Take the owned audio.
    #[must_use]
    pub fn into_audio(self) -> Audio {
        self.audio
    }
}

/// The reason that a capture ended without a semantic decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[non_exhaustive]
pub enum IncompleteReason {
    /// Five seconds of quiet without a positive decision. The audio is usable.
    SilenceLimit,
    /// The recording reached its duration bound. The audio may end in speech.
    DurationLimit,
}

/// Terminal capture result.
///
/// `Complete` and silence-limit `Incomplete` deliver owned audio.
/// A fallback remains incomplete for semantic accuracy and metrics.
///
/// The type is non-exhaustive: new outcomes can appear in a minor release.
#[derive(Debug)]
#[non_exhaustive]
pub enum Outcome {
    /// A current positive semantic decision created a statement.
    Complete(Statement),
    /// Captured speech with its stop reason.
    Incomplete {
        /// The captured audio.
        audio: Audio,
        /// Why capture ended.
        reason: IncompleteReason,
        /// Timing values for the capture.
        timings: Timings,
    },
    /// No speech started before the limit.
    NoSpeech,
    /// The caller cancelled the activation.
    Cancelled,
    /// Capture or inference failed.
    Failed(Error),
}

/// An accepted attempt emits at most one `Listening` event and exactly one
/// `Finished` event, unless the owner closes.
///
/// A startup failure can finish before `Listening`. The type is non-exhaustive:
/// new events can appear in a minor release.
#[derive(Debug)]
#[non_exhaustive]
pub enum Event {
    /// Capture is ready. The caller can start speaking.
    Listening(ActivationId),
    /// The attempt ended.
    Finished {
        /// The activation that ended.
        id: ActivationId,
        /// The terminal result.
        outcome: Outcome,
    },
}
