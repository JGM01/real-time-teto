# iPhone feasibility harness

This harness uses the same `teto-llm` crate on a physical phone. It measures text-to-clause latency and the cache probe. It disables journaling. The clause sink acknowledges clauses for orchestration measurements only. It produces no audio and does not prove playback. The iPhone 15 Pro Max is the current text and cache qualification device; audio remains pending.

The harness builds with the `measurement` feature.

Copy the exact `PHONE_FILE` artifact into the app Documents directory. Verify `PHONE_SHA256` before import. No multimodal projector is needed.

Build the isolated ABI library with Rust 1.88 and the installed iPhone SDK:

```sh
rustup target add aarch64-apple-ios --toolchain 1.88.0
make phone-build
```

Add `App/TetoPhoneSpikeApp.swift` to a SwiftUI app, use `App/TetoPhoneSpike.h` as the bridging header, and link `target/aarch64-apple-ios/release/libteto_phone_spike.a`. Link libc++, Metal, Foundation and Accelerate. Enable Documents file sharing. Choose the signing team in Xcode. No signing identity is stored here.

The harness measures at least ten turns and five minutes in the foreground, plus an immediate cancellation probe. Use Instruments for peak memory, thermal state and termination diagnostics. The JSONL report contains metrics and outcome metadata only. Record the exact hardware and OS with the report. Simulator results cannot qualify memory or thermals.

The FFI shim contains the only application-owned unsafe code: bounded UTF-8 pointer conversion and the C export. Do not call the synchronous export from the UI thread or call it concurrently.

A ready Xcode project is at `TetoPhoneSpike.xcodeproj`. Run `make phone-build`, open the project, select the paired device and select the Apple development team. Certificates and account credentials remain in Keychain.

For an authorized device, `TETO_PHONE_DEVICE=<devicectl-identifier> zsh tools/prepare_phone.sh` builds, signs, installs, copies the checksum-pinned model and launches the measurement. It never uninstalls other apps. Results and resource samples are separate JSONL files in the app Documents directory. The model is not copied back with the reports.

The Pro Max cache probe and cache run are recorded in [qualification](../teto-llm/QUALIFICATION.md). Prefix reuse is enabled for the Pro Max triple. The probe and the run are separate actions in the app.
