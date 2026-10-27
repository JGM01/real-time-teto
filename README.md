# real-time-teto

A local speech-to-clause pipeline. Native ASR and the LLM run in separate processes.

- `earshot`: capture, speech gating and endpointing. A workspace member.
- `teto-core`: Earshot and Whisper capture orchestration.
- `teto-llm-protocol`: native-free checked messages and the Policy 2 registry.
- `teto-llm`: model inference, clause assembly, receipt-authorized history and an encrypted journal. It also builds the private worker binary.
- `teto-llm-client`: typed parent-side client with no llama.cpp dependency.
- `teto-speech`: English speech contracts; model and audio backends are scaffolds.
- `teto-cli`: transcription, text and voice modes, plus real-model measurement commands.
- `phone-spike`: an iPhone feasibility harness.

Use Rust 1.88. Run `make check` and `make build-release`. The Makefile selects the pinned compiler, Clippy and rustdoc.

Model files are caller-provided and checksum-validated. They are excluded from version control. The desktop binaries must be installed together.

See [CLI setup](teto-cli/README.md), [LLM ownership](teto-llm/README.md), [the contract](llm-subsystem.md), [qualification](teto-llm/QUALIFICATION.md) and [the phone harness](phone-spike/README.md).

The LLM boundary produces text clauses. The speech crate is scaffolded with English contracts; its phonemizer, CVVC planner, prosody, PSOLA and device backends are not implemented yet. No TTS or audio qualification exists. No synthetic audio qualifies anything.

## License

The code is licensed under MIT OR Apache-2.0. Model files and voicebanks are not covered. The Teto voicebank is user-supplied and subject to its own terms.
