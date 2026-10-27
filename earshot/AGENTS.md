# AGENTS.md

## 1. Purpose and product boundary

Earshot is an internal Rust library in the Teto workspace. It captures one spoken
statement at a time and decides when that statement ends. It returns owned 16 kHz
mono audio to the caller.

Keep the capture API independent of transcription, language models, and playback.

The product boundary is:

- The stable surface is the set of items re-exported at the crate root.
- The hidden `tooling` module and the `earshot-audit` crate are not public API.
- Earshot does not transcribe, store, or upload audio.
- The stable build writes no files, uses no network, and emits no telemetry.
- The microphone stays closed while the service is idle.

## 2. Repository map

| Path | Role |
|---|---|
| `Cargo.toml` | Package metadata, features, pinned dependencies |
| `../Cargo.toml` | Workspace members and shared dependencies |
| `../Cargo.lock` | Locked dependency graph |
| `Makefile` | Canonical commands |
| `src/lib.rs` | Crate docs, stable re-exports, hidden `tooling` seam |
| `src/control.rs` | `Earshot`, `Control`, admission, receipt, close |
| `src/coordinator.rs` | State machine, clocks, cancellation, tracing |
| `src/endpoint.rs` | Speech tracking, snapshots, retained audio |
| `src/vad.rs` | Silero worker and messages |
| `src/turn.rs` | Semantic endpoint worker |
| `src/smart_turn.rs` | ONNX session, tensor contract, warm-up |
| `src/smart_turn/features.rs` | Whisper log-mel features |
| `src/capture/mod.rs` | Input source, pacing, watermark, format validation |
| `src/capture/source.rs` | Microphone capture and replay sources |
| `src/capture/pipeline.rs` | Source to frames to VAD |
| `src/capture/ring.rs` | Native-frame ring buffer |
| `src/capture/normalize.rs` | Mixing and resampling |
| `src/policy.rs` | Development policy values |
| `src/runtime.rs` | Thread spawn, panic capture, bounded sends |
| `src/session.rs` | Shared state and mailbox |
| `src/constants.rs` | Rates, sizes, queue capacities |
| `src/trace/mod.rs` | Trace feature selection |
| `src/trace/enabled.rs` | Bounded diagnostic types |
| `src/trace/disabled.rs` | Zero-storage trace facade |
| `src/tooling.rs` | Hidden cross-crate seam |
| `earshot-audit/` | Qualification grid and replay harness; not public API |
| `assets/` | Bundled model and third-party notices |
| `fixtures/` | Test audio and reference data |
| `docs/` | Internal architecture notes |
| `.github/workflows/ci.yml` | CI commands and platform matrix |
| `deny.toml` | Dependency and license policy |
| `clippy.toml` | Lint thresholds |

## 3. Canonical commands

Use the `Makefile`. Do not invent build, test, or release commands.

```text
make fmt
make clippy
make test
make test-audit
make test-all
make doc
make package
make deny
make audit
```

Release-only commands:

```text
make release-gates
make release-gates-long
```

These commands run ignored release tests on the qualified platform. Do not add
them to the default test loop.

Documentation acceptance is the same as the matching Makefile targets:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::too_many_lines
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace --all-features
```

The MSRV is 1.88.0. CI checks the workspace with the pinned 1.88.0 toolchain.

## 4. Architecture invariants

- The coordinator owns activation state. Only the coordinator changes `Idle`,
  `Capturing`, `Finalizing`, and `OutcomePending`.
- `Earshot` owns the event receiver. `Control` is the cross-thread handle.
  `Earshot` is `Send`, not `Sync`. Keep the event loop on one thread.
- One accepted attempt produces one terminal outcome. It emits at most one
  `Listening` event and exactly one `Finished` event, unless the owner closes. A
  startup failure can finish before `Listening`.
- Receipt releases admission. The owner releases the next activation when
  `recv` or `recv_timeout` receives `Finished`. There is no separate
  acknowledgment.
- The microphone stays closed while idle. `Earshot::open` loads and warms the
  engines only. Capture opens the device for an accepted activation.
- Audio is owned, mono, and 16 kHz. `start_sample` is on the activation's
  normalized source timeline. Do not return borrowed capture buffers.
- All queues are bounded. Capacities are in `src/constants.rs`. Do not add an
  unbounded queue or a blocking send in the audio callback.
- The default build has no trace code. The disabled trace facade uses zero
  storage. Keep trace code behind `tooling`.
- The endpoint owns the retained waveform. Inference must not change the
  waveform. Only a current positive semantic decision can create `Complete`.
- A positive decision becomes usable only when capture reaches the decision
  target. Do not bypass the coordinator's accept step.
- A worker failure closes the service. The coordinator publishes one `Failed`
  outcome for an active attempt and sets shutdown.

## 5. Policy and qualification rules

Policy values live in `src/policy.rs`. The current values are a development
baseline: 1024 ms minimum pause, 0.01 completion threshold, 5 s no-speech limit,
5 s silence limit, and 300 s maximum capture.

- Do not change a policy value without a new qualification.
- Do not describe the baseline as qualified.
- Do not claim that the model or corpus is qualified.
- Keep timer fallbacks incomplete. Only a current positive semantic decision
  creates `Complete`.
- The qualification pause grid is `[1024, 1280, 1536, 1792, 2048]` ms. The
  threshold range is `0.01..=0.99`. The `earshot-audit` `Candidate` type
  enforces both.
- The audit harness replays audio through the production workers. It does not
  change the production policy and does not qualify anything by itself.
- Do not add a qualification claim to README, rustdoc, package metadata, or CI
  names.

## 6. Documentation rules

- Write in Simplified Technical English. Use short sentences, active voice, and
  one instruction per sentence.
- Write Tiger Style: explicit, direct, and no clever wording. Define terms.
  Avoid marketing language.
- Rustdoc is the API reference. Keep public item documentation on the item.
- Do not duplicate API tables or policy tables across files. Link to the owning
  file or the rustdoc item.
- Keep the README example consistent with the crate-level example in
  `src/lib.rs`. There is no README-to-doctest bridge.
- Do not invent benchmarks. Quote only verified measurements and label the
  platform. Do not compare Earshot with other crates.
- Do not invent a repository URL, homepage, CI badge, version history, or
  support policy.
- Do not edit legal notices in `assets/` or `fixtures/`. They are third-party
  licenses, not project documentation.
- Do not describe `earshot-audit` as public API. Do not describe `tooling` as
  stable.
- Do not document a command unless it is in the `Makefile` and you ran it, or
  mark it as CI-only or release-only.

## 7. Release gates and MSRV

- MSRV: 1.88.0. Edition: 2024. Resolver: 3.
- The release check is `make fmt`, `make clippy`, `make test`, `make test-audit`,
  `make test-all`, `make doc`, `make package`, `make deny`, and `make audit`.
- The documentation check must use `RUSTDOCFLAGS="-D warnings"`.
- Run the release-only gates on the qualified platform before a release.
- Do not raise the MSRV, add a dependency, or change feature semantics without
  an explicit release decision.
- Keep `publish = false` in both packages unless the owner asks for a change.
- Do not add a binary target. The repository is a library and a test harness.
- Keep dependency pins intentional. `ort` is pinned at `2.0.0-rc.12`.
- No semver baseline exists until the first 1.0 tag. Run a semver check only
  against a real published baseline.

## 8. Known risks

- `ort` is a pinned release candidate. Its API and behavior can change.
- The dependency graph enables `ort` binary download through `silero`. A build
  can fetch an ONNX Runtime binary. Use an audited local runtime for offline or
  restricted builds.
- `tooling` traces contain raw audio samples. `Trace` also implements `Debug`
  and `Serialize`, so do not log or persist traces without consent.
- `tooling` WAV replay opens a caller-provided path. Do not pass untrusted paths
  to the audit harness.
- `Control` clones can activate and cancel. Treat a clone as a capability and
  give it only to trusted threads.
- CI actions are pinned by release tag, not commit SHA. Pin them by SHA if the
  threat model requires it.
- Documentation builds on the qualified platform are the release check. If the
  crate is later published, a docs.rs build runs on a non-macOS host and is best
  effort.
- The public API has no semver baseline until the first 1.0 tag.
- The policy, model, and corpus are not qualified.
- Only macOS on Apple silicon is qualified. Other platforms may compile, but
  this project makes no support claim for them.
- The bundled model makes the package large. The 1.0 package measured about
  7.3 MiB compressed. `fixtures/` and `earshot-audit/` are excluded from the
  `earshot` package.
- `Control::activate` blocks until the coordinator replies. Call it from a
  thread that can wait.
- `Earshot::close` can wait for an in-flight model call. Do not assume it is
  instantaneous.
- There is no README-to-doctest bridge. Keep examples synchronized by hand.
- Use `earshot-audit::Candidate` for replay configuration. It enforces the
  fixed pause grid and threshold range.

## 9. Engineering rules

- Prefer explicit types, names, and control flow.
- Keep functions short. The Clippy threshold is 70 lines.
- Keep every queue bounded and every wait bounded.
- Keep ownership explicit. The coordinator owns state. Workers own engines. The
  source owns the device stream or the replay clock.
- Do not use `unsafe`. The crate forbids it.
- Return `Result` for recoverable failures. Use panics only for internal
  invariants.
- Do not allocate, lock, or log in the audio callback. Use the preallocated
  ring.
- Add tests for every behavior change. Use property tests for audio boundaries
  and malformed input.
- Measure before making a performance claim. Use the release-only gates.
- Keep the public surface small. Do not expose internals to make tests easier.
  Use the hidden `tooling` seam.
- Do not edit the bundled model or its checksum. Do not edit third-party
  notices.
