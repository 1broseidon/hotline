# Changelog

All notable changes to hotline are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semver](https://semver.org/).

## [0.2.0] - 2026-07-03

### Added
- `HOTLINE.md` voice override: drop a file in your repo (or the state dir) to
  replace the channel's default persona. Mechanics and safety rules stay
  compiled in and always come first.
- Onboarding trio: `hotline setup` (save credentials once), `hotline init`
  (wire a project), `hotline start` (launch Claude Code with the channel
  loaded, `--yolo` to skip permission checks, args after `--` pass through).
- Official plugin path: `hotline init` installs the Claude Code plugin and
  enables it for the project; `hotline start` uses `--channels
  plugin:hotline@hotline` automatically once hotline is on the channel
  allowlist, and the dev flag until then. `--mcp-json` keeps the raw path.
- `hotline revoke <sender-id>`: remove an approved sender from the allowlist
  and purge their pending pairing codes.
- `hotline version` subcommand.
- mission-control: a best-practices template (`templates/mission-control/`)
  and a marketplace plugin whose `/mission-control:init` skill scaffolds a
  filing system, agent playbook, and starter voice into any project.
- email-sentry: a marketplace plugin that watches Gmail via `gog` and buzzes
  your channel only for mail that matters.
- `SECURITY.md`: threat model, defenses, known gaps, and a lost-phone runbook.
- Docs site at [hotline.dev](https://hotline.dev).

### Changed
- State moved to `${XDG_CONFIG_HOME:-~/.config}/hotline` with automatic
  one-time migration from the old location (left in place; legacy `TELE_GO_*`
  variables keep working for one release).
- Standard Go layout: entry point is `cmd/hotline`; `go install` path is
  `github.com/1broseidon/hotline/cmd/hotline@latest`.
- Channel instructions fit Claude Code's 2048-character cap: mechanics first,
  the voice gets the remaining budget and is truncated at a word boundary
  with a warning.

### Fixed
- More than half of the channel instructions were silently dropped by Claude
  Code's instruction cap.
- `HOTLINE_PROVIDERS` is documented as process environment (the `env` block
  of `.mcp.json` or project settings), not the state `.env`.

## [0.1.0] - 2026-07-02

Initial public release: Telegram, Signal, and Discord providers behind one
channel with a source router; pairing/allowlist access model; permission
relay; burst coalescing; media handling; Claude Code plugin marketplace;
brew tap.
