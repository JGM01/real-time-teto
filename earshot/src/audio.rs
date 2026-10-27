//! Owned mono audio on the 16 kHz timeline.

use std::fmt;

/// Owned, contiguous `f32` samples declared as 16 kHz mono audio.
///
/// `start_sample` counts mono samples from the start of the source. The debug
/// form reports the bounds and length, never the sample values.
pub struct Audio {
    pub(crate) samples: Vec<f32>,
    pub(crate) start_sample: u64,
}

impl Audio {
    /// Borrow the contiguous mono samples.
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Return the number of mono samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Return whether the audio contains no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Return the first sample offset on the normalized source timeline.
    #[must_use]
    pub fn start_sample(&self) -> u64 {
        self.start_sample
    }

    /// Return the first sample offset after the audio.
    #[must_use]
    pub fn end_sample(&self) -> u64 {
        self.start_sample + self.samples.len() as u64
    }

    /// Return the owned samples.
    #[must_use]
    pub fn into_samples(self) -> Vec<f32> {
        self.samples
    }
}

impl fmt::Debug for Audio {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Audio")
            .field("start_sample", &self.start_sample)
            .field("len", &self.samples.len())
            .finish()
    }
}
