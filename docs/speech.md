# Speech frontend and unit planning

The speech subsystem converts normalized English text into a checked phone
sequence and a timed unit plan.

```text
SpeechInput
  -> PhonemeSequence
  -> ProsodyPlan
  -> UnitPlan
```

The planner selects CVVC recordings from a local voicebank. The plan stores
recording IDs, phone spans, timing and bank revision. Rendering is outside this
subsystem.

## Bounds

One frontend owns the immutable lexicon, override lexicon and optional GRU.
`SpeechInput` owns one shared text buffer. Word records use original UTF-8 byte
spans. Phones use ARPAbet. Syllables partition each word into onset, one vowel
nucleus and coda.

Limits live with the code that enforces them:

| Area | Limit |
|---|---|
| Input | 240 scalars, 64 words, 512 phones, 256 syllables |
| Lexicon | 8 MiB, 200,000 rows, 2,000,000 phones, 64 phones per word |
| OTO | 8 MiB, 16,384 rows, 4,096 sources, 128 pitch groups |
| Archive | 512 MiB compressed, 256 MiB expanded, 8,192 entries |
| Plan | 1,024 realizations, 2,048 units, 4,096 probes, 128 diagnostics |

Import reads metadata only. It verifies archive paths, WAV headers, encoding and
signed timing. It does not decode PCM.

## Selection

The planner reads the sorted alias index and the OTO rows. It applies the
OpenUtau English X-SAMPA candidate order. Exact aliases precede bank-specific
substitutions. Missing combined recordings split an affricate or diphthong into
its two explicit symbols. A selected unit records its OTO row, target phone
span, realization span and substitution flags.

`UnitPlan::validate` proves:

- the key and sequence revision match;
- the bank revision matches;
- every phone has one realization, or the expected split pair;
- every realization is covered by a unit;
- every unit starts and ends on its target phone timing;
- every unit resolves to an OTO row with the same alias and pitch;
- the alias symbol stream equals the realized symbol stream after boundary removal.

## Stable contracts

| ID | Contract | Enforcement | Test | Status |
|---|---|---|---|---|
| SPCH-001 | Speech input rejects empty text, controls and oversize clauses. | `input` | `speech_input_rejects_empty_controls_and_oversized` | Required |
| SPCH-002 | Lexical and syllable spans form complete checked partitions. | `sequence` | `syllable_partition_preserves_every_phone` | Required |
| SPCH-003 | Text spans preserve UTF-8 and pronunciation ambiguity. | `frontend` | `frontend_preserves_utf8_word_spans_and_marks_ambiguity` | Required |
| SPCH-004 | Prosody contains one ordered valid timing for every phone. | `plan` | `prosody_rejects_reversed_and_oversized_timings` | Required |
| SPCH-005 | Import rejects escaping paths and invalid sample ranges. | `import` | `oto_import_rejects_traversal_overflow_and_invalid_ranges` | Required |
| SPCH-006 | Duplicate OTO rows and signed timing retain their provenance. | `import` | `oto_import_preserves_signed_timing_duplicates_and_sources` | Required |
| SPCH-007 | Every split phone retains both required components. | `plan` | `planner_preserves_both_affricate_and_diphthong_components` | Required |
| SPCH-008 | Exact aliases precede bank-specific substitutions. | `cvvc` | `planner_prefers_exact_alias_before_teto_substitution` | Required |
| SPCH-009 | Missing recordings return a typed failure without a partial plan. | `cvvc` | `planner_returns_missing_unit_without_partial_plan` | Required |
| SPCH-010 | Diagnostics retain bounded details without clause text. | `diagnostics` | `speech_debug_redacts_text_and_diagnostics` | Required |
| SPCH-011 | The decoder matches independent NumPy tokens and rejects missing EOS. | `gru` | `gru_matches_numpy_tokens` | Qualification |
| SPCH-012 | Import preserves separate pitch groups. | `index` | `oto_import_preserves_pitch_groups` | Required |

## Qualification

Use Rust 1.88. `make check` runs formatting, Clippy, tests, rustdoc, comment
checks and a fuzz target compile.

Prepare development assets:

```sh
python3 tools/prepare_speech.py
python3 -m venv target/p3/oracle-venv
target/p3/oracle-venv/bin/pip install numpy==2.3.5
cargo install cargo-fuzz --root target/p3/tools --locked
```

Place the user-supplied bank at the path in `voicebanks/README.md`. Then run:

```sh
make speech-qualify
make speech-fuzz
```

Qualification checks source hashes, the archive checksum, 1,050 frozen clauses
and the fixed 50-clause OpenUtau comparison. The corpus hashes are frozen. The
repair allowance is explicit in `teto-speech/assets/p3-sources.json`.

The default dependency tree contains `arrayvec`, `bitflags` and `thiserror`.
```
