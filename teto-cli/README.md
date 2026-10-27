# Teto CLI

Local validation of Earshot, Whisper and the clause-streaming LLM. Build both desktop binaries from the workspace root with `make build-release` (Rust 1.88). Keep `teto-cli` and `teto-llm-worker` together. The worker alone links llama.cpp. The parent owns ASR and Apple Keychain access.

## Modes

`TETO_MODE=transcribe` is the default. Set `TETO_WHISPER_MODEL` to a local GGML Whisper model, run the CLI in an interactive terminal, press SPACE and speak. Silence admits no LLM turn. Microphone access begins after SPACE.

`TETO_MODE=text` reads bounded UTF-8 stdin lines and starts no microphone or Whisper model. Each line is a user turn. `TETO_MODE=voice` uses the same SPACE capture and sends each transcript to the same client and renderer. An unknown mode is a configuration error.

```sh
# Explicit no-journal text demo.
TETO_MODE=text TETO_JOURNAL=off TETO_LLM_ASSETS="$PWD/models" \
  ./target/release/teto-cli

# Voice demo with an explicitly initialized journal.
TETO_MODE=voice TETO_LLM_ASSETS="$PWD/models" \
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal \
TETO_WHISPER_MODEL="$PWD/models/ggml-small.en.bin" ./target/release/teto-cli
```

The desktop profile requires `gemma-4-26B_q4_0-it.gguf` with the exact SHA-256 in [the policy registry](../teto-llm-protocol/src/policy.rs). Startup validates the checksum, template and tokenizer identity before inference. It does not download assets.

The worker hashes the model once. A metadata-keyed identity file avoids a full hash when the file does not change. Set `TETO_ASSET_CACHE` to select the cache directory. The default is `teto-asset-cache` under the system temporary directory.

## Journal

LLM modes enable journaling by default. Select an absolute `TETO_JOURNAL_DIR` whose parent exists. Explicit setup creates a new journal and a new Keychain entry. Startup never creates or replaces either.

```sh
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal ./target/release/teto-cli journal-init
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal ./target/release/teto-cli journal-delete
```

Deletion destroys the selected Keychain key and the ciphertext files. It refuses an active writer. There is no per-turn deletion. Key rotation selects a new journal. Keys never travel through arguments, environment variables or logs.

Eight bounded segments preserve existing records. Capacity exhaustion stops admission and exits nonzero. Initialize or select another journal, or restart with `TETO_JOURNAL=off`. Persistence errors never disable journaling. A failed append retains one encrypted frame. An unresolved `pending.cbor` makes the next startup fail closed.

## Output and shutdown

Transcription mode prints `transcript:` lines. LLM modes print `clause:` lines, flush stdout, then acknowledge display completion. The CLI does not synthesize or play audio. Stderr reports generation stop, playback stop, input truncation and partial status. Only acknowledged complete clauses enter history and the encrypted journal. A fresh process starts a fresh conversation.

EOF and Ctrl-C finalize and close. Broken stdout marks playback failed and attempts finalization before returning an error. Exit code zero means normal shutdown. Configuration, native worker, normalization, journal and output failures return one. A child crash stops the service without automatic restart.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `TETO_MODE` | `transcribe` | `text`, `voice` or `transcribe` |
| `TETO_LLM_ASSETS` | required in LLM modes | Directory containing the pinned GGUF |
| `TETO_LLM_PROFILE` | desktop profile | Built-in profile ID |
| `TETO_LLM_SYSTEM_PROMPT` | empty | Additional bounded instruction |
| `TETO_LLM_WORKER` | sibling binary | Explicit worker path |
| `TETO_ASSET_CACHE` | temporary directory | Model identity cache directory |
| `TETO_JOURNAL` | `on` | `on` or explicitly `off` |
| `TETO_JOURNAL_DIR` | required when enabled | Selected absolute journal directory |
| `TETO_WHISPER_MODEL` | required for ASR | Local Whisper model |
| `TETO_WHISPER_LANGUAGE` | `en` | Language code or `auto` |
| `TETO_WHISPER_GPU` | true on macOS | Boolean GPU selection |
| `TETO_LOG` | `warn,teto_core=info,teto_cli=info` | Tracing filter |
| `TETO_LOG_FORMAT` | text | `json` selects JSON lines |

Normal diagnostics redact LLM text and keys. Whisper trace logging can expose transcripts. Use `TETO_LOG=trace` only for ASR debugging.

## Measurement

`make check` runs formatting, Clippy, rustdoc, the fast tests, the comment check and the invariant check with warnings denied.

`qualify` measures 100 warm turns and a 1800-second soak with Whisper resident. `qualify-cold` measures 20 independently loaded children. `qualify-lifecycle` adds planned interruptions, reset and a clean process reopen. All reports use integer microseconds and include cache mode, cache reason, rebuilds and evictions.

```sh
TETO_LLM_ASSETS="$PWD/models" TETO_JOURNAL=off \
TETO_WHISPER_MODEL="$PWD/models/ggml-small.en.bin" ./target/release/teto-cli qualify
```

Compile the thermal sampler with `xcrun swiftc -O tools/thermal.swift -o /tmp/teto-thermal`. Use `python3 tools/qualify_desktop.py cold`, `python3 tools/qualify_desktop.py warm` and `python3 tools/qualify_desktop.py lifecycle` to sample process RSS and thermal state with an encrypted test journal. See [qualification evidence](../teto-llm/QUALIFICATION.md).
