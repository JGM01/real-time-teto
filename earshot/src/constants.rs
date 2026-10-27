//! Fixed rates, sizes, and timing limits.
//!
//! Sample counts use the 16 kHz mono timeline.

/// The normalized mono sample rate used by every delivered audio value.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const VAD_FRAME_SAMPLES: usize = 512;
pub const CAPTURE_BUFFER_MS: usize = 2_048;
pub const SUPPORTED_INPUT_RATES_HZ: [u32; 10] = [
    8_000, 16_000, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
];
pub const NORMALIZATION_BLOCK_SAMPLES: usize = 160;
#[cfg(all(test, not(debug_assertions)))]
pub const NORMALIZATION_BLOCK_BUDGET_MS: u64 = 2;
#[cfg(all(test, not(debug_assertions)))]
pub const NORMALIZATION_BUFFERING_BUDGET_MS: u64 = 32;

pub const MODEL_SAMPLES: usize = 128_000;
pub const PRE_SPEECH_SAMPLES: usize = 4_096;
pub const TRAILING_SAMPLES: usize = 4_096;
pub const PREROLL_FRAMES: usize = PRE_SPEECH_SAMPLES / VAD_FRAME_SAMPLES + 2;
/// The maximum trace length in samples: 308 seconds.
pub const TRACE_SAMPLES: usize = 308 * SAMPLE_RATE_HZ as usize;

pub const SCORED_CAPACITY: usize = 16;
pub const COMMAND_CAPACITY: usize = 8;
pub const EVENT_CAPACITY: usize = 2;
pub const MODEL_CAPACITY: usize = 1;
pub const EXIT_CAPACITY: usize = 2;
pub const STARTUP_CHANNEL_CAPACITY: usize = 1;
#[cfg(feature = "tooling")]
pub const TRACE_CAPACITY: usize = 2_048;
pub const QUEUE_WAIT_MS: u64 = 10;
pub const RING_POLL_MS: u64 = 2;

const _: () = assert!(SAMPLE_RATE_HZ.is_multiple_of(1_000));
const _: () = assert!(PRE_SPEECH_SAMPLES.is_multiple_of(VAD_FRAME_SAMPLES));
const _: () = assert!(TRAILING_SAMPLES.is_multiple_of(VAD_FRAME_SAMPLES));
const _: () = assert!(MODEL_SAMPLES.is_multiple_of(VAD_FRAME_SAMPLES));
