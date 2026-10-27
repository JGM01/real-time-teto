# iPhone feasibility harness

This uses the same `teto-llm` crate directly. It is a measurement harness, not a phone runtime qualification. It explicitly disables journaling. Its clause sink acknowledges completed clauses for orchestration measurements only; it produces no audio and does not prove playback.

Target: physical iPhone 15 Pro, 8 GB. Copy the exact `PHONE_FILE` artifact from the policy registry into the app's Documents directory. Verify `PHONE_SHA256` before import. No multimodal projector is needed.

Build the isolated ABI library with Rust 1.88 and the installed iPhone SDK:

```sh
rustup target add aarch64-apple-ios --toolchain 1.88.0
make phone-build
```

In a new iOS SwiftUI app, add `App/TetoPhoneSpikeApp.swift`, use `App/TetoPhoneSpike.h` as the Objective-C bridging header, and link `target/aarch64-apple-ios/release/libteto_phone_spike.a`. Link libc++, Metal, Foundation and Accelerate. Enable Documents file sharing (`UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace`) to import the model and export reports. Choose the user's signing team; no signing identity is stored here.

The early feasibility run measures at least ten turns and five minutes in the foreground, plus an immediate cancellation probe. This is separate from full device qualification. Use Instruments to record peak memory, thermal state and termination diagnostics. The JSONL report contains metrics and outcome metadata only. Record exact hardware and OS with the report. Simulator results cannot qualify memory or thermals.

The FFI shim contains the only application-owned unsafe code: bounded UTF-8 pointer conversion and the C export. The engine and desktop application forbid unsafe code. Do not call the synchronous export from the UI thread or call it concurrently.

A ready Xcode project is provided at `TetoPhoneSpike.xcodeproj`. Run `make phone-build` first, open that project, select the paired device, and select the Apple development team in Signing & Capabilities. Xcode records the user-selected team ID in the project; certificates and account credentials remain in Keychain. A valid development certificate and provisioning profile are required before installation. The paired iPhone 15 Pro Max (iPhone16,2, iOS 27.0.1) is authorized as a separate feasibility measurement; its results never qualify the specified iPhone 15 Pro.

For a device explicitly authorized for this task, `TETO_PHONE_DEVICE=<devicectl-identifier> zsh tools/prepare_phone.sh` builds, signs, installs, copies the checksum-pinned model and launches the measurement. It never uninstalls other apps. Apple Personal Teams have a limited number of installed development apps; freeing a slot and trusting the development profile may require user action. Results and sampled resources are separate JSONL files in the app's Documents directory. The model is not copied back with the reports.
