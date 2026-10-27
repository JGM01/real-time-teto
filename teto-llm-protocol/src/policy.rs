//! The single executable registry for Policy V1.

pub const VERSION: u32 = 1;
pub const CONTEXT_TOKENS: u32 = 4096;
pub const OUTPUT_TOKENS: u32 = 512;
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
pub const HISTORY_TURNS: usize = 64;
pub const HISTORY_BYTES: usize = 512 * 1024;
pub const FRAME_BYTES: usize = 128 * 1024;
pub const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
pub const SEGMENTS: u32 = 8;
pub const PREFILL_BATCH: usize = 64;
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
pub const RUNTIME: &str = "llama-cpp-2=0.1.154;llama.cpp=5f55650a78f92aff4d48d671423e888fac0469ff";

/// Render the canonical documentation table. Numerical rows use the constants above.
pub fn markdown() -> String {
    let mut rows = fixed_rows();
    rows.extend([
        ("Context", format!("{CONTEXT_TOKENS} tokens; minimum of configured/profile/runtime limits")),
        ("Output reserve", format!("{OUTPUT_TOKENS} generated tokens, including suppressed channels")),
        ("Clause queue / control queue", format!("{CLAUSE_QUEUE} / {CONTROL_QUEUE}; independent priority cancellation")),
        ("First clause", format!("target {FIRST_TARGET} words; cap {FIRST_WORDS} words and {FIRST_CHARS} Unicode scalar values")),
        ("Minimum clause", format!("prefer {MIN_WORDS} words or {MIN_CHARS} Unicode scalar values; natural short complete replies exempt")),
        ("Cache", format!("VerifiedPrefixReuse only for the checksummed desktop artifact on {PREFIX_REUSE_DEVICE}/macOS {PREFIX_REUSE_OS}, with recorded six-case proof; FullRebuild elsewhere and on eviction/reset/removal failure")),
        ("Later clause", format!("cap {LATER_WORDS} words and {LATER_CHARS} Unicode scalar values")),
        ("Input / system prompt", format!("{INPUT_BYTES} / {SYSTEM_BYTES} bytes")),
        ("Normalized turn / clauses", format!("{TURN_BYTES} bytes / {TURN_CLAUSES} clauses")),
        ("History", format!("token budget, {HISTORY_TURNS} exchanges, {HISTORY_BYTES} bytes; earliest bound wins")),
        ("Frame / record", format!("{FRAME_BYTES} bytes")),
        ("Journal capacity", format!("{SEGMENTS} segments of {SEGMENT_BYTES} bytes; no automatic deletion")),
        ("Prompt batch", format!("{PREFILL_BATCH} tokens between cancellation checks")),
        ("Child startup / shutdown", format!("{STARTUP_SECONDS} / {SHUTDOWN_SECONDS} seconds; explicit timeout failure")),
        ("Desktop model", format!("{PROFILE_ID}; {MODEL_FILE}; SHA-256 {MODEL_SHA256}; candidate, not qualified")),
        ("Runtime", RUNTIME.into()),
        ("Phone artifact", format!("{PHONE_PROFILE}; {PHONE_FILE}; SHA-256 {PHONE_SHA256}; spike only, not qualified")),
        ("Template identity", format!("desktop SHA-256 {DESKTOP_TEMPLATE_SHA256}; phone SHA-256 {PHONE_TEMPLATE_SHA256}; profile-owned direct generation prefixes")),
        ("Sampling", format!("temperature {SAMPLER_TEMP}; top-k {SAMPLER_TOP_K}; top-p {SAMPLER_TOP_P}; deterministic per-turn seed derivation")),
        ("Desktop measurement duration", format!("{WARM_TURNS} warm turns; {SOAK_SECONDS} second soak")),
        ("Lifecycle qualification", format!("interrupt after first displayed clause every {SOAK_INTERRUPT_EVERY} turns; reset every {SOAK_RESET_EVERY}; planned clean process reopen every {SOAK_REOPEN_EVERY}; errors stop the run")),
        ("Cold process measurement", format!("{COLD_START_RUNS} independently loaded children; OS page/shader caches uncontrolled; ASR remains resident")),
        ("Phone feasibility duration", format!("at least {PHONE_SPIKE_TURNS} turns and {PHONE_SPIKE_SECONDS} seconds; separate from device qualification")),
    ]);
    let mut output = String::from("| Decision | Policy V1 |\n|---|---|\n");
    for (name, value) in rows {
        output.push_str(&format!("| {name} | {} |\n", value.replace('|', "\\|")));
    }
    output
}

fn fixed_rows() -> Vec<(&'static str, String)> {
    [
        ("Architecture", "ASR/UI/playback parent; private LLM/journal child; same portable crate on phone"),
        ("Phone spike", "iPhone 15 Pro, 8 GB; Gemma 4 E2B candidate"),
        ("Profiles", "built-in immutable catalog; caller supplies assets; one model/context/producer"),
        ("Language / mode", "English; direct answer; thinking off; structural channel filtering"),
        ("Context builder", "exact template/tokenizer count plus output reserve; pin system/current user/reserve"),
        ("Eviction", "oldest finalized exchanges, optional memory, typed failure; no input truncation"),
        ("Receive", "blocking plus cancellation; one delivered unacknowledged clause"),
        ("Counting", "Unicode scalar values; normalized whitespace-separated words"),
        ("Receipts", "contiguous delivered prefix; accepted during generation and after end until sealing"),
        ("Finalization", "parent receipt fence; separate generation/playback stops; derived partial flag"),
        ("History content", "user and acknowledged complete clauses only; partial cancelled/token-limited prefixes retained"),
        ("Persistence", "journal only; fresh context/cache on restart; no retrieval/tree/view/compaction/replay"),
        ("Journal default", "enabled for LLM modes; explicit no-journal writes no files"),
        ("Key", "caller supplies zeroizing 256-bit key; Apple Keychain; never arguments/environment/logs"),
        ("Encryption", "XChaCha20-Poly1305; independent OS-random 192-bit nonce stored per record"),
        ("AAD", "journal ID, key ID, schema, segment, record ID, lengths"),
        ("Writer", "one writer and one encrypted pending record; durable append before successful finish"),
        ("Initialization", "explicit journal-init; ordinary startup never creates/replaces journal or key"),
        ("Journal failure", "fail startup/open; runtime closes admission and preserves current turn/pending ciphertext; no plaintext fallback"),
        ("Journal full", "reserve maximum record before admission; fail startup or stop after reserved turn; explicit new journal or no-journal restart"),
        ("Retry", "explicit only; same pending ciphertext/record ID; report pending record non-durability"),
        ("Deletion / rotation", "whole-journal crypto-shredding only; refuse active writer; rotation creates new journal; no per-turn deletion"),
        ("CLI", "TETO_MODE=text|voice|transcribe; default transcribe; shared clause renderer; print/flush then display receipt"),
        ("IPC", "private length-prefixed versioned CBOR pipes; bounded queues/credits; validate before allocation"),
        ("Crash", "stop admission; no automatic restart/replay; child failure is not cancellation"),
        ("Normalization", "markdown/whitespace/punctuation; omit code/tables/URLs/emoji; zero-to-twenty and small abbreviation table; digit spelling; unsupported numeric forms fail"),
        ("Variants", "8K context and twelve-word first target are separate qualification variants"),
        ("Qualification", "100 warm turns and thirty-minute soak; one-second clause/twenty tokens-per-second targets; audio gates pending real TTS; no fake audio"),
        ("Optional tests", "Loom and compile-fail supplementary; deterministic barriers required"),
    ].map(|(key, value)| (key, value.into())).into()
}
