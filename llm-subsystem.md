# LLM Subsystem Contract

Status: implementation contract, Policy V1. No model or device is qualified yet.

Complete user text and its truncation flag enter the subsystem. Owned English clauses leave it. The parent owns Earshot, Whisper, UI and eventual playback. A private child owns the LLM, model context, cache, history and encrypted journal. TTS is absent.

## Policy V1

The policy registry in `teto-llm-protocol/src/policy.rs` is the executable source for this table. The contract test checks that the table matches it. Change a fixed policy only with a new version.

<!-- POLICY:BEGIN -->
| Decision | Policy V1 |
|---|---|
| Architecture | ASR/UI/playback parent; private LLM/journal child; same portable crate on phone |
| Phone spike | iPhone 15 Pro, 8 GB; Gemma 4 E2B candidate |
| Profiles | built-in immutable catalog; caller supplies assets; one model/context/producer |
| Language / mode | English; direct answer; thinking off; structural channel filtering |
| Context builder | exact template/tokenizer count plus output reserve; pin system/current user/reserve |
| Eviction | oldest finalized exchanges, optional memory, typed failure; no input truncation |
| Receive | blocking plus cancellation; one delivered unacknowledged clause |
| Counting | Unicode scalar values; normalized whitespace-separated words |
| Receipts | contiguous delivered prefix; accepted during generation and after end until sealing |
| Finalization | parent receipt fence; separate generation/playback stops; derived partial flag |
| History content | user and acknowledged complete clauses only; partial cancelled/token-limited prefixes retained |
| Persistence | journal only; fresh context/cache on restart; no retrieval/tree/view/compaction/replay |
| Journal default | enabled for LLM modes; explicit no-journal writes no files |
| Key | caller supplies zeroizing 256-bit key; Apple Keychain; never arguments/environment/logs |
| Encryption | XChaCha20-Poly1305; independent OS-random 192-bit nonce stored per record |
| AAD | journal ID, key ID, schema, segment, record ID, lengths |
| Writer | one writer and one encrypted pending record; durable append before successful finish |
| Initialization | explicit journal-init; ordinary startup never creates/replaces journal or key |
| Journal failure | fail startup/open; runtime closes admission and preserves current turn/pending ciphertext; no plaintext fallback |
| Journal full | reserve maximum record before admission; fail startup or stop after reserved turn; explicit new journal or no-journal restart |
| Retry | explicit only; same pending ciphertext/record ID; report pending record non-durability |
| Deletion / rotation | whole-journal crypto-shredding only; refuse active writer; rotation creates new journal; no per-turn deletion |
| CLI | TETO_MODE=text\|voice\|transcribe; default transcribe; shared clause renderer; print/flush then display receipt |
| IPC | private length-prefixed versioned CBOR pipes; bounded queues/credits; validate before allocation |
| Crash | stop admission; no automatic restart/replay; child failure is not cancellation |
| Normalization | markdown/whitespace/punctuation; omit code/tables/URLs/emoji; zero-to-twenty and small abbreviation table; digit spelling; unsupported numeric forms fail |
| Variants | 8K context and twelve-word first target are separate qualification variants |
| Qualification | 100 warm turns and thirty-minute soak; one-second clause/twenty tokens-per-second targets; audio gates pending real TTS; no fake audio |
| Optional tests | Loom and compile-fail supplementary; deterministic barriers required |
| Context | 4096 tokens; minimum of configured/profile/runtime limits |
| Output reserve | 512 generated tokens, including suppressed channels |
| Clause queue / control queue | 2 / 4; independent priority cancellation |
| First clause | target 8 words; cap 12 words and 96 Unicode scalar values |
| Minimum clause | prefer 3 words or 12 Unicode scalar values; natural short complete replies exempt |
| Cache | VerifiedPrefixReuse only for the checksummed desktop artifact on Mac15,7/macOS 27.0.1, with recorded six-case proof; FullRebuild elsewhere and on eviction/reset/removal failure |
| Later clause | cap 40 words and 240 Unicode scalar values |
| Input / system prompt | 16384 / 4096 bytes |
| Normalized turn / clauses | 65536 bytes / 512 clauses |
| History | token budget, 64 exchanges, 524288 bytes; earliest bound wins |
| Frame / record | 131072 bytes |
| Journal capacity | 8 segments of 8388608 bytes; no automatic deletion |
| Prompt batch | 64 tokens between cancellation checks |
| Child startup / shutdown | 120 / 5 seconds; explicit timeout failure |
| Desktop model | gemma4-26b-qat-q4_0-macos-v1; gemma-4-26B_q4_0-it.gguf; SHA-256 3eca3b8f6d7baf218a7dd6bba5fb59a56ee25fe2d567b6f5f589b4f697eca51d; candidate, not qualified |
| Runtime | llama-cpp-2=0.1.154;llama.cpp=5f55650a78f92aff4d48d671423e888fac0469ff |
| Phone artifact | gemma4-e2b-qat-q4_0-ios-spike-v1; gemma-4-E2B_q4_0-it.gguf; SHA-256 fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634; spike only, not qualified |
| Template identity | desktop SHA-256 ae53464bf3be25802b3a5b37def7fd89667067d7577049b3b2d74c4d8de4c6d4; phone SHA-256 0a2c8073c878ab1da004bee933a998606537bbb62016310352c7285c3f01c5b5; profile-owned direct generation prefixes |
| Sampling | temperature 1; top-k 64; top-p 0.95; deterministic per-turn seed derivation |
| Desktop measurement duration | 100 warm turns; 1800 second soak |
| Lifecycle qualification | interrupt after first displayed clause every 10 turns; reset every 128; planned clean process reopen every 256; errors stop the run |
| Cold process measurement | 20 independently loaded children; OS page/shader caches uncontrolled; ASR remains resident |
| Phone feasibility duration | at least 10 turns and 300 seconds; separate from device qualification |
<!-- POLICY:END -->

## Ownership and public surface

`teto-llm` contains the portable engine and private `teto-llm-worker` binary. `teto-llm-protocol` contains native-free types and framed CBOR messages. The parent never links llama.cpp; the child never links Whisper or Earshot. No backend trait, model mutex, audio types, phonemes or token IDs cross this boundary. No global tracing subscriber is installed by either library. No network inference exists.

Checked constructors protect UserInput, TurnId, ClauseIndex and SpeakableText. Clauses own immutable UTF-8. Errors expose a bounded ErrorKind and static diagnostic message. Text and keys have redacted Debug output. One dedicated worker owns inference. All queues and text buffers are bounded.

Generation and playback have independent outcomes. GenerationStop is Eos, TokenLimit, Cancelled or Failed. PlaybackStop is Complete, Interrupted or Failed. Empty output means zero clauses. A finalized record stores both outcomes and derives partial from generation != Eos or playback != Complete.

## Receipts and finalization

The orchestrator owns finalization. Turn::acknowledge accepts receipts during generation and after generation end, until sealing. Only the contiguous delivered prefix can be acknowledged. Duplicate acknowledgments are idempotent. Skipped, future, undelivered, wrong-turn and post-seal receipts fail.

Normal completion drains generation, waits for output completion, drains receipts, and calls finish_turn(Complete). Interruption cancels generation, stops old output submissions, establishes an output receipt fence outside the audio callback, drains receipts, then calls finish_turn(Interrupted or Failed). Admission resumes only after durable finalization. The CLI uses synchronous successful print-and-flush as a display receipt and fence.

start_turn cancels an unsettled predecessor and returns PlaybackPending. It never silently seals one. Reset and close require the same barrier. A terminal event already observed is never replaced. Cancellation discards unread clauses and wakes blocked producer and consumer. Terminal state does not consume a clause slot.

The library also exposes finish_playback with the retained turn ID so an orchestrator can finalize after its consumer handle was dropped. close takes a mutable borrow and preserves the owner on PlaybackPending, allowing the caller to finish and retry.

## Context and normalization

The context builder alone renders and tokenizes prompts. It enforces prompt_tokens + output_reserve <= min(configured, profile, runtime context). The system instruction, current user and reserve are pinned. Evict oldest whole finalized exchanges, then optional memory when that future feature exists. Oversized pinned input fails without silent truncation.

Keep user text and acknowledged complete assistant clauses, including partial cancelled and token-limited turns. Keep both outcome axes. Unplayed text may exist in a bounded transient ledger but never enters committed history or disk. Do not retain reasoning or discarded tails.

V1 strips markdown wrappers, code, tables, URLs and emoji; normalizes whitespace and typographic punctuation; expands zero through twenty and a small abbreviation table; and spells larger unsigned integers as digits. Currency, ordinal, decimal, date and unit syntax fails closed. The model is instructed to write such expressions in words. Broader normalization waits for real TTS. Segmentation protects lexical units, preserves punctuation, and uses both word and character caps. Natural short complete replies may end early. Token-limit tails are dropped.

## Native and cache gates

Shared GGML is not a delivery prerequisite. whisper-rs-sys 0.15.0 exposes WHISPER_USE_SYSTEM_GGML but bundles GGML 0.9.5. llama-cpp-2 0.1.154 pins llama.cpp 5f55650a78f92aff4d48d671423e888fac0469ff with GGML 0.18.0. Compatibility is unproven. Any reconsideration has a four-engineer-hour limit and must prove API compatibility, one linked implementation, and inference in both load orders. Stop on failure. Do not patch kernels, use dynamic isolation or rewrite ASR.

The pinned Rust API exposes clear_kv_cache_seq returning Result<bool>, clear_kv_cache, and decode. Partial removal can fail. Use FullRebuild until the exact model passes a four-hour prefix-reuse probe against clean greedy baselines for append, normalized history, token limits, interruption, eviction and removal failure. Failed removal rebuilds explicitly. History correctness must not depend on cache capability. No cache is persisted across restarts.

## Journal lifecycle

The library receives a caller-supplied zeroizing 256-bit key. Apple binaries fetch it from Keychain and send it over a private pipe, never arguments or environment. One key belongs to one journal. A frame uses fresh OS-random 192-bit nonce and per-record XChaCha20-Poly1305. AAD covers journal ID, key ID, schema, segment, record ID and lengths. Retry the same pending ciphertext unchanged. There is no sequence reservation or secondary key.

Explicit setup creates journal identity, authenticated header and Keychain entry. Ordinary startup never creates or replaces either. Missing, corrupt, mismatched or inaccessible resources fail startup. Initialization rollback removes only resources created by that attempt.

Reserve one maximum-size record before admission. Preserve all segments. A full journal fails startup or stops admission after the reserved current turn. The CLI reports JournalFull, closes and exits nonzero. Continue only by explicitly selecting/initializing another journal or restarting without journaling. No automatic deletion occurs.

Write failure retains the current turn and one encrypted pending record, stops admission and surfaces the error. Only explicit retry is allowed. If disk cannot accept pending ciphertext, retain it in bounded memory and report non-durability. Successful finish requires durable append acknowledgment.

The writer first syncs the encrypted pending frame to pending.cbor, then appends and syncs the segment, then removes the pending artifact. Runtime retry reuses the exact frame and record ID. An unresolved pending artifact fails subsequent startup closed: V1 neither replays nor silently retries it. Preserve the artifact for diagnosis; explicit whole-journal deletion remains available.

Deletion is whole-journal only: refuse an active writer, delete its Keychain key and selected ciphertext files. Per-turn deletion and tombstones are unsupported. Crypto-shredding assumes no retained key copies; clear live key material. Key rotation creates a new journal.

## CLI and qualification

Explicit modes are text, voice and transcribe. Transcribe remains the default. Text needs no microphone or Whisper configuration. Voice uses SPACE capture. Both LLM modes use the same renderer and child client. Print clauses, flush, then acknowledge. Report generation stop, playback stop, truncation and partial status as metadata. EOF and Ctrl-C finalize and close.

The iPhone feasibility harness uses the same library directly on iPhone 15 Pro (8 GB). Measure build/link, load, memory, first clause, cancellation, cache and thermal behavior. Device and runtime results are unqualified until measured.

Qualify text-to-clauses with real-model tests separate from fast tests. Record cold/warm p50/p95, prompt evaluations, cache mode, throughput, aggregate memory and thermal behavior with Whisper resident. Use 100 measured warm turns and a thirty-minute soak. One second and twenty tokens/second are targets, not results. Real audio latency, first audible sample, starvation and TTS integration remain pending. Never use beep or synthetic audio as qualification.

## Stable invariants and traceability

Original IDs are preserved below. Amendments supersede the original wording where indicated. Deferred tests are activation gates, not claims of V1 coverage. All other rows require executable evidence.

<!-- INVARIANTS:BEGIN -->
| ID | Contract | Enforcement | Test | Status |
|---|---|---|---|---|
| LLM-001 | One active turn per `Llm`. | `engine/mailbox` | `one_active_turn` | Required |
| LLM-002 | One model context per `Llm`. | `engine/mailbox` | `one_context_lifetime` | Required |
| LLM-003 | At most one clause producer per `Llm`. | `engine/mailbox` | `one_clause_producer` | Required |
| LLM-004 | start_turn requests cancellation and refuses admission until the previous receipt barrier is sealed. | `engine/mailbox` | `replacement_cancels_before_admission` | Required; amended |
| LLM-005 | An old stream not already observed as terminal ends Cancelled exactly once. | `engine/mailbox` | `replacement_terminal_once` | Required; amended |
| LLM-006 | `reset_conversation` cancels the active turn. | `engine/mailbox` | `reset_active_turn` | Required |
| LLM-007 | `close` cancels the active turn. | `engine/mailbox` | `close_active_turn` | Required |
| LLM-008 | Clauses arrive in order. | `engine/mailbox` | `clause_order` | Required |
| LLM-009 | `Clause::index` is monotonic within a turn. | `engine/mailbox` | `clause_indices` | Required |
| LLM-010 | Every clause carries the `TurnId`. | `engine/mailbox` | `every_clause_has_turn` | Required |
| LLM-011 | No clause arrives after `End` or `Cancelled`. | `engine/mailbox` | `no_clause_after_terminal` | Required |
| LLM-012 | A call after `End` or `Cancelled` returns an error. | `engine/mailbox` | `receive_after_terminal_errors` | Required |
| LLM-013 | A stale `TurnId` is dropped downstream. | `engine/mailbox` | `stale_clause_dropped` | Required |
| LLM-014 | Cancellation is not an error. | `engine/mailbox` | `cancel_is_not_failure` | Required |
| LLM-015 | `next_clause` blocks until a clause, an end, or cancellation. | `engine/mailbox` | `receive_waits_and_wakes` | Required |
| LLM-016 | `TurnControl` can cancel from another thread. | `engine/mailbox` | `cross_thread_cancel` | Required |
| LLM-017 | The LLM never runs in the audio callback. | `engine/mailbox` | `inference_thread_identity` | Required |
| LLM-018 | The LLM never writes the audio ring. | `engine/mailbox` | `native_and_audio_isolation` | Required |
| LLM-019 | A cancelled assistant turn is never committed as complete; acknowledged prefixes may be partial. | `engine/mailbox` | `cancelled_never_complete` | Required; amended |
| LLM-020 | Context is bounded by tokens. | `protocol/types/policy` | `context_budget_bound` | Required |
| LLM-021 | Output is bounded by tokens. | `protocol/types/policy` | `output_token_bound` | Required |
| LLM-022 | Clause length is bounded. | `protocol/types/policy` | `clause_size_bounds` | Required |
| LLM-023 | Clause queue capacity is bounded. | `protocol/types/policy` | `slow_consumer_bound` | Required |
| LLM-024 | Token IDs never cross the boundary. | `protocol/types/policy` | `protocol_surface` | Required |
| LLM-025 | Phonemes never cross the boundary. | `protocol/types/policy` | `protocol_surface` | Required |
| LLM-026 | Audio never crosses the boundary. | `protocol/types/policy` | `protocol_surface` | Required |
| LLM-027 | Clause text is owned UTF-8 and is not mutated after emission. | `protocol/types/policy` | `clause_ownership_utf8` | Required |
| LLM-028 | Clause text is speakable text. | `protocol/types/policy` | `normalization_v1` | Required |
| LLM-029 | Output language is fixed English; real TTS language compatibility is pending. | `protocol/types/policy` | `unsupported_language_rejected` | Required; amended |
| LLM-030 | Diagnostic logs do not contain response text by default. | `engine/mailbox` | `response_sentinel_not_logged` | Required |
| LLM-031 | `TurnId` is monotonic per `Llm` and never reused. | `engine/mailbox` | `turn_id_reset_and_overflow` | Required |
| LLM-032 | Dropping the active `Turn` cancels generation. | `engine/mailbox` | `drop_active_turn` | Required |
| LLM-033 | `TurnControl::cancel` is idempotent. | `engine/mailbox` | `repeated_cancel` | Required |
| LLM-034 | One active model profile per `Llm`. | `profile/text/metrics` | `single_profile_lifetime` | Required |
| LLM-035 | A profile is validated before load. | `profile/text/metrics` | `invalid_profile_prevents_load` | Required |
| LLM-036 | Direct-answer mode is the default. | `profile/text/metrics` | `direct_answer_template` | Required |
| LLM-037 | Thinking is off unless the profile enables it. | `profile/text/metrics` | `thinking_disabled_template` | Required |
| LLM-038 | Reasoning and control tokens never enter clauses. | `profile/text/metrics` | `reasoning_and_control_suppressed` | Required |
| LLM-039 | Stop tokens come from the profile. | `profile/text/metrics` | `profile_stop_tokens` | Required |
| LLM-040 | Reuse cached prefixes only for a profile with verified capability and valid state. | `profile/text/metrics` | `cache_capability_gate` | Required; amended |
| LLM-041 | A verified cache hit does not reevaluate its proven prefix; FullRebuild is an explicit measured miss. | `profile/text/metrics` | `cache_evaluated_token_counts` | Required; amended |
| LLM-042 | Context eviction is measured. | `profile/text/metrics` | `eviction_metrics` | Required |
| LLM-043 | Model, runtime, and device qualify as a triple. | `profile/text/metrics` | `qualification_identity` | Required |
| LLM-044 | First answer token and first usable clause are separate metrics. | `profile/text/metrics` | `answer_and_clause_timestamps` | Required |
| LLM-045 | Decode starts only when `prompt_tokens + max_output_tokens <= context_tokens`. | `context/finalization` | `exact_fit_and_one_token_over` | Required |
| LLM-046 | Prompt token count uses the profile tokenizer and chat template. | `context/finalization` | `rendered_prompt_token_count` | Required |
| LLM-047 | The system prompt and current user turn are pinned. | `context/finalization` | `pinned_content_preserved` | Required |
| LLM-048 | Evict oldest whole finalized exchanges, including finalized partial exchanges. | `context/finalization` | `exchange_eviction_order` | Required; amended |
| LLM-049 | Optional memory is evicted before the current user turn. | `context/finalization` | `optional_memory_eviction` | Deferred activation gate |
| LLM-050 | If pinned content cannot fit, the turn fails with a typed error. | `context/finalization` | `oversized_pinned_prompt_error` | Required |
| LLM-051 | The current user turn is not silently truncated. | `context/finalization` | `oversized_input_not_truncated` | Required |
| LLM-052 | Context overflow is impossible by construction. | `context/finalization` | `decode_requires_budgeted_prompt` | Required |
| LLM-053 | Output-token limit remains possible. | `context/finalization` | `token_limit_with_context_headroom` | Required |
| LLM-054 | Persistent memory uses the same context budget. | `context/finalization` | `memory_shares_budget` | Deferred activation gate |
| LLM-055 | Dynamic memory stays in the uncached suffix. | `context/finalization` | `memory_suffix_order` | Deferred activation gate |
| LLM-056 | Persistent memory does not change the clause boundary. | `context/finalization` | `protocol_surface` | Required |
| LLM-057 | The context builder is the only prompt assembler. | `context/finalization` | `single_prompt_assembler` | Required |
| LLM-058 | Prompt assembly is separate from generation. | `context/finalization` | `builder_does_not_decode` | Required |
| LLM-059 | Effective context is the minimum of profile and runtime limits. | `context/finalization` | `effective_context_limit` | Required |
| LLM-060 | A tokenizer or template mismatch is a profile error. | `profile/text/metrics` | `profile_template_mismatch` | Required |
| LLM-061 | Assistant history stores played clauses only. | `context/finalization` | `unacknowledged_not_in_history` | Required |
| LLM-062 | The persistent log stores user text and played assistant clauses. | `journal/CLI` | `journal_contains_user_and_played_prefix` | Required |
| LLM-063 | A token-limited turn is marked in the persistent log. | `journal/CLI` | `token_limit_journal_record` | Required |
| LLM-064 | A cancelled/interrupted turn stores independent generation/playback outcomes and partial=true. | `journal/CLI` | `interrupted_journal_record` | Required; amended |
| LLM-065 | Unplayed assistant text never enters committed history or persistent storage; bounded transient staging is allowed. | `journal/CLI` | `unplayed_sentinel_absent` | Required; amended |
| LLM-066 | Persistent memory is encrypted at rest. | `journal/CLI` | `journal_aead_roundtrip_and_tamper` | Required |
| LLM-067 | User deletion is supported. | `journal/CLI` | `journal_delete_and_key_loss` | Required |
| LLM-068 | Context is bounded; Policy V1 also caps disk storage without automatic deletion. | `journal/CLI` | `journal_capacity_preserves_records` | Required; amended |
| LLM-069 | One writer owns a chat log. | `journal/CLI` | `second_writer_rejected` | Required |
| LLM-070 | View and tree writes are atomic and checksummed. | `journal/CLI` | `atomic_view_tree_write` | Deferred activation gate |
| LLM-071 | Diagnostic logs and persistent memory logs are separate stores. | `journal/CLI` | `diagnostics_are_not_journal` | Required |
| LLM-072 | Compaction runs outside the audio callback. | `journal/CLI` | `compaction_thread_identity` | Deferred activation gate |
| LLM-073 | Compaction does not block a live turn. | `journal/CLI` | `compaction_does_not_block_turn` | Deferred activation gate |
| LLM-074 | In production, assistant text enters history only after complete-clause playback acknowledgment. | `mailbox/parent orchestrator` | `receipt_authorizes_commit` | Required |
| LLM-075 | The CLI test harness substitutes display acknowledgment for playback acknowledgment. | `mailbox/parent orchestrator` | `display_receipt_commit_path` | Required |
| LLM-076 | The orchestrator drains output receipts through a fence before sealing. | `mailbox/parent orchestrator` | `finish_waits_for_receipt_fence` | Required; amended |
| LLM-077 | The audio callback never blocks on acknowledgment. | `engine/mailbox` | `receipt_handoff_never_blocks_callback` | Deferred activation gate |
| LLM-078 | The first persistence implementation is journal-only. | `journal/CLI` | `unsupported_memory_operations_rejected` | Required |
| LLM-079 | The first persistence implementation does not replay into context. | `journal/CLI` | `journal_not_replayed` | Required |
| LLM-080 | A journal key is supplied by the caller. | `journal/CLI` | `key_required_by_library` | Required |
| LLM-081 | Apple binaries obtain the journal key from Keychain. | `journal/CLI` | `keychain_adapter` | Required |
| LLM-082 | No plaintext journal fallback exists. | `journal/CLI` | `no_plaintext_fallback` | Required |
| LLM-083 | A journal open failure fails startup when the journal is enabled. | `journal/CLI` | `journal_open_failure` | Required |
| LLM-084 | A journal write failure stops new turns. | `journal/CLI` | `write_failure_stops_turns` | Required |
| LLM-085 | A no-journal mode is explicit and user-selected. | `journal/CLI` | `explicit_no_journal_writes_nothing` | Required |
| LLM-086 | Silent journal disable is forbidden. | `journal/CLI` | `failure_never_disables_journal` | Required |
| LLM-087 | A cancelled turn may commit complete played clauses as a partial assistant turn. | `context/finalization` | `cancelled_played_prefix` | Required |
| LLM-088 | Partial cancelled turns enter context with independent outcome axes and partial=true. | `context/finalization` | `cancelled_prefix_next_prompt` | Required; amended |
| LLM-089 | Token-limited turns enter context with generation_stop=TokenLimit, independently of playback_stop. | `context/finalization` | `token_limit_next_prompt` | Required; amended |
| LLM-090 | Journal initialization is explicit. | `journal/CLI` | `explicit_initialization_only` | Required |
| LLM-091 | Ordinary startup never creates or replaces a key. | `journal/CLI` | `startup_never_creates_or_replaces_key` | Required |
| LLM-092 | An existing journal with a missing key fails. | `journal/CLI` | `existing_journal_missing_key` | Required |
| LLM-093 | An existing key with no journal fails. | `journal/CLI` | `existing_key_missing_journal` | Required |
| LLM-094 | Receipts are accepted during generation and after generation end, until sealing. | `mailbox/parent orchestrator` | `ack_during_decode_and_after_end` | Required |
| LLM-095 | Finalized records preserve generation stop and playback stop independently. | `protocol/types/policy` | `token_limit_then_playback_interrupted` | Required |
| LLM-096 | Sealed records reject late receipts. | `mailbox/parent orchestrator` | `late_receipt_rejected` | Required |
| LLM-097 | Capacity exhaustion explicitly stops startup or new admissions. | `journal` | `journal_full_startup_and_runtime` | Required |
| LLM-098 | Deletion is whole-journal only; per-turn deletion is unsupported. | `journal` | `per_turn_delete_unsupported` | Required |
| LLM-099 | Each new encryption obtains an OS-random 192-bit nonce; retries reuse the existing frame. | `journal` | `nonce_frames_rng_failure_and_retry` | Required |
| LLM-100 | Native dependency trees are isolated and local IPC is bounded. | `IPC/dependency graph` | `ipc_limits_backpressure_and_child_crash` | Required |
| LLM-101 | Absent real TTS, audio qualification remains pending. | `qualification report` | `qualification_marks_audio_pending` | Required |
| LLM-102 | Fake audio cannot qualify TTS or audible latency. | `qualification report` | `qualification_requires_real_tts_evidence` | Required |
| LLM-103 | Unproven or failed cache reuse explicitly rebuilds. | `profile/text/metrics` | `cache_failure_rebuilds` | Required |
| LLM-104 | One executable versioned policy registry matches this document. | `protocol/types/policy` | `policy_and_traceability_complete` | Required |
<!-- INVARIANTS:END -->

## Verification

Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings -D clippy::too_many_lines`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, and `cargo test --workspace` with Rust 1.88. Real-model tests are ignored and require explicit assets. See `teto-llm/QUALIFICATION.md` for measured and pending evidence. No benchmark or qualification is inferred from a successful build.
