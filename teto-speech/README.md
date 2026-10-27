# teto-speech

A bounded English frontend and CVVC unit planner. The pipeline produces a checked
`PhonemeSequence` and a timed `UnitPlan` with source, OTO, alias, and pitch provenance.
Prosody prediction, PSOLA, and device output remain unimplemented seams. P3 produces
no audio and makes no audio latency or quality claim.

Default features contain checked contracts, mapping, and planning logic. They load
no dictionary, checkpoint, voicebank, or native backend.

| Feature | Capability |
|---|---|
| `cmudict` | Embedded pinned CMU dictionary and deterministic English syllables |
| `g2p-gru` | Direct g2p_en GRU from a checksummed caller-supplied NPZ checkpoint |
| `voicebank-wav` | Bounded local OTO, WAV header, and ZIP/directory import |
| `llm-adapter` | Clause identity and text adaptation without inference dependencies |
| `testing` | Synthetic metadata construction for property tests and fuzz targets |

Call `CmudictPhonemizer::embedded(OovBackend)` with caller-owned
`SpeechDiagnostics`. Import with `TetoOtoImport::load`. Pass a checked
`ProsodyPlan` by value to `TetoCvvcPlanner::plan`. Plans borrow no model or bank
storage; their IDs resolve against the recorded manifest revision. `ImportedBank`
retains a separate backing location for the source locators.

One owner holds the frontend and GRU. Asset tables and lexical arrays use contiguous
storage. Word lookup borrows source spans. The planner uses fixed-capacity alias
candidates. Diagnostics contain spans, IDs, and reason enums, with bounded details
and saturating counters. No runtime path downloads assets or logs clause text.

See `../docs/speech.md` for invariants, qualification commands, and the explicit
OpenUtau coverage-repair differences. See `../THIRD_PARTY.md` for attribution.
