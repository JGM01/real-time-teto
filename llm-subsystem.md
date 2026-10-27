# LLM Subsystem Contract

Status: Policy 2. This document defines the current subsystem. It contains no change history.

Complete user text and its truncation flag enter the subsystem. Owned English clauses leave it. The parent owns Earshot, Whisper, the user interface and playback. A private child owns the model, context, cache, history and encrypted journal. TTS is absent.

## Policy 2

The constants in `teto-llm-protocol/src/policy.rs` are the executable source for this table.

<!-- POLICY:BEGIN -->
| Decision | Policy 2 |
|---|---|
| Architecture | ASR, UI and playback in the parent; model and journal in a private child; one portable engine crate |
| Profiles | immutable catalog; caller supplies assets; one model and one context per process |
| Language / mode | English; direct answer; thinking off; structural channel filtering |
| Context builder | exact template and tokenizer count plus the output reserve; system, current user and reserve are pinned |
| History | token budget with a 524288-byte memory guard; the token budget binds |
| Eviction | evict the oldest finalized exchanges to the low-water mark; render once; rebuild once |
| Receive | blocking call with cancellation; delivery window of one or two clauses; receipts stay contiguous |
| Delivery window | one or two delivered clauses; the two-clause window is an experiment |
| Counting | Unicode scalar values; whitespace-separated words |
| Receipts | contiguous delivered prefix; accepted during generation and after generation end until sealing |
| Finalization | parent receipt fence; independent generation and playback stops; derived partial flag |
| History content | user text and acknowledged assistant clauses; partial prefixes are retained |
| Persistence | journal only; fresh context and cache on restart; no replay, retrieval, tree, view or compaction |
| Journal default | enabled in LLM modes; explicit no-journal mode writes no files |
| Key | caller supplies a zeroizing 256-bit key; Apple Keychain; never arguments, environment or logs |
| Encryption | XChaCha20-Poly1305; independent OS-random 192-bit nonce per record |
| AAD | journal ID, key ID, schema, segment, record ID and lengths |
| Writer | one writer and one retained encrypted frame; durable append before successful finish |
| Initialization | explicit journal initialization; ordinary startup never creates or replaces a journal or key |
| Journal failure | fail startup; at runtime stop admission and keep the current turn and encrypted frame; no plaintext fallback |
| Journal full | reserve one maximum record before admission; stop after the reserved turn |
| Retry | explicit; reuse the retained frame and record ID; report pending durability |
| Deletion / rotation | whole-journal crypto-shredding; refuse an active writer; rotation creates a new journal |
| CLI | `TETO_MODE=text`, `voice` or `transcribe`; default `transcribe`; print, flush, then acknowledge |
| IPC | private length-prefixed versioned CBOR pipes; bounded queues; validate before allocation |
| Crash | stop admission; no automatic restart or replay; child failure is not cancellation |
| Normalization | markdown, whitespace and punctuation normalization; omit code, tables, URLs and emoji; number and abbreviation expansion; unsupported forms fail |
| Variants | 8K context and twelve-word first target are separate qualification variants |
| Qualification | 100 warm turns and a thirty-minute soak; one-second clause and twenty tokens-per-second targets; audio gates pending real TTS |
| Context | 4096 tokens; minimum of configured, profile and runtime limits |
| Output reserve | 512 generated tokens, including suppressed channels |
| Clause queue / control queue | 2 / 4; independent priority cancellation |
| First clause | target 8 words; cap 12 words and 96 Unicode scalar values |
| Minimum clause | prefer 3 words or 12 Unicode scalar values; short complete replies are exempt |
| Cache | verified prefix reuse for the desktop artifact on Mac15,7/macOS 27.0.1 and the phone artifact on iPhone16,2/iOS 27.0.1; full rebuild elsewhere and after eviction, reset or removal failure |
| Asset integrity | SHA-256 on first use; metadata-keyed identity cache after that |
| Later clause | cap 40 words and 240 Unicode scalar values |
| Input / system prompt | 16384 / 4096 bytes |
| Normalized turn / clauses | 65536 bytes / 512 clauses |
| History low water | 70 percent of the prompt budget |
| Frame / record | 131072 bytes |
| Journal capacity | 8 segments of 8388608 bytes; no automatic deletion |
| Prompt batch | 512 tokens per decode call |
| Child startup / shutdown | 120 / 5 seconds; explicit timeout failure |
| Desktop model | gemma4-26b-qat-q4_0-macos-v1; gemma-4-26B_q4_0-it.gguf; SHA-256 3eca3b8f6d7baf218a7dd6bba5fb59a56ee25fe2d567b6f5f589b4f697eca51d; text-to-clause qualified on the recorded host; audio pending |
| Runtime | llama-cpp-2=0.1.154;llama.cpp=5f55650a78f92aff4d48d671423e888fac0469ff |
| Phone device | iPhone 15 Pro Max (iPhone16,2), 8 GB; iOS 27.0.1 |
| Phone artifact | gemma4-e2b-qat-q4_0-ios-spike-v1; gemma-4-E2B_q4_0-it.gguf; SHA-256 fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634; six-case cache probe and text-to-clause run passed on the Pro Max; audio pending |
| Template identity | desktop SHA-256 ae53464bf3be25802b3a5b37def7fd89667067d7577049b3b2d74c4d8de4c6d4; phone SHA-256 0a2c8073c878ab1da004bee933a998606537bbb62016310352c7285c3f01c5b5; profile-owned generation prefixes |
| Sampling | temperature 1; top-k 64; top-p 0.95; per-turn seed derivation |
| Desktop measurement | 100 warm turns; 1800-second soak |
| Lifecycle qualification | interrupt after the first displayed clause every 10 turns; reset every 128; clean process reopen every 256; errors stop the run |
| Cold process measurement | 20 independently loaded children; OS page and shader caches uncontrolled; ASR remains resident |
| Phone feasibility | at least 10 turns and 300 seconds; the iPhone 15 Pro Max is the qualification device |
<!-- POLICY:END -->

## Ownership and public surface

`teto-llm` contains the portable engine and the private `teto-llm-worker` binary. `teto-llm-protocol` contains native-free types and framed CBOR messages. The parent does not link llama.cpp. The child does not link Whisper or Earshot. No backend trait, model mutex, audio type, phoneme or token ID crosses the boundary. No library installs a global tracing subscriber. No network inference exists.

Checked constructors protect `UserInput`, `TurnId`, `ClauseIndex` and `SpeakableText`. Clauses own immutable UTF-8. Errors expose a bounded `ErrorKind`. Text and keys have redacted `Debug` output. One inference thread owns the model and context. Every queue and text buffer has a fixed bound.

Generation and playback have independent outcomes. `GenerationStop` is `Eos`, `TokenLimit`, `Cancelled` or `Failed`. `PlaybackStop` is `Complete`, `Interrupted` or `Failed`. Empty output means zero clauses. A finalized record stores both outcomes and derives `partial` from them.

## Receipts and finalization

The orchestrator owns finalization. `Turn::acknowledge` accepts receipts during generation and after generation end, until sealing. Only the contiguous delivered prefix can be acknowledged. Duplicate receipts are idempotent. Skipped, future, undelivered, wrong-turn and post-seal receipts fail.

Normal completion drains generation, waits for output completion, drains receipts, and calls `finish_turn(Complete)`. Interruption cancels generation, stops new output submissions, establishes a receipt fence outside the audio callback, drains receipts, then calls `finish_turn(Interrupted or Failed)`. Admission resumes only after durable finalization. The CLI uses a successful print and flush as a display receipt and fence.

`start_turn` cancels an unsettled predecessor and returns `PlaybackPending`. It never seals silently. Reset and close require the same barrier. An observed terminal event is never replaced. Cancellation discards unread clauses and wakes blocked producer and consumer. Terminal state does not consume a clause slot.

`finish_playback` accepts the retained turn ID so the orchestrator can finalize after its consumer handle is dropped. `close` takes a mutable borrow and preserves the owner on `PlaybackPending`.

## Context and normalization

The context builder alone renders and tokenizes prompts. It enforces `prompt_tokens + output_reserve <= min(configured, profile, runtime context)`. The system instruction, current user turn and reserve are pinned.

The history has one token budget and one byte guard. When the prompt does not fit, the builder evicts the oldest whole finalized exchanges until the prompt is at or below 70 percent of the prompt budget. It renders once, then rebuilds the cache once. It never truncates the current input. Oversized pinned input fails with `ContextBudget`.

The builder keeps user text and acknowledged complete assistant clauses, including partial cancelled and token-limited turns. It keeps both outcome axes. Unplayed text stays in a bounded transient buffer and never enters history or disk. It does not retain reasoning or discarded tails.

Normalization strips markdown wrappers, code, tables, URLs and emoji. It normalizes whitespace and typographic punctuation. It expands zero through twenty and a small abbreviation table. It spells larger unsigned integers as digits. Currency, ordinal, decimal, date and unit syntax fails closed. Segmentation protects lexical units, preserves punctuation, and uses both word and character caps. Token-limit tails are dropped.

## Native and cache gates

Shared GGML is not a delivery prerequisite. `whisper-rs-sys` bundles GGML 0.9.5. `llama-cpp-2` pins GGML 0.18.0. Compatibility is unproven. Any shared-GGML change has a four-engineer-hour limit and must prove API compatibility, one linked implementation, and inference in both load orders.

The pinned Rust API exposes `clear_kv_cache_seq`, `clear_kv_cache` and `decode`. Partial removal can fail. Prefix reuse is enabled only for a checksummed artifact on its recorded host triple. Full rebuild is the fallback on every other host and after eviction, reset or removal failure. History correctness does not depend on cache capability. No cache survives a restart.

The asset identity cache trusts the model path metadata. An actor with write access to the model file and the cache can replace both. The journal key is independent.

## Journal lifecycle

The library receives a caller-supplied zeroizing 256-bit key. Apple binaries fetch it from Keychain and send it over a private pipe. One key belongs to one journal. A frame uses a fresh OS-random 192-bit nonce and per-record XChaCha20-Poly1305. AAD covers journal ID, key ID, schema, segment, record ID and lengths. Retry reuses the retained ciphertext unchanged.

Explicit setup creates the journal identity, authenticated header and Keychain entry. Ordinary startup never creates or replaces either. Missing, corrupt, mismatched or inaccessible resources fail startup. Initialization rollback removes only resources created by that attempt.

The writer reserves one maximum record before admission. A full journal fails startup or stops admission after the reserved turn. The CLI reports `JournalFull`, closes and exits nonzero. Continue only by selecting another journal or restarting without journaling. No automatic deletion occurs.

A write failure keeps the current turn and one encrypted frame in memory. If the segment write fails, the writer stores the frame in `pending.cbor` and reports pending durability. Admission stops. Only explicit retry is allowed. Successful finish requires a durable segment append. An unresolved pending artifact makes the next startup fail closed.

Deletion is whole-journal only. It refuses an active writer, deletes the Keychain key and removes the ciphertext files. Per-turn deletion and tombstones do not exist. Key rotation creates a new journal.

## CLI and qualification

The modes are `text`, `voice` and `transcribe`. `transcribe` is the default. Text mode needs no microphone and no Whisper configuration. Voice mode uses SPACE capture. Both LLM modes use one renderer and one child client. The renderer prints each clause, flushes, then acknowledges display.

The measurement harness records integer microseconds. It reports cache mode, cache reason, prompt tokens, cached tokens, evaluated tokens, generated tokens, evictions, rebuilds and phase durations. A desktop run uses 100 warm turns and a 1800-second soak. A lifecycle run interrupts every 10 turns, resets every 128 turns and reopens the process every 256 turns.

Audio qualification requires real TTS. First audible sample, starvation and complete audio latency remain pending. Synthetic audio cannot qualify these gates.

## Stable invariants and traceability

Each row names one observable behavior and the test that fails when the behavior changes. Type-enforced rules are not listed.

<!-- INVARIANTS:BEGIN -->
| ID | Contract | Enforcement | Test | Status |
|---|---|---|---|---|
| LLM-001 | One turn is active per `Llm`. | `api` | `replacement_cancels_before_admission` | Required |
| LLM-002 | One model context serves a process. | `worker` | `one_context_lifetime` | Required |
| LLM-003 | One clause producer serves a turn. | `mailbox` | `ordered_clauses_carry_turn` | Required |
| LLM-004 | A replacement cancels the old turn before admission. | `api` | `replacement_cancels_before_admission` | Required |
| LLM-005 | An unobserved old stream ends `Cancelled` once. | `mailbox` | `cancel_is_idempotent_and_not_failure` | Required |
| LLM-006 | Reset and close cancel the active turn. | `api` | `reset_active_turn` | Required |
| LLM-007 | Dropping a turn cancels generation. | `api` | `drop_active_turn` | Required |
| LLM-008 | Clauses arrive in order with monotonic indices. | `mailbox` | `ordered_clauses_carry_turn` | Required |
| LLM-009 | No clause arrives after a terminal event. | `mailbox` | `terminal_is_single_use` | Required |
| LLM-010 | Receipts accept only the contiguous delivered prefix. | `mailbox` | `receipts_are_contiguous_and_idempotent` | Required |
| LLM-011 | Complete sealing requires the receipt fence. | `mailbox` | `complete_seal_requires_receipt_fence` | Required |
| LLM-012 | Sealed turns reject receipts. | `mailbox` | `sealed_turn_rejects_receipts` | Required |
| LLM-013 | A cancelled turn keeps its acknowledged prefix. | `mailbox` | `cancel_keeps_played_prefix` | Required |
| LLM-014 | Worker failure wakes the consumer with `Worker`. | `mailbox` | `worker_failure_wakes_consumer` | Required |
| LLM-015 | A full clause queue blocks the producer. | `mailbox` | `slow_consumer_blocks_producer` | Required |
| LLM-016 | Cancellation wakes a blocked consumer. | `mailbox` | `cross_thread_cancel_wakes_consumer` | Required |
| LLM-017 | Receipts are accepted during and after generation. | `mailbox` | `receipt_during_and_after_generation` | Required |
| LLM-018 | Receipt and seal have one linearized result. | `mailbox` | `receipt_racing_seal_has_one_linearized_result` | Required |
| LLM-019 | Prompt plus output reserve fits the context limit. | `context` | `exact_fit_and_one_token_over` | Required |
| LLM-020 | The context limit is the minimum of profile and runtime limits. | `context` | `effective_context_limit` | Required |
| LLM-021 | Oversized pinned content fails without truncation. | `context` | `oversized_pinned_prompt_error` | Required |
| LLM-022 | Eviction removes the oldest exchange first. | `context` | `eviction_removes_the_oldest_exchange` | Required |
| LLM-023 | Budget arithmetic preserves the headroom identity. | `context` | `budget_arithmetic_matches_widened_oracle` | Required |
| LLM-024 | Clause length is bounded by position. | `protocol` | `clause_size_bounds` | Required |
| LLM-025 | Clause text is owned UTF-8 and immutable. | `protocol` | `clause_ownership_utf8` | Required |
| LLM-026 | Clause text is speakable text. | `text` | `speakable_text_rejects_markup_and_controls` | Required |
| LLM-027 | Turn IDs are nonzero and monotonic. | `protocol` | `turn_id_reset_and_overflow` | Required |
| LLM-028 | Token IDs, phonemes and audio never cross the boundary. | `protocol` | `protocol_surface` | Required |
| LLM-029 | Inputs are bounded and never silently truncated. | `protocol` | `oversized_input_not_truncated` | Required |
| LLM-030 | Diagnostic output contains no response text or key. | `protocol` | `response_sentinel_not_logged` | Required |
| LLM-031 | Channel filtering removes reasoning and tool text. | `text` | `reasoning_and_control_suppressed` | Required |
| LLM-032 | Code, tables, URLs and emoji are omitted. | `text` | `normalization_omits_code_tables_urls_emoji` | Required |
| LLM-033 | Clause output does not depend on input chunk boundaries. | `text` | `chunking_is_invariant` | Required |
| LLM-034 | Numeric and abbreviation normalization is deterministic. | `text` | `normalization_v1` | Required |
| LLM-035 | Journal records use authenticated encryption. | `journal` | `journal_aead_roundtrip_and_tamper` | Required |
| LLM-036 | The journal stores user text and the played prefix. | `journal` | `journal_contains_user_and_played_prefix` | Required |
| LLM-037 | A failed write keeps one durable encrypted frame. | `journal` | `write_failure_stops_turns` | Required |
| LLM-038 | One writer owns a journal. | `journal` | `second_writer_rejected` | Required |
| LLM-039 | Initialization is explicit and never replaces a journal. | `journal` | `explicit_initialization_only` | Required |
| LLM-040 | A missing key or journal fails closed. | `journal` | `journal_open_failure` | Required |
| LLM-041 | Deletion removes the whole journal. | `journal` | `journal_delete_and_key_loss` | Required |
| LLM-042 | Capacity exhaustion stops admission and preserves records. | `journal` | `journal_full_startup_and_runtime` | Required |
| LLM-043 | A truncated frame fails closed. | `journal` | `truncated_frame_fails_closed` | Required |
| LLM-044 | The worker rejects oversized frames. | `worker` | `oversized_worker_frame_rejected` | Required |
| LLM-045 | Parent and child link disjoint native trees. | `worker` | `native_and_audio_isolation` | Required |
| LLM-046 | An invalid profile fails before model load. | `profile` | `invalid_profile_prevents_load` | Required |
| LLM-047 | Journal operations run in the private child. | `worker` | `worker_journal_init_inspect_delete` | Required |
| LLM-048 | A journal operation requires the caller key. | `worker` | `key_required_by_library` | Required |
| LLM-049 | Explicit no-journal mode writes no files. | `worker` | `explicit_no_journal_writes_nothing` | Required |
| LLM-050 | Policy constants and this table agree. | `protocol` | `policy_numbers_match_document` | Required |
| LLM-051 | Prefix reuse is enabled only for a recorded artifact and host triple. | `profile` | `phone_profile_reuses_prefix_on_recorded_host` | Required |
| LLM-052 | The delivery window is one or two and receipts remain contiguous. | `mailbox` | `two_clause_window_keeps_receipts_contiguous` | Required |
<!-- INVARIANTS:END -->

## Verification

Run `make check` with Rust 1.88. It runs formatting, Clippy with warnings denied, rustdoc with warnings denied, the fast tests and the invariant check. Real-model tests need explicit assets and run with `make qualify`. Performance gates need the measurement runs in `teto-llm/QUALIFICATION.md`. A build is not a qualification.
