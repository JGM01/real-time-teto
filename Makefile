TOOLCHAIN_BIN := $(dir $(shell rustup which --toolchain 1.88.0 rustc))
export PATH := $(TOOLCHAIN_BIN):$(PATH)
export RUSTC := $(TOOLCHAIN_BIN)rustc
export RUSTDOC := $(TOOLCHAIN_BIN)rustdoc
CARGO := $(TOOLCHAIN_BIN)cargo

.PHONY: fmt clippy test doc build build-release check policy qualify phone-build
fmt:
	$(CARGO) fmt --all -- --check
format:
	$(CARGO) fmt --all
	$(CARGO) fmt --manifest-path phone-spike/Cargo.toml
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings -D clippy::too_many_lines
test:
	$(CARGO) test --workspace $(TEST_FILTER)
doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps
build:
	$(CARGO) build --workspace
build-release:
	$(CARGO) build --workspace --release
check: fmt clippy test doc
policy:
	$(CARGO) run --quiet -p teto-llm-protocol --example render_policy
qualify:
	$(CARGO) test --release -p teto-llm --test real_model $(TEST_FILTER) -- --ignored --test-threads=1
cache-probe:
	$(CARGO) run --release -p teto-llm --example cache_probe
keychain-test:
	$(CARGO) test -p teto-cli keychain_adapter -- --ignored --test-threads=1
replay-test:
	$(CARGO) test --release -p teto-core replay_speech_to_clauses -- --ignored --test-threads=1
isolation-test:
	$(CARGO) test --release -p teto-core concurrent_native_inference -- --ignored --test-threads=1 --nocapture
phone-build:
	IPHONEOS_DEPLOYMENT_TARGET=17.0 CMAKE_OSX_DEPLOYMENT_TARGET=17.0 BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios="-isysroot $$(xcrun --sdk iphoneos --show-sdk-path)" $(CARGO) build --manifest-path phone-spike/Cargo.toml --release --target aarch64-apple-ios
phone-clean-native:
	$(CARGO) clean --manifest-path phone-spike/Cargo.toml -p llama-cpp-sys-2 --release --target aarch64-apple-ios
phone-link: phone-build
	xcrun swiftc -parse-as-library -sdk "$$(xcrun --sdk iphoneos --show-sdk-path)" -target arm64-apple-ios17.0 -module-cache-path /tmp/teto-swift-module-cache -import-objc-header phone-spike/App/TetoPhoneSpike.h phone-spike/App/TetoPhoneSpikeApp.swift -L phone-spike/target/aarch64-apple-ios/release -lteto_phone_spike -lc++ -framework Metal -framework Foundation -framework Accelerate -o phone-spike/target/TetoPhoneSpike
