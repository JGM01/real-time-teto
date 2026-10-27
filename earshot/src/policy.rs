//! Fixed development policy. The values are provisional.

use crate::constants::SAMPLE_RATE_HZ;

/// Endpoint policy in samples on the 16 kHz mono timeline.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Policy {
    pub pause_samples: u64,
    pub threshold: f32,
    pub no_speech_samples: u64,
    pub silence_limit_samples: u64,
    pub maximum_samples: usize,
}

impl Default for Policy {
    fn default() -> Self {
        const SAMPLE_RATE: u64 = SAMPLE_RATE_HZ as u64;
        Self {
            pause_samples: 1_024 * SAMPLE_RATE / 1_000,
            threshold: 0.01,
            no_speech_samples: 5 * SAMPLE_RATE,
            silence_limit_samples: 5 * SAMPLE_RATE,
            maximum_samples: 300 * SAMPLE_RATE as usize,
        }
    }
}
