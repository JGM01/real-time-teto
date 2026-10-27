/// A bounded speech failure.
#[derive(Debug, thiserror::Error)]
pub enum SpeechError {
    /// The input is empty, malformed or out of range.
    #[error("invalid speech input")]
    InvalidInput,
    /// A length, index or time bound failed.
    #[error("speech bound exceeded")]
    Bounds,
    /// The requested language or backend is not available.
    #[error("speech capability is not available")]
    Unavailable,
    /// A pronunciation cannot be produced.
    #[error("pronunciation failed")]
    Pronunciation,
    /// The OOV decoder exceeded its step limit.
    #[error("decoder limit reached")]
    DecoderLimit,
    /// A model artifact does not match its manifest.
    #[error("model mismatch")]
    ModelMismatch,
    /// A voicebank artifact is invalid.
    #[error("voicebank invalid")]
    VoicebankInvalid,
    /// No unit can render a target phone span.
    #[error("missing voicebank unit")]
    MissingUnit,
    /// Rendering failed.
    #[error("render failed")]
    Render,
    /// The device format is unsupported.
    #[error("unsupported audio format")]
    UnsupportedFormat,
    /// The caller cancelled the work.
    #[error("speech cancelled")]
    Cancelled,
    /// The output device was lost.
    #[error("audio device lost")]
    DeviceLost,
}

/// A speech result.
pub type Result<T> = std::result::Result<T, SpeechError>;
