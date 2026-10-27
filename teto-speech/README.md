# teto-speech

A bounded English frontend and CVVC unit planner. The crate produces a checked
`PhonemeSequence` and a timed `UnitPlan` with OTO, alias and pitch provenance.
Audio rendering is outside this crate.

Default features contain checked contracts, mapping and planning logic. They load
no dictionary, checkpoint, voicebank or native backend.

| Feature | Capability |
|---|---|
| `cmudict` | Embedded pinned CMU dictionary and deterministic English syllables |
| `g2p-gru` | Direct g2p_en GRU from a checksummed caller-supplied NPZ checkpoint |
| `voicebank-wav` | Bounded local OTO, WAV header and ZIP/directory import |
| `llm-adapter` | Clause identity and text adaptation without inference dependencies |
| `qualification` | Report binary for local corpus and bank qualification |
| `testing` | Synthetic metadata construction for property tests and fuzz targets |

Call `CmudictPhonemizer::embedded(OovBackend)` with caller-owned
`SpeechDiagnostics`. Import with `import_voicebank`. Pass a checked
`ProsodyPlan` by value to `TetoCvvcPlanner::plan`. Plans borrow no model or bank
storage. Their IDs resolve against the recorded bank revision. `ImportedBank`
retains a separate backing location for the source locators.

One owner holds the frontend and GRU. Asset tables and lexical arrays use
contiguous storage. Word lookup borrows source spans. The planner uses
fixed-capacity alias candidates. Diagnostics contain spans, IDs and reason enums,
with bounded details and saturating counters. No runtime path downloads assets or
logs clause text.

See `../docs/speech.md` for contracts and qualification commands. See
`../THIRD_PARTY.md` for attribution.
