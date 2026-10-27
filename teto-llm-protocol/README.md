# teto-llm-protocol

Native-free checked boundary types, Policy 2 and versioned length-prefixed CBOR. The decoder validates the frame length before allocation and rejects unknown versions, trailing data and invalid checked fields. Secret-bearing frames use a zeroizing buffer. Messages carry owned user and clause text, receipts, outcome metadata and lifecycle requests. Native handles, token IDs, phonemes and audio are absent.

The private desktop protocol is lockstep request/response. Each frame carries a sequence number. A response with the wrong sequence number is discarded. A Pull spends a clause credit. The delivery window is one or two clauses. Receipts stay contiguous. A second delivery is possible only within the configured window. Cancellation uses the reader path and discards unread clauses. Terminal state is outside the two clause slots. Control requests have four slots. Out-of-state and stale receipts fail. A child failure is a service failure.

`policy.rs` is the executable policy registry. `llm-subsystem.md` restates it. The contract test checks the numeric rows. The invariant check confirms that every matrix row names a test that exists.
