//! PCM ring buffer configuration.
/// One PCM ring configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingConfig {
    /// Capacity in frames.
    pub capacity_frames: usize,
    /// Startup threshold in frames.
    pub start_threshold_frames: usize,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            capacity_frames: 5_760,
            start_threshold_frames: 1_920,
        }
    }
}
