.PHONY: fmt clippy test test-audit test-all doc package deny audit msrv ci release-gates release-gates-long

fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::too_many_lines

test:
	cargo test -p earshot
	cargo test -p earshot --features serde
	cargo test -p earshot --features tooling

test-audit:
	cargo test -p earshot-audit

test-all:
	cargo test --workspace --all-features

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

package:
	cargo package -p earshot --list
	cargo package -p earshot

deny:
	cargo deny check

audit:
	cargo audit

msrv:
	cargo msrv --min

ci: fmt clippy test test-audit test-all doc package deny audit

release-gates:
	cargo test -p earshot --release release_model_budgets -- --ignored --nocapture
	cargo test -p earshot --release release_normalization_budgets -- --ignored --nocapture

release-gates-long:
	cargo test -p earshot-audit --release --test capture release_five_minute_capture -- --ignored --nocapture
