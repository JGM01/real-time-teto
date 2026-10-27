//! Native-free measurement definitions.
use crate::policy;
use std::time::Duration;

/// Audio metrics remain pending until a real output node exists.
pub const AUDIO_STATUS: &str = "pending: real TTS required";

/// Fixed prompts for a measurement run.
pub const PROMPTS: [&str; 4] = [
    "My name is Robin. Greet me briefly.",
    "What name did I give you?",
    "Explain a rainbow in one sentence.",
    "I meant a sunset. Correct your explanation briefly.",
];

/// One measurement run.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub minimum_turns: usize,
    pub duration: Duration,
    pub interrupt_every: Option<u64>,
    pub reset_every: Option<u64>,
    pub reopen_every: Option<u64>,
    pub asr_resident: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            minimum_turns: policy::WARM_TURNS,
            duration: Duration::from_secs(policy::SOAK_SECONDS),
            interrupt_every: None,
            reset_every: None,
            reopen_every: None,
            asr_resident: false,
        }
    }
}

impl Options {
    pub fn lifecycle() -> Self {
        Self {
            interrupt_every: Some(policy::SOAK_INTERRUPT_EVERY),
            reset_every: Some(policy::SOAK_RESET_EVERY),
            reopen_every: Some(policy::SOAK_REOPEN_EVERY),
            ..Self::default()
        }
    }

    pub fn phone() -> Self {
        Self {
            minimum_turns: policy::PHONE_SPIKE_TURNS,
            duration: Duration::from_secs(policy::PHONE_SPIKE_SECONDS),
            ..Self::default()
        }
    }
}

/// Nearest-rank percentile.
pub fn percentile(values: &[u64], percent: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let rank = (ordered.len() * percent).div_ceil(100).saturating_sub(1);
    ordered.get(rank).copied()
}
