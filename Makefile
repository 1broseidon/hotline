# The development instance runs on a data directory inside the checkout, so
# nothing done in it can reach real data.
dev: export TOAD_DATA_DIR := $(CURDIR)/.toad-dev

.PHONY: check ui-check dev build verify icons

check: ui-check
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

# The window is TypeScript, and a window that does not compile is a broken
# build however green the Rust is. `bun install` is a no-op when the lockfile
# is already satisfied.
ui-check:
	cd ui && bun install --frozen-lockfile && bun run typecheck

# The Tauri CLI is a cargo subcommand (`cargo install tauri-cli --version ^2`).
# It runs from the shell crate, where tauri.conf.json is, and starts Vite for
# the window itself.
dev:
	cd crates/toad-desktop && cargo tauri dev

# A release: the window built by Vite, the shell by cargo, bundled by the
# Tauri CLI into target/release/bundle (AppImage, deb and rpm on Linux).
build:
	cd crates/toad-desktop && cargo tauri build

# The headless harnesses drive the real core over the wire; Phase 0 adds the first.
verify:
	cargo test --workspace --test '*'

# Every platform's app icon, rendered from the one tile in assets/. The CLI
# also writes Android and iOS sets; there is no phone here yet, so they go.
icons:
	cd crates/toad-desktop && cargo tauri icon ../../assets/toad-tile.svg -o icons && rm -rf icons/android icons/ios
