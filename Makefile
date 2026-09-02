# The development instance runs on a data directory inside the checkout, so
# nothing done in it can reach real data.
dev: export TOAD_DATA_DIR := $(CURDIR)/.toad-dev

.PHONY: check dev verify

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

# The Tauri CLI is a cargo subcommand (`cargo install tauri-cli --version ^2`).
# It runs from the shell crate, where tauri.conf.json is, and starts Vite for
# the window itself.
dev:
	cd crates/toad-desktop && cargo tauri dev

# The headless harnesses drive the real core over the wire; Phase 0 adds the first.
verify:
	cargo test --workspace --test '*'
