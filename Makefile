# The development instance runs on a data directory inside the checkout, so
# nothing done in it can reach real data.
dev: export TOAD_DATA_DIR := $(CURDIR)/.toad-dev

.PHONY: check dev verify

check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

# Phase 1 gives this a shell to run and a window to open.
dev:
	@echo "there is no shell yet; see docs/design.md, Phase 1" && exit 1

# The headless harnesses drive the real core over the wire; Phase 0 adds the first.
verify:
	cargo test --workspace --test '*'
