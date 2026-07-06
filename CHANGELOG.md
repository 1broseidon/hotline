# Changelog

All notable changes to hotline are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semver](https://semver.org/).

## [0.4.0] - 2026-07-06

### Added
- **Codex harness support**: hotline now drives OpenAI Codex CLI's experimental
  `app-server` as a third harness (`HOTLINE_HARNESS=codex`), alongside Claude
  Code and OpenCode. Unlike OpenCode's daemon, `codex app-server` has no
  dial-back address over stdio, so hotline owns it as a spawned subprocess and
  is its sole JSON-RPC client: one thread per hotline instance, persisted and
  resumed across restarts (including a full process restart, not just a
  client reconnect), with the approval relay reusing the same code/TTL-cache
  pattern as the OpenCode adapter. Persona and safety instructions ride
  `thread/start`'s `developerInstructions` field, so `hotline init --harness
  codex` needs no scaffolded project file. This is a Phase 1 adapter: replies
  forward directly to the channel rather than through hotline's MCP tools, so
  `react`, `edit_message`, and attachment downloads aren't available yet, and
  denying a command approval ends the whole turn rather than letting Codex
  try something else.
- `hotline start --harness codex`, with full `--yolo` parity: it sets both
  `HOTLINE_CODEX_APPROVAL_POLICY=never` and `HOTLINE_CODEX_SANDBOX=danger-full-
  access` together, since they're independent knobs — approval policy alone
  only skips the confirmation prompt, while commands can still fail outright
  wherever the sandbox itself can't initialize (a bubblewrap/AppArmor-
  unprivileged-userns restriction seen on Ubuntu 23.10+). `--harness opencode`
  now returns a clear rejection instead of silently doing nothing, since
  OpenCode spawns hotline rather than the other way around.
- Codex added to the quickstart docs as a third tab (Claude Code, Codex,
  OpenCode) on [hotline.dev](https://hotline.dev/docs/).

## [0.3.0] - 2026-07-06

### Added
- **OpenCode harness support**: hotline now drives OpenCode alongside Claude
  Code. `hotline init --harness opencode` scaffolds a dedicated primary agent
  (`.opencode/agents/hotline.md`) whose system prompt is hotline's mechanics
  and voice — OpenCode ignores the MCP `instructions` field, so this is how
  the channel's safety rules and register reach it. `HOTLINE_HARNESS` selects
  the harness; inbound messages push in via OpenCode's session API and render
  through a shared `<channel>` envelope so the agent gets `chat_id`/`source`
  regardless of harness.
- `publish`: an MCP tool that hosts a local artifact (a folder or a single
  HTML file) at a public, temporary link — a static server plus a quick
  tunnel, zero accounts, zero config. The exposure backend is pluggable and
  operator-selected (`HOTLINE_PUBLISH_EXPOSURE`): `localhostrun` (default,
  zero-install), `cloudflared` (if the binary is present), or `local`
  (loopback only, for operators who front it themselves). A safe-path guard
  refuses to publish the filesystem root, the home directory, the working
  directory or its parents, or a directory containing `.git`/`.env`/ssh or
  cloud credentials. Tunnel subprocesses die with hotline (`Pdeathsig`)
  instead of orphaning, and a shutdown hook tears down every published
  artifact at once.
- `docs/defaults` on hotline.dev: a documented "sane defaults" permission
  profile for both harnesses — auto-allow reads and in-project edits, keep
  external writes and shell commands gated, deny secrets outright. The
  middle ground between asking every time and `--yolo`.
- A steering guardrail against tools that block on a local terminal prompt
  (a multiple-choice question, a plan approval): the channel's remote user
  can't answer them, so the session freezes. Agents are told to ask as a
  normal message and use `reply`'s buttons for a pick-one instead.

### Changed
- Default voice is friendlier and more casual out of the box (a "register"
  pass on the built-in persona), and the default permission posture is less
  chatty: safe, everyday work (reads, in-project edits, read-only navigation)
  goes through without a prompt; anything reaching outside the project or
  running an arbitrary command still asks.
- OpenCode-specific steering (write with the edit tool rather than shell,
  prefer hotline's own tools like `publish` over a general skill that does
  something similar) ships in the OpenCode agent file only — it doesn't cost
  any of Claude Code's instruction budget.

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
