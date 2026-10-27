# Teto CLI

Local validation of Earshot, Whisper and the clause-streaming LLM subsystem. Build both desktop binaries from the workspace root with `make build-release` (Rust 1.88). Keep `teto-cli` and `teto-llm-worker` together. The worker alone links llama.cpp; the parent owns ASR and Apple Keychain access.

## Modes

`TETO_MODE=transcribe` is the default. Set `TETO_WHISPER_MODEL` to a local GGML Whisper model, run the CLI in an interactive terminal, press SPACE and speak. Silence admits no LLM turn. Microphone access begins after SPACE.

`TETO_MODE=text` reads bounded UTF-8 stdin lines and starts no microphone or Whisper model. Each line is a user turn. `TETO_MODE=voice` uses the same SPACE capture and sends each resulting transcript to the same LLM client and clause renderer.

```sh
# Explicit no-journal text demo; no Whisper configuration needed.
TETO_MODE=text TETO_JOURNAL=off TETO_LLM_ASSETS="$PWD/models" \
  ./target/release/teto-cli

# Voice demo with an explicitly initialized journal.
TETO_MODE=voice TETO_LLM_ASSETS="$PWD/models" \
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal \
TETO_WHISPER_MODEL="$PWD/models/ggml-small.en.bin" ./target/release/teto-cli
```

The built-in desktop profile requires `gemma-4-26B_q4_0-it.gguf` with the exact SHA-256 in [Policy V1](../teto-llm-protocol/src/policy.rs). Obtain the candidate from [Google's model repository](https://huggingface.co/google/gemma-4-26B-A4B-it-qat-q4_0-gguf). Startup validates its checksum and template/tokenizer identity before inference. It does not download assets.

## Journal setup and deletion

LLM modes enable journaling by default. Select an absolute `TETO_JOURNAL_DIR` whose parent already exists. Explicit setup creates a new journal and a new Keychain entry; startup never creates or replaces either.

```sh
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal ./target/release/teto-cli journal-init
TETO_JOURNAL_DIR=/absolute/path/to/chat-journal ./target/release/teto-cli journal-delete
```

Deletion destroys the selected journal's Keychain key and ciphertext files, and refuses an active writer. There is no per-turn deletion. Key rotation means selecting a new journal. Keys never travel through arguments, environment variables or logs.

Eight bounded segments preserve existing records. Capacity exhaustion stops admission and exits nonzero; explicitly initialize/select another journal or restart with `TETO_JOURNAL=off`. Persistence errors never disable journaling. A failed append preserves the current turn and bounded pending ciphertext. The library exposes explicit retry; the CLI surfaces the failure and exits. An unresolved `pending.cbor` makes subsequent startup fail closed; preserve it for diagnosis. V1 does not replay or automatically recover it.

## Output and shutdown

Transcription mode prints `transcript:` lines. LLM modes print `clause:` lines, flush stdout, then acknowledge display completion. Successful display proves the history commit path only; no TTS or audio playback is implemented. Stderr reports generation stop, playback stop, input truncation and partial status. Only acknowledged complete clauses enter history and the encrypted journal. A fresh process always starts a fresh conversation.

EOF and Ctrl-C finalize and close. Broken stdout marks playback failed and attempts finalization before returning an error. Exit code zero means normal shutdown; configuration, native worker, normalization, journal or output failures return one. A child crash stops the service without automatic restart.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `TETO_MODE` | `transcribe` | `text`, `voice`, `transcribe` |
| `TETO_LLM_ASSETS` | required in LLM modes | Directory containing the pinned GGUF |
| `TETO_LLM_PROFILE` | desktop Policy V1 profile | Built-in profile ID; immutable per instance |
| `TETO_LLM_SYSTEM_PROMPT` | empty | Additional bounded trusted instruction |
| `TETO_LLM_WORKER` | sibling binary | Explicit worker path for development |
| `TETO_JOURNAL` | `on` | `on` or explicitly `off` |
| `TETO_JOURNAL_DIR` | required when enabled | Selected absolute journal directory |
| `TETO_WHISPER_MODEL` | required for ASR | Local Whisper model file |
| `TETO_WHISPER_LANGUAGE` | `en` | Language code or `auto` |
| `TETO_WHISPER_GPU` | true on macOS | Boolean GPU selection |
| `TETO_LOG` | `warn,teto_core=info,teto_cli=info` | Tracing filter; `off` disables logs |
| `TETO_LOG_FORMAT` | text | `json` selects JSON lines |

Normal diagnostic formatting redacts LLM text and keys. Whisper trace logging can expose transcripts; reserve `TETO_LOG=trace` for explicit ASR debugging. Native LLM generation logging remains suppressed.

## Validation

`make check` runs formatting, Clippy, deterministic tests and rustdoc with warnings denied. Real weights and physical devices are separate gates. `qualify-cold` measures twenty independently loaded children with ASR resident. Its latency starts before worker/model loading; operating-system page and shader caches remain uncontrolled. `qualify-lifecycle` adds planned interruptions after display receipts, resets, eviction and clean process reopen while preserving the journal. `qualify` runs the real child for at least one hundred warm measured turns and thirty minutes with Whisper resident:

```sh
TETO_LLM_ASSETS="$PWD/models" TETO_JOURNAL=off \
TETO_WHISPER_MODEL="$PWD/models/ggml-small.en.bin" ./target/release/teto-cli qualify
```

Compile `tools/thermal.swift` to `/tmp/teto-thermal` with `xcrun swiftc -O tools/thermal.swift -o /tmp/teto-thermal`. Use `python3 tools/qualify_desktop.py cold`, `python3 tools/qualify_desktop.py warm`, and `python3 tools/qualify_desktop.py lifecycle` to additionally sample process RSS and system thermal indicators with an explicitly initialized encrypted test journal. It preserves that journal and records its location. See [qualification evidence](../teto-llm/QUALIFICATION.md) for measurements and pending gates.
