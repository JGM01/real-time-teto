# real-time-teto

Local speech-to-clause pipeline with separate native ASR and LLM processes on desktop.

- `earshot`: capture, speech gating and endpointing; independent subsystem.
- `teto-core`: Earshot/Whisper capture orchestration and reliable transcripts.
- `teto-llm-protocol`: native-free checked messages and canonical Policy V1.
- `teto-llm`: local inference, normalized clause streams, receipt-authorized history and encrypted journal; private worker binary.
- `teto-cli`: explicit transcription, text and voice demos, plus real-model measurement commands.
- `phone-spike`: direct use of the same engine crate in a physical iPhone feasibility app.

Use Rust 1.88 and run `make check` and `make build-release`. The Makefile selects the actual pinned compiler, Clippy and rustdoc. Model files are caller-provided and checksum-validated; they are excluded from version control. Desktop binaries must be installed together.

See [CLI setup and modes](teto-cli/README.md), [LLM API and ownership](teto-llm/README.md), [the contract and invariant matrix](llm-subsystem.md), [qualification evidence](teto-llm/QUALIFICATION.md), and [the physical phone harness](phone-spike/README.md).

The current boundary produces text clauses. Phonemization, TTS, actual playback receipts and audio qualification remain future integration work. No synthetic audio is used to qualify them.
