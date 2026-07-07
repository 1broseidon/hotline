# Changelog

All notable changes to hotline are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semver](https://semver.org/).

## [Unreleased]

### Added
- Multi-provider quick-view labels on the Claude Code path: with several
  providers configured, the inbound display name carries the provider
  ("George · signal"), so Claude Code's quick view — which renders only the
  server and user, dropping `meta.source` — shows where a message came from.
  Display-only: pairing and access key on `user_id`, single-provider setups
  are unchanged, and the OpenCode envelope (which already renders `source`)
  is untouched.

## [0.8.0] - 2026-07-07

### Added
- **Passcode gate on public publishes.** `publish` links exposed through a
  public tunnel (`localhostrun`, `cloudflared`) are no longer open to anyone
  who finds the hostname: visitors get a self-contained unlock page and enter
  a 6-digit code once, then hold a random 128-bit session cookie (HttpOnly,
  Secure, SameSite=Lax — never the code itself) for the life of the publish.
  The tool result is now a `Link:` line plus a `Passcode:` line, and that
  format is part of the feature: relayed into the chat verbatim, phones offer
  the code as one-time-code autofill from the recent message, so unlocking is
  tap link → tap code. The code is generated with crypto/rand, compared in
  constant time, and guarded by a global per-publish attempt limiter: ten
  wrong guesses hard-lock the publish (every request 404s from then on,
  correct code included, with one line on stderr; republish for a fresh link
  and code). Nothing secret rides the URL — link previews, browser history,
  and intermediary logs carry no token — and forwarding link + code together
  is sharing the artifact, exactly like forwarding the file. The `local`
  backend stays ungated: loopback is the operator's own machine. Neither the
  passcode nor session tokens are ever logged, and the publish server's error
  log is discarded so request state can never reach stderr.

### Fixed
- **Single-file publish no longer serves its parent directory.** Publishing
  one file used to expose every sibling in its directory (enumerable, with
  directory listings rendered). It now serves exactly that file at the link
  root and 404s every other path; the returned URL is the bare origin with no
  basename segment.
- **Signal: inbound reactions are no longer dropped.** Reacting to a message
  on Signal now reaches the agent as a `kind="reaction"` channel event (emoji
  as content, `reaction="added"|"removed"`, `target_message_id` in the
  adapter's timestamp-based id shape), matching the Telegram provider. Gated
  like a button tap: unpaired senders are dropped and a reaction never starts
  a pairing. Previously the receive path silently discarded reaction
  envelopes.

## [0.7.0] - 2026-07-07

### Added
- **Always-on supervisor: `hotline up` / `hotline down`.** `hotline start`'s
  always-on sibling: a self-contained supervisor (no systemd required) that
  owns the Claude Code process and keeps the agent alive until explicitly
  brought down. Claude runs on a supervisor-allocated pty (interactive Claude
  Code needs a controlling terminal; pure syscalls, Linux and macOS, zero new
  dependencies) in its own session/process group, and is restarted on any
  exit with exponential backoff — 2s doubling to a 10-minute ceiling, reset
  after 5 minutes of healthy uptime, never giving up — so a 3am crash no
  longer silently eats a 9am schedule: the restarted session's catch-up scan
  fires the overdue schedule exactly once. `hotline up` detaches by default;
  `--foreground` runs attached (the tmux/systemd shape) and it takes start's
  flags (`--yolo`, `--providers`, `--` passthrough re-applied on every
  respawn, so `-- --continue` resumes across restarts). A supervised session
  gains a `restart` MCP tool so the paired user can say "restart yourself":
  it only writes the supervisor's control file (argv/env/cwd stay fixed by
  the operator; the reason is only logged), the same path SIGHUP uses.
  State lives under `<state>/supervisor/` with the house flock/atomic-write
  discipline — liveness is the held lock, not a pid file — plus
  `supervisor.log` (event breadcrumbs) and `harness.log` (harness output,
  size-rotated at 5MB). `hotline status` reports the supervisor phase, pids,
  restart count, and last exit.
- **`hotline up` supervises both harnesses.** `HOTLINE_HARNESS` picks what
  runs, exactly like `hotline run`: claude (the default) on the supervisor
  pty, or `opencode serve` headless on plain pipes — no pty, same
  session/process-group and restart discipline. The serve port and hostname
  are derived from `OPENCODE_SERVER_URL` (default `http://127.0.0.1:4096`),
  the same source the hotline MCP child dials, so daemon and client always
  agree; `HOTLINE_SUPERVISOR_DIR` rides opencode's environment into the
  spawned hotline (verified: opencode passes its process env through to MCP
  children, merged with opencode.json's explicit env block), so the
  `restart` tool works on this path too — a restart bounces serve, and
  opencode's on-disk sessions re-attach via hotline's session pinning /
  most-recent selection. `--yolo` on the opencode path errors instead of
  being silently ignored (it maps to a claude-only flag; set opencode.json's
  `permission` block instead). Claude-path caveat, found live: until hotline
  is on Claude's approved channels allowlist, the dev-channel flag's
  per-launch confirmation makes each unattended respawn park on a prompt;
  the allowlist switch (automatic in `channelArgs`) is what makes claude
  always-on hands-off.

## [0.6.0] - 2026-07-07

### Added
- **Proactive scheduling.** hotline can now fire scheduled prompts at future
  times, delivered as synthetic inbound turns through the same message path a
  real message uses (tagged `kind="schedule"`), so the agent acts on them with
  full tool access and normal permission gating — reminders, recurring
  check-ins, deferred work. A new `schedule` MCP tool lets the agent
  `create`/`list`/`cancel` schedules from chat; recurrence is a preset enum
  (`once`, `daily`, `weekly`, `every_n_hours`, `every_n_days`), not cron. A
  one-off's fire time accepts a relative offset (`+2m`, `+1h30m`, units h/m/s)
  as well as an absolute time, so "remind me in 5 minutes" never requires the
  agent to check the clock itself first. State persists to `schedules.json` at
  the state root under the same flock/atomic write discipline as
  `access.json`, so mutations apply live to a running daemon. A 10s ticker
  plus one eager catch-up scan at startup means an overdue schedule fires
  exactly once (persist-before-inject: the next fire time is advanced under
  the lock before the turn is injected, preventing double-fires). Operators
  get a `hotline schedule list|remove|pause|resume` CLI (pause/resume are the
  operator kill-switch and are deliberately not agent-accessible). Times are
  server-local (`time.Local`); a configurable timezone is deferred.

## [0.5.0] - 2026-07-06

### Removed
- **Codex harness support**, added in 0.4.0. Live use surfaced enough rough
  edges to pull it rather than ship it half-working: no MCP tool surface in
  Phase 1 (replies only forward directly, no `react`/`edit_message`/
  attachments), denying a command approval ends the whole turn instead of
  letting the agent try something else, `codex app-server`'s sandbox
  doesn't initialize on some Linux setups (a bubblewrap/AppArmor-
  unprivileged-userns restriction), and — the one with no workaround —
  `thread/resume` ignores any updated `developerInstructions`, so a voice or
  behavior fix can't reach an already-running thread without abandoning its
  context. `HOTLINE_HARNESS=codex`, `hotline init --harness codex`, and
  `hotline start --harness codex` are gone; `claude` and `opencode` remain
  the two supported harnesses. The removed code lives on the
  `experiment/codex-harness` branch for whenever `codex app-server` (or
  hotline's Phase 2 design for it) is further along.

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
