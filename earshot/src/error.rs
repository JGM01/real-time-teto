//! Failures from source setup, worker execution, and inference.

use std::io;

/// Failures from source setup, worker execution, and inference.
///
/// The error type is non-exhaustive: new variants can appear in a minor release.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The service is closed and cannot accept another request.
    #[error("Earshot is closed")]
    Closed,
    /// A finite source ended before an endpoint decision.
    #[error("Capture ended before an endpoint decision")]
    SourceEnded,
    /// A tooling configuration value is outside its permitted range.
    #[cfg(any(test, feature = "tooling"))]
    #[error("Invalid configuration: {0}")]
    InvalidConfig(&'static str),
    /// The host reports no default input device.
    #[error("No input device")]
    NoInputDevice,
    /// The input rate, channel count, or sample format is unsupported.
    #[error(
        "Input must use a supported rate (8/16/24/32/44.1/48/88.2/96/176.4/192 kHz), \
         mono or stereo, and f32 or PCM16"
    )]
    UnsupportedInputFormat,
    /// The audio device or its stream failed.
    #[error("Audio device error: {0}")]
    DeviceError(#[source] cpal::Error),
    /// A tooling source could not read its audio.
    #[cfg(any(test, feature = "tooling"))]
    #[error("Source I/O error: {0}")]
    SourceIo(#[source] io::Error),
    /// A tooling source received malformed WAV data.
    #[cfg(any(test, feature = "tooling"))]
    #[error("Invalid WAV: {0}")]
    InvalidWav(#[source] hound::Error),
    /// A worker thread could not start.
    #[error("Cannot start worker: {0}")]
    ThreadSpawn(#[source] io::Error),
    /// The voice activity detector could not initialize.
    #[error("Cannot initialize VAD: {0}")]
    VadInitialization(#[source] silero::Error),
    /// Voice activity inference failed.
    #[error("VAD inference failed: {0}")]
    VadInference(#[source] silero::Error),
    /// A detector returned a value that is not finite or not in `[0, 1]`.
    #[error("VAD probability is not finite or outside [0, 1]: {0}")]
    InvalidProbability(f32),
    /// The capture ring lost native frames.
    #[error("Capture overrun at sample {expected}; next sample is {actual}")]
    CaptureOverrun {
        /// The first expected sample.
        expected: u64,
        /// The sample that arrived instead.
        actual: u64,
    },
    /// Another activation or unread outcome blocks a new activation.
    #[error("Earshot is busy")]
    Busy,
    /// The semantic endpoint model could not initialize.
    #[error("Smart Turn initialization failed: {0}")]
    TurnInitialization(#[source] ort::Error),
    /// Semantic endpoint inference failed.
    #[error("Smart Turn inference failed: {0}")]
    TurnInference(#[source] ort::Error),
    /// The model tensors do not match the required contract.
    #[error("Smart Turn tensor contract is invalid")]
    ModelContract,
    /// Audio contains a non-finite sample.
    #[error("Audio contains a non-finite sample")]
    InvalidAudio,
    /// A worker panicked while it owned its state.
    #[error("Worker panicked: {0}")]
    WorkerPanicked(&'static str),
    /// A worker stopped without an error report.
    #[error("Worker stopped: {0}")]
    WorkerExited(&'static str),
}

/// A result that uses the Earshot error type.
pub type Result<T> = std::result::Result<T, Error>;
