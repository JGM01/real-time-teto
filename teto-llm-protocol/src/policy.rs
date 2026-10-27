//! Fixed policy values.

pub const VERSION: u32 = 2;

pub const CONTEXT_TOKENS: u32 = 4096;
pub const OUTPUT_TOKENS: u32 = 512;
pub const HISTORY_BYTES: usize = 512 * 1024;
pub const HISTORY_LOW_WATER_PERCENT: u32 = 70;

pub const CLAUSE_QUEUE: usize = 2;
pub const CONTROL_QUEUE: usize = 4;
pub const FIRST_TARGET: usize = 8;
pub const FIRST_WORDS: usize = 12;
pub const FIRST_CHARS: usize = 96;
pub const MIN_WORDS: usize = 3;
pub const MIN_CHARS: usize = 12;
pub const LATER_WORDS: usize = 40;
pub const LATER_CHARS: usize = 240;

pub const INPUT_BYTES: usize = 16 * 1024;
pub const SYSTEM_BYTES: usize = 4 * 1024;
pub const TURN_BYTES: usize = 64 * 1024;
pub const TURN_CLAUSES: usize = 512;
pub const FRAME_BYTES: usize = 128 * 1024;
pub const RECORD_BYTES: usize = FRAME_BYTES - 1024;
pub const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
pub const SEGMENTS: u32 = 8;

pub const PREFILL_CHUNK: usize = 512;
pub const STARTUP_SECONDS: u64 = 120;
pub const SHUTDOWN_SECONDS: u64 = 5;

pub const WARM_TURNS: usize = 100;
pub const COLD_START_RUNS: usize = 20;
pub const SOAK_SECONDS: u64 = 1800;
pub const SOAK_INTERRUPT_EVERY: u64 = 10;
pub const SOAK_RESET_EVERY: u64 = 128;
pub const SOAK_REOPEN_EVERY: u64 = 256;
pub const PHONE_SPIKE_TURNS: usize = 10;
pub const PHONE_SPIKE_SECONDS: u64 = 300;

pub const SAMPLER_TOP_K: i32 = 64;
pub const SAMPLER_TOP_P: f32 = 0.95;
pub const SAMPLER_TEMP: f32 = 1.0;

pub const MODEL_FILE: &str = "gemma-4-26B_q4_0-it.gguf";
pub const MODEL_SHA256: &str = "3eca3b8f6d7baf218a7dd6bba5fb59a56ee25fe2d567b6f5f589b4f697eca51d";
pub const PROFILE_ID: &str = "gemma4-26b-qat-q4_0-macos-v1";
pub const PHONE_PROFILE: &str = "gemma4-e2b-qat-q4_0-ios-spike-v1";
pub const PHONE_FILE: &str = "gemma-4-E2B_q4_0-it.gguf";
pub const PHONE_SHA256: &str = "fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634";
pub const DESKTOP_TEMPLATE_SHA256: &str =
    "ae53464bf3be25802b3a5b37def7fd89667067d7577049b3b2d74c4d8de4c6d4";
pub const PHONE_TEMPLATE_SHA256: &str =
    "0a2c8073c878ab1da004bee933a998606537bbb62016310352c7285c3f01c5b5";
pub const PREFIX_REUSE_DEVICE: &str = "Mac15,7";
pub const PREFIX_REUSE_OS: &str = "27.0.1";
pub const PHONE_PREFIX_REUSE_DEVICE: &str = "iPhone16,2";
pub const PHONE_PREFIX_REUSE_OS: &str = "27.0.1";
pub const RUNTIME: &str = "llama-cpp-2=0.1.154;llama.cpp=5f55650a78f92aff4d48d671423e888fac0469ff";
