# Earshot

Earshot is a Rust library for capturing one spoken statement and deciding when that statement ends. It opens the microphone only after an activation is accepted, keeps two inference engines warm, and returns owned 16 kHz mono audio to the caller. At runtime, it writes no files, uses no network, and emits no telemetry.

## Platform support

macOS on Apple silicon is the only tested platform. Other platforms may compile, but they are not qualified. Support for them is best effort. Documentation is built locally on macOS with warnings denied for release.

## Quick start

Earshot is not published yet, so use a path dependency. The example below opens the microphone, accepts one activation, and handles a completed statement.

```rust
use earshot::{Earshot, Event, Outcome};

fn main() -> earshot::Result<()> {
    let mut earshot = Earshot::open()?;
    let control = earshot.control();
    let id = control.activate()?;

    loop {
        match earshot.recv()? {
            Event::Listening(listening) => {
                assert_eq!(listening, id);
            }
            Event::Finished {
                id: finished,
                outcome,
            } => {
                assert_eq!(finished, id);
                if let Outcome::Complete(statement) = outcome {
                    let _audio = statement.into_audio();
                    // Process audio.samples() as 16 kHz mono f32.
                }
                break;
            }
            _ => break,
        }
    }

    earshot.close()
}
```

The example handles `Complete` only. A real application should match the other outcomes too.

`Earshot::open` loads and warms both engines. The microphone stays closed until an activation is accepted. `Control::activate` accepts one attempt. The owner then receives `Event::Listening` when capture is ready, followed by exactly one `Event::Finished`.

The next activation is released when the owner receives `Finished`. There is no separate acknowledgment, and an unread `Finished` keeps the service busy.

## Outcomes and lifecycle

An accepted attempt produces at most one `Listening` event and exactly one `Finished` event, unless the owner closes. A startup failure can produce `Finished` before `Listening`.
- `Outcome::Complete(statement)` means a current positive semantic decision created a statement. It carries owned audio.
- `Outcome::Incomplete` with `IncompleteReason::SilenceLimit` returns speech audio after five seconds without speech. Completion is unconfirmed.
- `Outcome::Incomplete` with `IncompleteReason::DurationLimit` returns the retained audio at the capture bound. The returned audio may stop mid-speech.
- `Outcome::NoSpeech` means no speech started before the limit. It carries no audio.
- `Outcome::Cancelled` means the caller cancelled the activation. It carries no audio.
- `Outcome::Failed(error)` means capture or inference failed. It carries no audio.

`Complete` requires a current positive semantic decision. A timer fallback never creates a statement.

`Earshot::close` stops capture and joins all workers. It can wait for an in-flight model call, but it never waits for an event consumer. Dropping the owner also stops the service.

## Threading and cancellation

`Earshot` is `Send` but not `Sync`, so keep the event loop on one thread. `Control` is `Send + Sync + Clone`; other threads can hold clones. A clone can activate or cancel, so give clones only to trusted threads.

`Control::activate` blocks until the coordinator replies. It returns `Busy` while capture is active or an outcome is unread, and `Closed` after the owner closes.

`Control::cancel` does not block and can run on another thread. Stale IDs are accepted and ignored, so a stale request cannot cancel a newer activation. Cancellation of the current activation ends it with `Cancelled`.

The two engines run on dedicated threads for the lifetime of the owner. The VAD worker prepares the source, frames the audio, and joins the capture producer. The producer owns the device stream or the replay clock.

## Audio contract and supported input formats

`Audio` holds owned, contiguous `f32` samples at 16 kHz mono. `start_sample` is the first sample offset on the activation's normalized source timeline, and `end_sample` is `start_sample + len()`. The `Debug` output shows the bounds and length, never the sample values.

Accepted native input:
- Rates: 8, 16, 24, 32, 44.1, 48, 88.2, 96, 176.4, and 192 kHz.
- Channels: mono or stereo.
- Sample formats: `f32` or PCM16.

The normalizer mixes stereo to mono and resamples to 16 kHz. Replay and microphone input use the same path. The default input device must report a supported format. Non-finite audio returns `InvalidAudio`.

A ring overrun is a hard error (`CaptureOverrun`). Earshot does not silently drop audio.

## Features

The default build enables no features. It does not include `hound`, `serde`, or trace code.
- `serde` adds `Serialize` for `ActivationId`, `Timings`, and `IncompleteReason`.
- `tooling` is hidden and unstable. It is not semver-checked. It enables trace types and WAV and sample replay, and it implies `serde`.

`tooling` exists for the audit harness, not as public API. The `earshot-audit` crate is a test and qualification harness, also not public API.

A `tooling` trace can contain the captured audio samples. Treat traces as sensitive data. Do not log or persist them without the speaker's consent.

## Bundled model and licenses

Earshot embeds the Smart Turn v3.2 CPU ONNX model. The upstream revision is `f766f81`, and the model license is BSD-2-Clause. The SHA-256 is recorded in `src/smart_turn.rs` and `assets/NOTICE`.

Third-party notices are in:
- `assets/NOTICE`
- `assets/LICENSE.smart-turn`
- `assets/LICENSE.whisper-features-apache`
- `assets/LICENSE.whisper-features-bsd`
- `fixtures/LICENSE.silero-vad`

The project license is `MIT OR Apache-2.0`. The bundled model is not qualified.

## Build, test, and release-gate commands

Run commands from the repository root. The `Makefile` is the source of truth.

The current dependency graph enables `ort` binary download through `silero`.
A build can fetch an ONNX Runtime binary. If your environment requires offline
or audited builds, provide a local ONNX Runtime and review the `ort` build
features before you build.

```text
make fmt          # cargo fmt --all -- --check
make clippy       # workspace, all targets, all features, warnings denied
make test         # earshot: default, serde, and tooling builds
make test-audit   # earshot-audit
make test-all     # workspace, all features
make doc          # RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
make package      # cargo package -p earshot --list, then package
make deny         # cargo deny check
make audit        # cargo audit
```

Release-only checks:

```text
make release-gates
make release-gates-long
```

These run ignored release tests on the qualified platform. They are not part of the default test loop.

The MSRV is 1.88.0. CI also runs the workspace check on the pinned 1.88.0 toolchain.

The 1.0 verification run recorded these measurements on the development Mac. They are measurements, not general guarantees.

| Check | Result |
|---|---|
| Default tests | 38 unit tests and one doctest |
| `serde` tests | 38 unit tests and one doctest |
| `tooling` tests | 40 unit tests and one doctest |
| Audit tests | two unit tests, one golden replay, one accelerated capture test |
| Smart Turn | p50 34.6 ms, p95 34.8 ms, maximum 36.4 ms |
| Silero | maximum 112 microseconds per frame |
| Five-minute replay | 300.02 s elapsed, 20.7 ms backlog, 5.9 ms VAD maximum |
| Package | about 7.3 MiB compressed |
| MSRV | 1.88.0 |
| `cargo deny check` | pass |
| `cargo audit` | no vulnerabilities |

## Limits and non-goals

Earshot handles one statement at a time. One activation owns the service until the owner receives `Finished`.

Capture is bounded at 300 s of retained audio. At runtime, the stable build writes no files, uses no network, and emits no telemetry. Earshot does not transcribe, store, or upload audio.
