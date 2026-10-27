# teto-llm

Local inference and owned English clauses. One inference thread owns one model and one context. The crate provides exact context budgeting, conditional KV reuse, structural channel filtering, bounded normalization, receipt accounting and encrypted finalization. The desktop binary exposes this engine through private framed pipes. Applications use the native-free `teto-llm-protocol` crate.

`Llm::open(OpenConfig)` validates the immutable assets and the persistence resources. `start_turn(UserInput)` returns a blocking `Turn`. `TurnControl` supports cross-thread cancellation and complete-clause receipts. Receipts are accepted after generation end until sealing. Receiving text never commits it.

The orchestrator drains generation and output receipts through a fence, then calls `finish_turn` with the independent playback outcome. Successful finalization is durable when the journal is enabled. `finish_playback` accepts the retained turn ID when the consumer handle is dropped. Dropping a turn cancels generation but does not authorize history. `close` and reset return `PlaybackPending` for unsettled output and keep the owner. A replacement request cancels old generation and returns the same typed error.

Generation stop and playback stop stay independent. History contains bounded user text, acknowledged clauses and partial outcome metadata. A prompt reserves the full output budget. When the prompt does not fit, the builder evicts the oldest exchanges to 70 percent of the prompt budget and rebuilds once. Cache reuse never changes canonical history. Full rebuild is the fallback on unverified hosts and after eviction, reset or removal failure.

The journal uses a caller-supplied zeroizing key, random XChaCha20-Poly1305 nonces, authenticated metadata, one writer and one retained encrypted frame. Setup and deletion are explicit. The journal does not replay into context. There is no retrieval, summary, view, tree, compaction or per-turn deletion.

Measurements use integer microseconds. The optional `measurement` feature adds the real-model harness. Enable it for the phone spike and for measurement runs.

See [the contract](../llm-subsystem.md), [the policy registry](../teto-llm-protocol/src/policy.rs), [CLI instructions](../teto-cli/README.md), [the phone harness](../phone-spike/README.md) and [qualification](QUALIFICATION.md).
