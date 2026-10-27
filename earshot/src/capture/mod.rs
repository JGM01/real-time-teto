//! Native capture transport, source lifecycle, and normalization.

pub(crate) mod normalize;
pub(crate) mod pipeline;
pub(crate) mod ring;
pub(crate) mod source;

use crate::{
    Error, Result,
    constants::{SAMPLE_RATE_HZ, SUPPORTED_INPUT_RATES_HZ, VAD_FRAME_SAMPLES},
    runtime,
};
#[cfg(any(test, feature = "tooling"))]
use std::path::PathBuf;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
};

/// Replay timing. Accelerated replay pauses its sample clock during inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPacing {
    /// Advance the sample clock only when inference is idle.
    Accelerated,
    /// Advance the sample clock in real time.
    Realtime,
}

/// A concrete input for one activation. File chunk sizes count native frames.
#[derive(Debug)]
pub enum InputSource {
    /// Capture the default microphone.
    Microphone,
    /// Replay one WAV file through the normalization path.
    #[cfg(any(test, feature = "tooling"))]
    File {
        /// The WAV file to replay.
        path: PathBuf,
        /// Native frames per chunk.
        chunk_size: usize,
        /// The replay timing.
        pacing: ReplayPacing,
    },
    /// Replay owned 16 kHz mono samples through the normalization path.
    #[cfg(any(test, feature = "tooling"))]
    Samples {
        /// The samples to replay.
        audio: Arc<[f32]>,
        /// Native frames per chunk.
        chunk_size: usize,
        /// The replay timing.
        pacing: ReplayPacing,
    },
}

impl InputSource {
    /// Read a WAV through the same normalization path as microphone input.
    #[cfg(any(test, feature = "tooling"))]
    pub fn file(path: impl Into<PathBuf>, pacing: ReplayPacing) -> Self {
        Self::File {
            path: path.into(),
            chunk_size: crate::constants::VAD_FRAME_SAMPLES,
            pacing,
        }
    }

    /// Replay owned 16 kHz mono samples in complete voice activity frames.
    #[cfg(feature = "tooling")]
    pub fn samples(audio: Arc<[f32]>, pacing: ReplayPacing) -> Self {
        Self::Samples {
            audio,
            chunk_size: crate::constants::VAD_FRAME_SAMPLES,
            pacing,
        }
    }

    pub(crate) fn pacing(&self) -> ReplayPacing {
        match self {
            Self::Microphone => ReplayPacing::Realtime,
            #[cfg(any(test, feature = "tooling"))]
            Self::File { pacing, .. } | Self::Samples { pacing, .. } => *pacing,
        }
    }

    pub(crate) fn is_realtime(&self) -> bool {
        self.pacing() == ReplayPacing::Realtime
    }
}

/// Activation-local control. No source has access to the program shutdown flag.
#[derive(Clone)]
pub(crate) struct SourceContext {
    pub cancel: Arc<AtomicBool>,
    #[cfg_attr(
        not(any(test, feature = "tooling")),
        allow(dead_code, reason = "replay pacing")
    )]
    pub pacing: ReplayPacing,
}

impl SourceContext {
    pub(crate) fn new(cancel: Arc<AtomicBool>, pacing: ReplayPacing) -> Self {
        Self { cancel, pacing }
    }

    pub(crate) fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    pub(crate) fn wait_until_listening(&self) -> bool {
        !self.cancel.load(Ordering::Acquire)
    }

    pub(crate) fn spawn_audio(
        &self,
        body: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<JoinHandle<Result<()>>> {
        runtime::spawn("earshot-capture", body)
    }

    #[cfg(test)]
    pub(crate) fn test_context() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), ReplayPacing::Accelerated)
    }
}

/// Native capture progress rejects predictions made before queued speech.
#[derive(Clone)]
pub(crate) struct Watermark {
    pub frames: Arc<AtomicU64>,
    pub rate_hz: u32,
}

impl Watermark {
    /// Return the captured 16 kHz samples, rounded up to a VAD frame boundary.
    #[must_use]
    pub(crate) fn complete_sample(&self) -> u64 {
        let frames = self.frames.load(Ordering::Acquire);
        let samples = frames * u64::from(SAMPLE_RATE_HZ);
        let frame = u64::from(self.rate_hz) * VAD_FRAME_SAMPLES as u64;
        samples.div_ceil(frame) * VAD_FRAME_SAMPLES as u64
    }
}

/// Check the native sample rate and channel count.
///
/// # Errors
///
/// Returns `Error::UnsupportedInputFormat` for an unsupported rate or a channel
/// count other than one or two.
pub(crate) fn validate_format(rate_hz: u32, channels: u16) -> Result<()> {
    if !SUPPORTED_INPUT_RATES_HZ.contains(&rate_hz) || !matches!(channels, 1 | 2) {
        return Err(Error::UnsupportedInputFormat);
    }
    Ok(())
}
