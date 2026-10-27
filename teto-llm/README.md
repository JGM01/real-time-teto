# teto-llm

Portable local inference and owned English clauses. A dedicated thread owns one pinned model/context, exact context budgeting, conditional KV reuse, structural channel filtering, bounded normalization/segmentation, receipt accounting and encrypted finalization. The desktop binary exposes these through private framed pipes; applications depend on the native-free `teto-llm-protocol` crate.

`Llm::open(OpenConfig)` validates immutable assets and persistence resources. `start_turn(UserInput)` returns a blocking `Turn`; `TurnControl` supports cross-thread cancellation and complete-clause receipts. Receipt acceptance continues after generation end until sealing. Receiving text alone never commits it.

The orchestrator drains generation and output receipts through an output fence, then calls `finish_turn` with the independent playback outcome. Successful finalization is durable when journaling is enabled. `finish_playback` accepts the retained turn ID if the consumer handle was dropped; dropping a turn cancels generation but does not silently authorize history. `close(&mut self)` and reset return `PlaybackPending` for unsettled output, retaining the owner so the caller can finalize and retry. Replacement requests cancel old generation and return the same typed error.

Generation stop and playback stop remain independent. History contains the original bounded user text, acknowledged canonical clauses and partial/outcome metadata. Prepared prompts reserve the full output budget and evict whole finalized exchanges. Cache reuse never changes canonical history. Full rebuild remains the fallback on unverified platforms, eviction, reset or failed removal.

The journal uses a caller-supplied zeroizing key, random XChaCha20-Poly1305 nonces, authenticated metadata, one writer and bounded pending ciphertext. Explicit setup/deletion are separate from ordinary opening. No replay, retrieval, summaries, views, tree, compaction or per-turn deletion exist in V1.

See the [contract and invariant matrix](../llm-subsystem.md), [canonical policy](../teto-llm-protocol/src/policy.rs), [CLI instructions](../teto-cli/README.md), [physical phone harness](../phone-spike/README.md) and [qualification report](QUALIFICATION.md). Tests using scripted input remain private implementation tests, with no public backend abstraction. Audio qualification requires real TTS and remains pending.
