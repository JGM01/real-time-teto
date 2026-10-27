# English speech frontend and unit planning

P3 is implemented and qualified against the pinned local bank.

P3 converts normalized English text into checked lexical phones, syllables, and
selected CVVC units. The timed plan uses caller-supplied timing. Production
prosody, PCM rendering, playback, and playback receipts belong to later phases.

## Data flow and bounds

One frontend owns the immutable lexicon, override lexicon, and optional GRU.
`SpeechInput` retains one shared text allocation. Word records contain original
UTF-8 byte spans. Phones remain ARPAbet. Syllables partition each word into onset,
one vowel nucleus, and coda. Vowel-free pronunciations fail explicitly.

The maximum-onset table accepts common English two- and three-consonant onsets.
`NG` cannot begin a medial onset. A cluster split that cannot retain its complete
onset emits an uncertainty diagnostic. The syllabifier never changes phone stress.

Lexicon parsing preserves distinct alternatives and selects the lowest ordinal.
Identical pronunciations collapse. Conflicting equal ordinals fail. Explicit
whole-word overrides precede lookup. Unsupported text and unavailable OOV prediction
fail without number expansion, spelling substitution, or silent character removal.

The pinned checkpoint has 256 embedding and hidden values, 768 gate values, and
74 output tokens. Its tensor headers override the upstream comments that state 128.
The encoder retains one hidden state. Decoder output and gates use reusable storage.
The NumPy oracle preserves the upstream float32 equations and first-index argmax.
A word has at most 64 ASCII bytes. EOS must occur within 20 decoder steps.

The bank importer reads headers without decoding PCM. Sources retain their relative
locator, data range, format, and duration. Backing paths remain separate from the
structural manifest revision. ZIP Unicode path fields are validated by the ZIP
crate; remaining raw paths use strict UTF-8 or CP932 decoding. Symlinks, traversal,
path collisions, and unsupported ZIP64 containers fail. ZIP32 directory limits are
checked before the archive reader allocates its index.

OTO time uses integer microseconds. Negative cutoff gives length from offset;
positive cutoff trims from the WAV end. Consonant duration and signed overlap remain
separate fields. Consonant velocity remains absent for standard OTO rows. Every duplicate row remains present. Canonical OTO path and source
line define duplicate precedence. Source and alias IDs do not depend on archive
entry order. Manifest hashing excludes filesystem timestamps and absolute paths.
The inventory hash covers the sorted unique physical alias texts.

Each OTO directory retains its pitch groups and prefix-map metadata. Distinct
prefix/suffix pairs form separate recorded groups. Lookup applies each group’s
decoration and validates the undecorated bank symbols. Selection is
exact within a requested group. `PitchPolicy::Prefer` permits explicit fallback to
another group and records it. The supplied bank has one unpitched group. The import report counts absent `bV`
and velar-nasal reference forms. Contextual missing units belong to coverage diagnostics.

The planner retains ARPAbet-to-bank realization components. Missing combined
recordings can split a diphthong or affricate into two explicit parts. Selected
units can overlap in phone context. Validation proves coverage of every component,
checks each unit against the actual alias symbols, and verifies OTO provenance.
Unit intervals use the hull of the supplied target-phone timings. They do not
perform acoustic transition placement.

Limits live in `teto-speech/src/limits.rs`. The original clause caps remain intact.
Import caps include 512 MiB compressed, 256 MiB expanded, 8,192 entries, 16,384 OTO
rows, 4,096 sources, and 128 groups. Metadata lines have 4 KiB, aliases 256 bytes,
paths 1,024 bytes, and header scans 64 KiB. Planning admits at most 1,024 components,
2,048 units, and 4,096 lookup probes. Diagnostics retain at most 128 events;
counters continue to saturating integer limits when details fill.

## OpenUtau comparison

The development oracle compiles the pinned upstream projects. It constructs its own
syllable and word contexts from the same lexical phones, pitch, and fixture timing.
It uses an inventory-backed upstream singer interface. It loads no audio or neural
frontend. Each word receives its own note; the upstream code splits multisyllabic
words and constructs cross-word consonant context.

The fixed 50-clause corpus has 44 exact cases. Six cases require seven additional
recordings because the upstream cluster fallback omits lexical phones when an
intermediate CC alias is absent. These are explicit availability fallbacks:

| Zero-based case | Retained phone spans | Mechanism |
|---|---|---|
| 2 | `[12,13)`, `[13,14)` | Retain the first consonant cluster across repeated clusters. |
| 7 | `[17,18)` | Retain the affricate before the final consonant and next word. |
| 12 | `[21,22)` | Retain the stop before the next onset cluster. |
| 23 | `[5,6)` | Retain the fricative before a stop and affricate. |
| 35 | `[17,18)` | Retain the stop inside the cross-word consonant cluster. |
| 41 | `[12,13)` | Retain the stop before the next onset cluster. |

Every repair has `COVERAGE_REPAIR` provenance and a diagnostic. It must cover a
component that no other selected unit covers. The independent qualification check
recomputes the phone gaps after removing repairs, checks this exact allowance, and
compares every remaining syllable and ending alias group with OpenUtau. It rejects
other mismatches. The recordings themselves remain subject to the bank terms.

## Stable invariants

Checked constructors enforce span ordering and lexical stress representation.
These runtime rows name observable behavior and its regression test.

<!-- INVARIANTS:BEGIN -->
| ID | Contract | Enforcement | Test | Status |
|---|---|---|---|---|
| SPCH-001 | Speech input rejects empty text, controls, and oversize clauses. | `types` | `speech_input_rejects_empty_controls_and_oversized` | Required |
| SPCH-002 | Lexical and syllable spans form complete checked partitions. | `sequence` | `syllable_partition_preserves_every_phone` | Required |
| SPCH-003 | Text spans preserve UTF-8 and pronunciation ambiguity. | `frontend` | `frontend_preserves_utf8_word_spans_and_marks_ambiguity` | Required |
| SPCH-004 | Prosody contains one ordered valid timing for every phone. | `plan` | `prosody_rejects_missing_duplicate_and_reversed_timings` | Required |
| SPCH-005 | Import rejects escaping paths and invalid sample ranges. | `oto` | `oto_import_rejects_traversal_overflow_and_invalid_ranges` | Required |
| SPCH-006 | Duplicate OTO rows and signed timing retain their provenance. | `oto` | `oto_import_preserves_signed_timing_duplicates_and_sources` | Required |
| SPCH-007 | Every split phone retains both required components. | `plan` | `planner_preserves_both_affricate_and_diphthong_components` | Required |
| SPCH-008 | Exact aliases precede bank-specific substitutions. | `select` | `planner_prefers_exact_alias_before_teto_substitution` | Required |
| SPCH-009 | Missing recordings return a typed failure without a partial plan. | `select` | `planner_returns_missing_unit_without_partial_plan` | Required |
| SPCH-010 | Diagnostics retain bounded details without clause text. | `diagnostics` | `speech_debug_redacts_text_and_diagnostics` | Required |
| SPCH-011 | The decoder matches independent NumPy tokens and rejects missing EOS. | `gru` | `gru_matches_numpy_tokens` | Required |
| SPCH-012 | Import preserves separate pitch groups. | `manifest` | `oto_import_preserves_pitch_groups` | Required |
<!-- INVARIANTS:END -->

## Qualification

Use Rust 1.88. `make check` includes default and combined P3 feature tests, Clippy,
rustdoc, comment checks, and invariant traceability. It does not download assets.

Prepare optional development assets explicitly:

```sh
python3 tools/prepare_speech.py
python3 -m venv target/p3/oracle-venv
target/p3/oracle-venv/bin/pip install numpy==2.3.5
cargo install cargo-fuzz --root target/p3/tools --locked
```

Place the user-supplied archive at the path documented in `voicebanks/README.md`.
Then run:

```sh
make speech-qualify
make speech-fuzz
```

Qualification verifies source hashes and the official archive checksum. It requires
all 1,050 frozen clauses to produce valid plans, exact NumPy parity on 230 inputs,
and the fixed OpenUtau comparison. The coverage set contains 50 independent clauses
and 1,000 combinations of response templates and common, technical, and name terms.
The corpus hashes were frozen before planner tuning. No overrides mask its failures.
The GRU suite includes generated inputs that exercise the decoder limit.

The .NET 10 SDK builds the reference. NumPy and .NET are development dependencies.
Reports and oracle inputs remain under ignored `target/p3/`. Fuzz targets admit
bounded raw OTO bytes and checked phone/alias inputs. Synthetic fixture WAVs contain
silence and remain in temporary test directories. No Teto audio enters tests or git.

P3 evidence establishes lexical and unit-plan correctness. It establishes no speech
naturalness, audio timing, rendering speed, or device-completion result.

The recorded local run passes 1,050 of 1,050 plans, 230 GRU cases including twelve
missing-EOS cases, and all 50 OpenUtau cases under the explicit repair allowance.
Both fuzz targets pass 10,000 bounded iterations. `make check` passes under Rust 1.88.
The default dependency tree contains `arrayvec`, `bitflags`, and `thiserror` only.
