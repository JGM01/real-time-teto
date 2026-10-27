# teto-llm-protocol

Native-free checked boundary types, Policy V1 and versioned length-prefixed CBOR. The decoder validates the frame length before allocation and rejects unknown versions, trailing data and invalid checked fields. Secret-bearing serialization buffers zeroize on drop. Messages carry owned user/clause text, receipts, outcome metadata and explicit lifecycle requests; native runtime handles, token IDs, phonemes and audio are absent.

The private desktop protocol is lockstep request/response with bounded independent reader/writer paths. A Pull spends a clause credit; a delivered clause must be acknowledged before another delivery. Cancellation uses the priority reader path and discards unread clauses. Terminal state is outside the two clause slots. Control requests have four slots. Out-of-state and stale receipts fail; a child failure is a service failure.

`policy.rs` is the executable policy registry. Regenerate its documentation with `make --silent policy | python3 tools/update_policy.py`. The contract check rejects missing/duplicate invariant identities, missing enforcement/test references, unmarked deferred gates and policy drift.
