TOOLCHAIN_BIN := $(dir $(shell rustup which --toolchain 1.88.0 rustc))
export PATH := $(TOOLCHAIN_BIN):$(PATH)
export RUSTC := $(TOOLCHAIN_BIN)rustc
export RUSTDOC := $(TOOLCHAIN_BIN)rustdoc
CARGO := $(TOOLCHAIN_BIN)cargo

.PHONY: fmt clippy test doc build build-release check comments fuzz-check qualify phone-build speech-qualify speech-fuzz
fmt:
	$(CARGO) fmt --all -- --check
	$(CARGO) fmt --manifest-path fuzz/Cargo.toml -- --check
format:
	$(CARGO) fmt --all
	$(CARGO) fmt --manifest-path phone-spike/Cargo.toml
	$(CARGO) fmt --manifest-path fuzz/Cargo.toml
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings -D clippy::too_many_lines
test:
	$(CARGO) test --workspace $(TEST_FILTER)
test-features:
	$(CARGO) test -p teto-speech --features llm-adapter
	$(CARGO) test -p teto-speech --features cmudict,g2p-gru,voicebank-wav,llm-adapter
	$(CARGO) clippy -p teto-speech --all-targets --features cmudict,g2p-gru,voicebank-wav,llm-adapter -- -D warnings -D clippy::too_many_lines
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc -p teto-speech --features cmudict,g2p-gru,voicebank-wav,llm-adapter --no-deps
doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps
build:
	$(CARGO) build --workspace
build-release:
	$(CARGO) build --workspace --release
check: fmt clippy test test-features doc comments fuzz-check
comments:
	python3 tools/check_comments.py
fuzz-check:
	$(CARGO) check --manifest-path fuzz/Cargo.toml --all-targets
qualify:
	$(CARGO) test --release -p teto-llm --features measurement --test real_model $(TEST_FILTER) -- --ignored --test-threads=1
cache-probe:
	$(CARGO) run --release -p teto-llm --features measurement --example cache_probe
keychain-test:
	$(CARGO) test -p teto-cli keychain_adapter -- --ignored --test-threads=1
replay-test:
	$(CARGO) test --release -p teto-core replay_speech_to_clauses -- --ignored --test-threads=1
isolation-test:
	$(CARGO) test --release -p teto-core concurrent_native_inference -- --ignored --test-threads=1 --nocapture
fuzz-smoke:
	cd fuzz && cargo +nightly fuzz run text -- -runs=1000
	cd fuzz && cargo +nightly fuzz run frame -- -runs=1000

phone-build:
	IPHONEOS_DEPLOYMENT_TARGET=17.0 CMAKE_OSX_DEPLOYMENT_TARGET=17.0 BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios="-isysroot $$(xcrun --sdk iphoneos --show-sdk-path)" $(CARGO) build --manifest-path phone-spike/Cargo.toml --release --target aarch64-apple-ios
phone-clean-native:
	$(CARGO) clean --manifest-path phone-spike/Cargo.toml -p llama-cpp-sys-2 --release --target aarch64-apple-ios
phone-link: phone-build
	xcrun swiftc -parse-as-library -sdk "$$(xcrun --sdk iphoneos --show-sdk-path)" -target arm64-apple-ios17.0 -module-cache-path /tmp/teto-swift-module-cache -import-objc-header phone-spike/App/TetoPhoneSpike.h phone-spike/App/TetoPhoneSpikeApp.swift -L phone-spike/target/aarch64-apple-ios/release -lteto_phone_spike -lc++ -framework Metal -framework Foundation -framework Accelerate -o phone-spike/target/TetoPhoneSpike

speech-qualify:
	python3 tools/qualify_speech.py --bank voicebanks/TETO-English-150401.zip
speech-fuzz:
	mkdir -p target/p3/fuzz-corpus/oto target/p3/fuzz-corpus/alias_planner target/p3/fuzz-corpus/lexicon target/p3/fuzz-corpus/import
	PATH="$(CURDIR)/target/p3/tools/bin:$(PATH)" RUSTC="$$(rustup which --toolchain nightly rustc)" RUSTDOC="$$(rustup which --toolchain nightly rustdoc)" rustup run nightly cargo fuzz run oto target/p3/fuzz-corpus/oto fuzz/seeds/oto -- -runs=10000 -max_len=4096 -timeout=5
	PATH="$(CURDIR)/target/p3/tools/bin:$(PATH)" RUSTC="$$(rustup which --toolchain nightly rustc)" RUSTDOC="$$(rustup which --toolchain nightly rustdoc)" rustup run nightly cargo fuzz run alias_planner target/p3/fuzz-corpus/alias_planner fuzz/seeds/alias_planner -- -runs=10000 -max_len=4096 -timeout=5
	PATH="$(CURDIR)/target/p3/tools/bin:$(PATH)" RUSTC="$$(rustup which --toolchain nightly rustc)" RUSTDOC="$$(rustup which --toolchain nightly rustdoc)" rustup run nightly cargo fuzz run lexicon target/p3/fuzz-corpus/lexicon -- -runs=10000 -max_len=4096 -timeout=5
	PATH="$(CURDIR)/target/p3/tools/bin:$(PATH)" RUSTC="$$(rustup which --toolchain nightly rustc)" RUSTDOC="$$(rustup which --toolchain nightly rustdoc)" rustup run nightly cargo fuzz run import target/p3/fuzz-corpus/import -- -runs=1000 -max_len=4096 -timeout=5
