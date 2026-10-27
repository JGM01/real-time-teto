# Code Standard

## Priorities

1. Safety.
2. Performance.
3. Developer experience.

## Bounds and units

Every queue, buffer, loop and timeout has a policy constant. Every arithmetic
operation that can overflow uses a checked or saturating form.

Data types use explicit widths. Time values use `u64` microseconds. A field
name states its unit. Milliseconds and floating point values appear only in
reports.

## Control flow

Each state machine is one `enum`. Correlated booleans are not state. A function
returns early on error. A function is at most 70 lines.

## Allocation

The per-token path does not allocate in this crate. A clause allocates at most
one text buffer. A turn uses bounded buffers. Allocations inside dependencies
are documented exceptions.

## Errors and assertions

Public functions validate input and return a typed error. `assert!` marks an
internal invariant. Every `assert!` in a worker has a containment path that
fails the active turn.

`unwrap` and `expect` are forbidden outside tests. A local invariant that
cannot fail uses `expect` only with a short message that states the invariant.

## Concurrency

One model and one context belong to one thread. Locks recover from poison.
A condition wait has a named predicate and a wake path.

## Naming

Names describe the mechanism. Avoid abbreviations that are not domain terms.
Use `context`, `mailbox`, `journal`, `prefix_cache`, `clause`, `turn`, `token`,
`frame` and `segment`. Domain terms: `ASR`, `IPC`, `CBOR`, `AEAD`, `GGUF`,
`KV`, `LLM`, `TTS`.

## Comments

Comments use simplified technical English:

* Present tense and active voice.
* One idea per sentence. At most 25 words per sentence.
* Describe what the code does: input, output, bound, unit, owner, invariant.
* No history, rationale, first person or metaphor.
* Use `must`, `can` and `cannot`. Do not use `should`.
* Rationale and history belong in `docs/`.

Examples:

```rust
// Move the clause text into the played buffer. The queue keeps no copy.
// Clear the token cache when an exchange leaves the prompt. Positions change.
```

```rust
/// Return the next clause event.
///
/// # Errors
/// Returns `Exhausted` after the terminal event.
/// Returns `Worker` when the inference thread fails.
```

Banned phrases: `previously`, `used to`, `no longer`, `legacy`, `old behavior`,
`old implementation`, `changed from`, `we `, `our `, `TODO`, `FIXME`, `XXX`,
`HACK`, `for now`, `temporarily`, `will be`.

## Tests

A test names one observable behavior. Tests do not parse source text. Tests do
not mock the standard library. A property test covers arithmetic and chunk
boundaries. A replay test covers text assembly. A failure model covers the
journal write path.

Unit tests live in the module file under `#[cfg(test)] mod tests`. A unit test
that needs private items stays in the module file. End-to-end tests live in the
crate `tests/` directory. A `src` module does not contain a `tests.rs` file.
