# The development instance runs on a data directory inside the checkout, so
# nothing done in it can reach real data.
dev: export HOTLINE_DATA_DIR := $(CURDIR)/.hotline-dev

.PHONY: check ui-check dev build verify icons tray-icons

check: ui-check
	python3 -m unittest discover -s scripts -p 'test_*.py'
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
# the window itself. On macOS the build goes through a runner that signs the
# debug binary, so the keychain stops asking on every launch; the script says
# why.
ifeq ($(shell uname -s),Darwin)
dev: DEV_RUNNER := --runner $(CURDIR)/scripts/cargo-dev-sign
endif
dev:
	cd crates/hotline-app && cargo tauri dev $(DEV_RUNNER)

# A release: the window built by Vite, the shell by cargo, bundled by the
# Tauri CLI into target/release/bundle (AppImage, deb and rpm on Linux).
build:
	cd crates/hotline-app && cargo tauri build

# The headless harnesses drive the real core over the wire; Phase 0 adds the first.
verify:
	cargo test --workspace --test '*'

# Every platform's app icon, rendered from the one tile in assets/. The CLI
# also writes Android and iOS sets; there is no phone here yet, so they go.
# The tray is the mark without the tile: 32 pixels of tile is a blob, and a
# macOS template image has to be solid black so the OS can tint it. The
# accent is the tile's, spelled here because rsvg reads no currentColor from
# the page.
icons: tray-icons
	cd crates/hotline-app && cargo tauri icon ../../assets/hotline-tile.svg -o icons && rm -rf icons/android icons/ios

tray-icons:
	sed 's/currentColor/#6bcb62/' assets/hotline-mark.svg | rsvg-convert -w 32 -h 32 -o crates/hotline-app/icons/tray.png -
	sed 's/currentColor/#000000/' assets/hotline-mark.svg | rsvg-convert -w 44 -h 44 -o crates/hotline-app/icons/tray-template.png -

## The website: the landing page in site/ with the docs built under it at /docs.
.PHONY: site site-deploy og-card toad-redirect-deploy
site:
	cd docs-site && bun install --frozen-lockfile && bun run build

site-deploy: site
	cd site && npx wrangler@latest deploy

## The social card every shared link shows, rendered from site/card/og.html.
## It used to be a committed binary with no source, which is how it went on
## saying Toad. The script embeds the fonts, renders headless, and checks the
## result against the geometry it is meant to hold.
og-card:
	python3 site/card/render.py

## The old domains, answering 301 from hotline.dev. Deployed on its own: it
## changes only when the redirect does.
toad-redirect-deploy:
	cd site/redirect && npx wrangler@latest deploy
