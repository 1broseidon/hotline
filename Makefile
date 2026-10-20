# The development instance runs on a data directory inside the checkout, so
# nothing done in it can reach real data.
dev: export HOTLINE_DATA_DIR := $(CURDIR)/.hotline-dev

.PHONY: check ui-check deps dev build verify icons tray-icons speech-engine precheck precheck-full hooks hooks-nudge

# What GitHub's runners would fail, caught on this Mac before a push: Windows
# by cross-compiling, each window test file alone, the workflows linted.
# The full run adds Linux in a container, a runner's bare PATH and a shuffled
# test order. docs/development.md says what each leg catches.
precheck:
	scripts/precheck

precheck-full:
	scripts/precheck --full

# The git hooks in .githooks: pre-commit in seconds, pre-push runs
# precheck. Worktrees share the setting, so every agent's commits pass the
# same gate. dev and check only say when it is off; turning it on is yours.
hooks:
	git config core.hooksPath .githooks

hooks-nudge:
	@[ "$$(git config --get core.hooksPath)" = .githooks ] || echo "git hooks are off: make hooks turns them on (docs/development.md#git-hooks)"

check: hooks-nudge ui-check deps speech-engine
	python3 scripts/security-citations
	python3 -m unittest discover -s scripts -p 'test_*.py'
	python3 -m unittest discover -s site/tests -p 'test_*.py'
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

# The desk's speech engine is linked from a prebuilt archive the build never
# downloads itself: this puts it in .speech-engine, checked against its
# pinned SHA-256 (scripts/speech-engine.sha256).
speech-engine:
	python3 scripts/fetch-speech-engine

# The window is TypeScript, and a window that does not compile is a broken
# build however green the Rust is. `bun install` is a no-op when the lockfile
# is already satisfied. Biome lints and never formats: the tree has one
# style already, and a formatter would reflow it for no invariant. A rule the
# tree does not pass yet is at warn in ui/biome.json until it is fixed.
#
# A filtered `cargo test session` writes only the types it matched into
# contract.ts, and the typecheck that follows is a page of "has no exported
# member" naming everything but the cause; any error that mentions the
# generated file means it is partial, so it is said in one line first.
#
# Knip fails on a window export, file or dependency nothing uses. Its Vite
# plugin reads both HTML entries for their scripts but not the stylesheet
# computer.html links, so computerViewer.css is exempt. contract.ts is the
# whole Rust contract, not only what the window reads, and cadence.ts,
# pacing.ts and reveal.ts export what Hotline Mobile copies.
ui-check:
	cd ui && bun install --frozen-lockfile
	@cd ui && if node_modules/.bin/tsc --noEmit | grep -q generated/contract; then \
		echo "ui/src/generated/contract.ts is partial: run cargo test -p hotline-contract, unfiltered, to write it whole"; exit 1; fi
	cd ui && bun run typecheck && bun run lint && bun run knip && bun run test

# The dependency gates. machete fails on a Cargo dependency nothing uses.
# cargo-deny fails on an advisory against Cargo.lock that deny.toml does not
# list with its review condition, and on a crate under a license nobody
# reviewed; scripts/window_licenses.py does the same for the window's
# packages, and THIRD-PARTY-NOTICES.md says what each license obliges and
# what neither tool reads (fonts, bundled builds, the speech engine). bun
# audit fails on any advisory, of any severity, against the window's
# lockfile. Both cargo tools are pinned, here and in CI, so a new release of
# either cannot fail a tree that passed: a different version stops with the
# line that installs this one. The advisory databases are fetched on every
# run, which is the point of the gate.
MACHETE_VERSION := 0.9.2
DENY_VERSION := 0.20.2
deps:
	@cargo machete --version 2>/dev/null | grep -qx '$(MACHETE_VERSION)' || { \
		echo "make check needs cargo-machete $(MACHETE_VERSION): cargo install --locked cargo-machete@$(MACHETE_VERSION)"; exit 1; }
	@cargo deny --version 2>/dev/null | grep -qx 'cargo-deny $(DENY_VERSION)' || { \
		echo "make check needs cargo-deny $(DENY_VERSION): cargo install --locked cargo-deny@$(DENY_VERSION)"; exit 1; }
	cargo machete --with-metadata
	cargo deny --locked check advisories licenses
	cd ui && bun install --frozen-lockfile
	python3 scripts/window_licenses.py
	cd ui && bun audit

# The Tauri CLI is a cargo subcommand (`cargo install tauri-cli --version ^2`).
# It runs from the shell crate, where tauri.conf.json is, and starts Vite for
# the window itself. On macOS the build goes through a runner that signs the
# debug binary, so the keychain stops asking on every launch; the script says
# why.
ifeq ($(shell uname -s),Darwin)
dev: DEV_RUNNER := --runner $(CURDIR)/scripts/cargo-dev-sign
endif
dev: hooks-nudge speech-engine
	cd crates/hotline-app && cargo tauri dev $(DEV_RUNNER)

# A release: the window built by Vite, the shell by cargo, bundled by the
# Tauri CLI into target/release/bundle (AppImage, deb and rpm on Linux).
build: speech-engine
	cd crates/hotline-app && cargo tauri build

# The headless harnesses under crates/hotline-core/tests drive the real core
# over the wire.
verify: speech-engine
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
	sed 's/currentColor/#6bcb62/' assets/hotline-mark-plain.svg | rsvg-convert -w 32 -h 32 -o crates/hotline-app/icons/tray.png -
	sed 's/currentColor/#000000/' assets/hotline-mark-plain.svg | rsvg-convert -w 44 -h 44 -o crates/hotline-app/icons/tray-template.png -

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
