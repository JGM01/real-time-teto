# LLM qualification evidence

Policy V1 is implemented and functionally validated. The desktop candidate misses the one-second first-clause p95 target when history eviction forces a rebuild. The Pro Max feasibility run also shows substantial rebuild latency and thermal pressure. Neither result establishes production audio qualification: real TTS, first audible sample, starvation and complete audio latency remain pending.

## Identity and method

Desktop: Mac15,7, Apple M3 Pro, 36 GiB, macOS 27.0.1. Native runtime: `llama-cpp-2` and `llama-cpp-sys-2` exactly 0.1.154, llama.cpp `5f55650a78f92aff4d48d671423e888fac0469ff`, Metal, full SWA cache. Rust 1.88.0. The exact Google Gemma 4 26B-A4B QAT Q4_0 artifact and template hashes are in the [canonical registry](../teto-llm-protocol/src/policy.rs). Both were validated before generation.

Sampling uses temperature one, top-k sixty-four, top-p point-nine-five and the specified SHA-256 per-turn seed derivation. Desktop measurement uses base seed zero; phone uses seven. Percentiles use nearest rank. Generation throughput includes EOS/suppressed-channel tokens and excludes prompt evaluation and queue waiting. First-clause timing measures text delivery, before display acknowledgment.

The parent contains Whisper/Earshot; the worker contains llama.cpp. [Built-symbol checks](qualification/native-isolation.json) and resolved dependency tests verify the separate trees. The real WAV test additionally measured **1.03 seconds of overlapping Whisper and LLM inference** in separate processes under Rust 1.88. This is independent of the latency measurements.

## Cache capability

[cache-m3-pro.json](qualification/cache-m3-pro.json) records six clean-baseline comparisons with approximately nineteen hundred cached tokens: append, normalization, token limit, interruption, eviction and forced failed removal. Each compared eight greedy continuation tokens and counted actual prompt evaluations. All continuations matched. A shorter-prefix run also passed. The probe completed within its four-hour limit.

This proves the cache operation for the checksummed desktop profile on the recorded hardware/OS. Prefix reuse is enabled only within that scope. Unverified platforms use FullRebuild. Eviction, reset and failed removal explicitly rebuild and report their reason and evaluated-token count. A rebuild is never reported as a cache hit. Phone prefix reuse remains unverified.

## Functional evidence

Formatting, 116 deterministic Rust 1.88 tests, Clippy with warnings denied and a seventy-line function limit, and rustdoc with warnings denied pass. Tests cover blocked producer/consumer cancellation, receipt/seal races, malformed/bounded IPC, exact budget arithmetic, chunk-boundary properties, AEAD tampering, maximum-size records, actual eight-segment capacity, pending-write retry, legacy byte-array frames and explicit resource setup/deletion. All 104 stable invariant IDs have enforcement/test references; deferred activation gates are checked explicitly. Loom and compile-fail suites are supplementary and are not present.

Seventeen real-model test cases were exercised separately from fast tests through serial and focused runs. These include persistent context, replacement, active close/reset, dropped consumers, played-prefix cancellation, actual token limit, portable cancellation measurement, encrypted durable finalization, authenticated reopening and wrong-key refusal. Token limit followed by playback interruption was finalized through an encrypted journal and followed by another prompt. Tests do not assert an exact generated sentence; one conversation asserts recall of a supplied name.

The WAV integration passed: existing WAV → Earshot → Whisper → private child → normalized clauses → recording receipts → finalized exchange. Silence admitted no LLM turn. The real CLI tests passed text follow-up/EOF, Ctrl-C during output, broken stdout with failed playback finalization and active child crash as service failure. The isolated Apple Keychain test passed and removed its temporary test entry. Default transcription starts no child or journal; text mode requires no ASR configuration.

The original counting prompt ended at EOS, so it could not prove the token limit. A longer prose request reached the actual 512-token limit. The first phone attempts failed normalization because the E2B artifact's generation prefix differs from the desktop template. Profiles now own distinct generation prefixes and validate exact template hashes; subsequent physical runs completed. These earlier failed reports are preserved with the successful evidence. Unplayed text was not written to diagnostics.

## Desktop performance

The [summary](qualification/summary.json), [turn metrics](qualification/desktop-final-run.log) and [resource samples](qualification/desktop-final-resources.json) record a real parent/client run with Whisper resident and an explicitly initialized Keychain-backed encrypted journal. Measurements contained no competing inference tests. The journal remains preserved at the location recorded in the resource report; no replay restored model context.

| Measurement | Observed |
|---|---:|
| Model-resident session | 260 turns, 1,804 seconds |
| First 100 warm turns, first-clause p50 / p95 | 463 ms / 8,980 ms |
| All turns, first-clause p50 / p95 | 8,706 ms / 9,324 ms |
| Verified-prefix cache-hit subset, p95 | 512 ms, 64 turns |
| Aggregate generation throughput | 27.39 tokens/second |
| Generation failures | Zero |
| Evictions applied to later prompts | 195 |
| Peak sampled parent-plus-child RSS | 16.24 GB / 15.12 GiB |
| Sampled ProcessInfo thermal state | Nominal throughout |
| Twenty cold process starts, first-clause p50 / p95 | 33.36 / 33.93 seconds |

Cold timing includes worker/model loading while ASR remains resident; OS page and shader caches are uncontrolled. The first hundred warm samples exclude the initial turn and include cache misses after eviction. Once the 64-exchange bound is reached, conservative prompt rebuilding dominates latency. The one-second p95 target is **missed**; the twenty-token/second aggregate generation target is met. Policy limits were retained.

RSS and thermal pressure were sampled every five seconds. Summed RSS can count shared pages twice and does not provide complete GPU allocation accounting; the reported peak is sampled. Thermal pressure is an OS category, not a measured temperature. `pmset` recorded no warning levels. Exact measurement executable hashes are stored in resource JSON; cold and warm runs have distinct build identities. Subsequent boundary hardening and the separate lifecycle run are tracked independently.

The final mixed lifecycle run completed **375 turns in 1,807 seconds**, including **37 planned playback interruptions**, reset after turn 128 and a clean process reopen after turn 256. Generation failures: **0**. The same encrypted journal survived reopening; model context started fresh. Peak sampled RSS was 15.12 GiB, with nominal and fair thermal pressure observed. First-hundred-warm p50/p95 were 424/9535 ms; aggregate generation throughput was 27.35 tokens/second. [Lifecycle turn records](qualification/desktop-lifecycle-run.log) and [resources/build identities](qualification/desktop-lifecycle-resources.json) preserve the evidence. This run includes the final boundary hardening and lifecycle renderer. Errors stop the harness; a crash never triggers automatic restart. The 8K-context and twelve-word-first-target variants remain unqualified.

## Physical phone feasibility

The same `teto-llm` crate ran directly on the authorized **iPhone 15 Pro Max**, iPhone16,2, iOS 27.0.1. This does not qualify the specified iPhone 15 Pro. The signed SwiftUI app and checksum-verified E2B model remain installed. Account credentials remain in Keychain; Xcode records the user-selected signing team in the project.

The five-minute harness explicitly disables journaling and uses tagged TestSink receipts, with no audio output or ASR. The two completed runs measured 58 turns in 308 seconds and 56 turns in 311 seconds. The latter includes resident-memory telemetry:

| Pro Max feasibility measurement | Observed |
|---|---:|
| Model load | 14.04 seconds |
| First-clause p50 / p95 | 5.70 / 10.09 seconds |
| Aggregate generation throughput | 18.78 tokens/second |
| Peak sampled resident memory | 2.05 GB / 1.91 GiB |
| Peak sampled physical footprint | 258 MB |
| Cancellation probe | 1.96 ms, immediately after admission |
| Cache mode | FullRebuild |
| Thermal states observed | Nominal, fair, serious |
| Run interruption / generation failure | None |

[Raw phone reports](qualification/phone-pro-max/) contain metrics and resource samples only. Resident memory and physical footprint account for mappings differently and do not establish total GPU/system memory use. The cancellation probe does not establish worst-case cancellation during an in-flight native batch. First-clause latency grows with prompt history under FullRebuild. These are feasibility results, with significant latency/thermal limitations; sustained production phone operation, the exact Pro device, phone prefix reuse and all audio gates remain pending.
