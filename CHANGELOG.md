# Changelog

All notable changes to hotline are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[semver](https://semver.org/).

## [Unreleased]

## [0.11.0] - 2026-07-25

The v2 line. 318 commits since 0.10.0: a first-party mobile/web client and the
app channel that feeds it, a hosted relay with three frozen wire contracts, a
box-to-box fleet, Mission Control, and four harnesses behind one `hotline up`.

### Added
- **The app channel — a first-party client transport.** A direct WebSocket
  channel beside Telegram/Signal/Discord, with a persisted outbox so replay
  survives a restart, a full-thread journal, user-send echo, attachments,
  history paging, images, and reactions. Provider-aware channel/formatting
  instructions; markdownv2/HTML replies are down-converted to CommonMark for the
  app. An inbound coalescer with a typing hold gate (window and grace unified at
  3s, tunable via `HOTLINE_APP_COALESCE_WINDOW`) turns a burst of taps into one
  turn.
- **The hotline mobile client.** A separate closed-source app that speaks the
  app channel: pairing by QR and deep link, a conversation switcher, staged
  multi-photo compose, artifact cards in a no-network web sandbox, a per-bot
  apps drawer, an agent page (NOW / SCHEDULED / LOOPS / APPS), and the hotline
  mark (icon, adaptive icon, splash, favicon). It is developed and released
  outside this repository.
- **Push notifications, end to end.** Expo push for the app channel, the full
  native lifecycle, per-device wake pre-gating, an explicit presence frame with
  a 60s box-side liveness lease, a signed push-gateway path (ECDSA P-256, gated
  behind `HOTLINE_PUSH_ENDPOINT`), optional clear previews
  (`HOTLINE_PUSH_PREVIEW=clear`) and a per-device preview preference (FB23),
  `mutableContent` so the iOS notification-service extension runs (FB56),
  successful-job-completion pushes (FB44), and job Live Activity updates
  delivered directly to APNs (FB64).
- **The hosted relay is the default rendezvous.** `relay.hotline.dev` is baked
  in: an outbound relay connector, relay photos and artifact publish, the full
  relay client contract, room revocation by room id (FB27), a rolled
  mythological-creature name for unnamed rooms, and
  `HOTLINE_ASSISTANT_NAME` to override the display name.
- **Three frozen wire contracts.** `protocol/v2` (spec, frame schema, goldens),
  `protocol/core-v1` (envelope codec, box identity key, signed core client,
  fixtures + schemas), and `protocol/mailbox-v1` — each frozen behind fixture
  gates, with the generated artifacts checked against the code that implements
  them.
- **Durable mailbox delivery.** mailbox-v1 box dispatch, the client, and the
  live box↔relay seam: messages survive a device being offline, resync from a
  watermark, and reconcile orphans rather than silently dropping.
- **Code-based device linking.** `protocol/code-link-v1` implements a CPace OTP
  crypto core against the IETF draft's test vectors; `hotline relay new-link
  --code` is the box side.
- **Multi-device coexistence.** Additive minting, a room manager, per-device
  push, a mailbox provisioned on every attach, and a box-brokered read-state
  watermark so a message read on the phone is read on the desktop.
- **Agent-native UX v1 — elements.** Typed inline UI (cards, progress,
  decisions, checklists) over the wire, an `agent_state` frame, the `job` tool,
  and the `/el` bridge; box-authored resolution edits keep settled element state
  persisted and synced (FB92).
- **Automatic job cards (FB13).** Subagent work opens, updates, and closes a
  live card in the app — box core plus the Claude Code adapter and the pi
  harness. Cards persist and recover across restarts (FB71), and a box-side
  watcher auto-nudges and auto-closes orphaned cards (FB84).
- **Mission Control.** A durable thread store with an index, handoff, and
  render path; the `mission` MCP tool with index injection, a `hotline mission`
  CLI twin, per-harness seeding and mounting, Claude Code `SessionStart` /
  `PreCompact` hooks installed on `init`, and a pre-compact handoff loop with a
  soft context cap for pi.
- **Pi as a supervised harness, and bring-your-own-harness as the posture.**
  The `@1broseidon/hotline-pi` extension, a hardened subagent tool co-loaded
  with the channel, a four-agent starter pack (architect / implementer /
  researcher / scout), the `hotline-setup` skill, an operator model knob, and
  cross-language goldens generated from the real binary with a CI drift gate.
- **The claude-sdk harness.** A Claude Agent SDK managed harness — child bridge,
  proxy, session loop — first-class in Go, with a per-box config surface
  (`HOTLINE_SDK_MODEL` / `_EFFORT` / `_MAX_TURNS`), a model catalog reported as
  `harness_catalog`, FB13 job cards, and the Mission Control handoff loop.
- **Live model and effort changes, no restart.** A `set_sdk_config` control and
  `sdk_config_result` frame (protocol/v2 amendment), `sdk_apply` stdio
  notifications, `setModel` on the live session, effort hot-apply, and a
  two-tier model guard that applies a catalog-absent but valid id as unverified
  rather than refusing. The app's model row is the box's own advertised list.
  Pi joins the same hot-apply path with bidirectional TUI sync.
- **`@1broseidon/hotline-harness-core`.** The plumbing pi and claude-sdk share —
  child, queue, jsonrpc, session, auth, inbound, log — extracted into one
  package.
- **The fleet: box-to-box A2A (L1–L5).** An agent registry with a CLI and
  serve-side fleet rooms (L1), the agent path with tools, refusals and acks
  (L2), a dialer with a device-leg client and duplex state machine (L3),
  observability — a `fleet_state` transient, live `fleet ls`, and journal
  rotation (L4), and a box-attested capabilities manifest (L5). Delivery is
  durable and exactly-once via a journal WAL.
- **`hotline tail`** — a live harness event viewer.
- **Read-only MCP introspection tools** `list_schedules` and `list_loops`.
- **`curl -fsSL https://hotline.dev/install.sh | sh` — the install door.**
  A POSIX `sh` installer (`site/install.sh`, served at `hotline.dev/install.sh`)
  that resolves a release, downloads the matching goreleaser archive, verifies its
  SHA-256 against `checksums.txt`, verifies the cosign signature on that
  checksums file, and only then moves one binary into `~/.local/bin`. No sudo,
  no package manager, no lifecycle scripts, nothing written until every check
  passes. `HOTLINE_VERSION` pins a release, `HOTLINE_INSTALL_DIR` retargets, and
  a re-run is an atomic in-place upgrade that works even while an old hotline is
  running. An unsupported platform refuses with a source-build fallback; a
  missing `cosign` degrades with a loud warning rather than skipping quietly.
- **Signed releases.** goreleaser now signs `checksums.txt` with cosign in
  keyless mode, publishing `checksums.txt.sig` and `checksums.txt.pem`.
  Verifying that one signature transitively covers every archive in the release.
  There is no private key: the signer identity is a tagged run of this repo's
  `release.yml`, attested by GitHub's OIDC issuer and logged in Sigstore's
  public transparency log.
- **Fleet: orchestrator authority (F1).** A box can now be told, by its operator and only by its operator, that one fleet peer directs its work. `hotline fleet grant <peer> [--ttl <dur>]` records a grant on that edge's registry entry, bound to the peer's *pinned box-key fingerprint* (not the renameable alias); `hotline fleet revoke <peer>` drops it, and `fleet ls` shows `authority=granted[,expires_in=…]`. On a granted edge an inbound `task` (= assign), `cancel`, or `status_req` frame is framed to the agent as an **orchestrator directive** — act on it as work, or refuse with a reason — instead of the standing "untrusted peer data" marker; every other kind (`brief`/`result`/`ack`/`refuse`/`ping`), every other edge, and every frame after the grant expires or is revoked keeps that marker verbatim. The directive preamble still forbids approving pairings/access, changing permissions or capabilities, restarting, and anything destructive, on the peer's say-so. The grant is unreachable from the wire: no frame, tool call, or message body can create, extend, or refresh it (fleet_msg is rebuilt from validated fields, so a smuggled `authority` object is discarded), it is non-transitive (a grant on one edge confers nothing on another, even for the same peer key), one-way (what the box *sends* is unchanged, so a granted worker cannot steer its hub), and it stops applying the moment the peer's key pin changes. Revocation takes effect on the next inbound frame — the framing switch re-reads the registry rather than trusting the live session's captured edge. Directives received, and every grant/revoke, are journaled to `fleet.log` alongside the edge's existing durable frame journal.
- **Fleet kinds `cancel`, `status_req`, `refuse`.** The closed `fleet_msg.kind` enum grows the orchestrator vocabulary: `cancel` (wind the work down, send a final result) and `status_req` (report state) join `task` as the down set, and `refuse` makes "outside my charter" a first-class answer up. There is deliberately **no `restart` kind** — restarts are operator-only, so a compromised orchestrator cannot even phrase the order. Both boxes need this binary before the new kinds are usable; a pre-F1 peer protocol-errors on an unknown kind, and `brief|task|result|ack|ping` are unchanged.
- **claude-sdk harness 0.2.0 — reply enforcement + delivery re-arm (M1.1).** Closes a proven second-order leak observed on a live box: an operator turn's buffered text was forwarded by the fallback lane, then the SDK ran a *continuation* turn for the same inbound (a second `result`, no fresh user echo) that ended in new trailing text with no `mcp__hotline__reply` call — and slipped silently, because delivery was armed once per envelope. Now a conversation `(source, chat_id)` stays *awaiting-delivery* across turns until a SUCCESSFUL reply lands for it, so every continuation turn that produces operator-facing text is protected; the lane dedups per conversation so the same text is never sent twice, never attributes a continuation to an internal/schedule/notify/fleet turn, and forwards nothing (logging `ambiguous-continuation`) rather than cross-talk when it cannot tell which of several awaiting conversations a turn belongs to. A new `Stop` hook (`HOTLINE_SDK_ENFORCE=stop-hook|off`, default `stop-hook`, independent of `HOTLINE_SDK_FALLBACK`) is the locked door in front of that net: while the ending turn still owes the operator a reply it blocks the stop and tells the model to call `mcp__hotline__reply` now — the same "unanswered" predicate the lane uses — capped at two blocks per turn, after which the turn ends and the fallback lane delivers. Every settle now logs one line per outcome (`fired` / `reply-satisfied` / `miss why=…` / `blocked-by-hook`), so a silent miss can no longer happen without a trail.
- **claude-sdk harness 0.2.0 — auth containment (M2).** A dead Claude credential no longer respawns the box forever in silence. The harness classifies auth failures (`auth_status`/assistant error/first-turn throw), and on the third consecutive one it notifies the last operator once through the still-healthy channel child, writes an `auth.fatal` marker, and exits with code 5. The supervisor pins its backoff at Max (a 10-minute cold loop, no give-up state) while the marker exists — cleared on the harness's next successful init, which restores normal backoff — and records "auth failure — credentials need the operator" in the status line.
- **claude-sdk harness 0.2.0 — delivery guarantee + instruction profile (M1).** A ground-up rebuild of the Claude Agent SDK harness lives clean beside the 0.1 one in `harness/claude-sdk-v2` (child identity stays `claude-sdk`; point `HOTLINE_CLAUDE_SDK_ENTRY` at its `dist/index.js` to run it). A turn ledger uuid-stamps every inbound envelope and tracks which SDK turn consumes it, so an operator turn that ends without a `mcp__hotline__reply` call has its buffered text forwarded by a fallback lane (`HOTLINE_SDK_FALLBACK`, default on) instead of silently staying in the box — schedule/notify/fleet turns are excluded. The harness gets its own uncapped instruction profile (reply contract → preset neutralizer → Task-tool delegation doctrine, no pi Agent-tool doctrine) plus a per-turn reply-contract reminder, retiring the `run_claudesdk.go` instruction debt. A `fallback_count` lane counter rides `harness_info`.

### Changed
- **Homebrew ships as a Cask, not a Formula.** Formulas are meant to build from
  source; hotline ships an already-built binary, which is what Casks are for.
  goreleaser deprecated `brews:` in v2.10 and removes it in v3. The tap file
  moves from `Formula/hotline.rb` to `Casks/hotline.rb`;
  `brew install 1broseidon/tap/hotline` is unchanged. The cask strips the macOS
  quarantine attribute on install, since the binaries are not notarized.
- **`hotline up` is the unified operator launcher.** It now stays in the foreground by default. The headless Pi and OpenCode harnesses can detach with `--background` / `-d`; Claude rejects detached mode because its development-channel consent requires an attended terminal. The old `--foreground` flag is a deprecated no-op with a notice, `hotline start` is a deprecated alias for attached `hotline up`, and `hotline run` is the raw MCP-server/poller/ticker plumbing invoked by `.mcp.json`.
- **Detached Claude prompt simulation was removed.** The `--ack-dev-channels` flag, persisted acknowledgement, folder-trust bootstrap, ANSI/VT dialog detector, and PTY write-back seam are gone. Hotline never auto-answers Claude terminal prompts. For a long-lived Claude box, run `tmux new -s hotline -- hotline up`, detach with Ctrl-b d, and reattach with `tmux attach -t hotline`.
- **Box state is isolated by provider identity.** The unnamed box keeps its existing state-root paths, while a named box stores Mission Control, schedules, loops, notify state, jobs, and supervisor state under `bots/<name>/`. Runtime and provider-consumer liveness now use lifetime filesystem locks: a second process that overlaps a live box or provider state refuses with an explicit ownership error instead of sharing it. `bot.pid` remains diagnostic and no longer decides liveness by PID probing.
- **The Claude TUI attaches to the operator terminal on `hotline up`.** The
  supervisor bridges the PTY across terminal-ownership edges and routes its own
  diagnostics off the terminal while the TUI owns it.
- **The instruction budget is 4096 bytes, and the voice comes first.** The
  channel register tail carries the voice charter ahead of the Mission Control
  block, fits Claude Code's real 2KB cap, and warns loudly at launch when it has
  to truncate.
- **Box-owned assistant identity (FB21).** The box owns the assistant name,
  syncs it to devices, and lets a device rename it; a rename refreshes the push
  title (relay re-register plus `botName`).

### Fixed
- **Fleet edge death is recoverable, and one-sided death is audible (F2).** A dial edge that failed 12 consecutive handshakes used to be tombstoned `unreachable` with its creds zeroed — terminal, with no revive verb in the CLI, so ~8 minutes of relay churn (54 × `code=1006 unexpected EOF` in 24h) permanently retired a known-good edge and cost an operator re-pair. It now drops to a **cold-retry tier** instead: the edge is flagged `unreachable` (a recoverable state, *not* a tombstone — creds retained, edge still live, still accepting inbound), keeps being dialed every ~5 minutes, and **revives itself on the next successful handshake**. `removed` (operator) and `revoked` (peer) stay terminal; only network-caused death is reversible. The handshake counter also got honest: a socket that dies *before* `welcome_fleet` no longer resets the streak as if the edge had connected. The other half of that failure — the serving side happily queueing frames for a peer that had written the edge off (`pending=2`, forever, silently) — now raises an alarm: a per-minute liveness sweep logs `STALE PENDING` to `fleet.log` (throttled per edge) whenever an edge holds queued outbound with no session and no contact for 10 minutes, and both `unreachable` and `stale_pending` surface in `fleet ls`, `fleet ls --json`, the fleet MCP tool, and the operator `fleet_state` snapshot. The sweep only ever reports — it never tombstones, drops frames, or tears an edge down.
- **Project-scoped operator verbs stay on the current project's box.** `up`, `start`, `down`, `status`, `tail`, and the other box-state commands now adopt the nearest parent `.mcp.json` hotline identity before resolving paths. `down` can no longer stop the base supervisor from a named-box project, and implicit `status` follows the configured Telegram instance instead of reading base state. Malformed or ambiguous hotline entries and provider/bot conflicts refuse rather than guessing.
- **Every channel-serving runtime now ticks its script loops.** The loop runner moved from the `hotline up` supervisor into the box-owning `hotline run` process, so plain `run` sessions tick and supervised Claude, OpenCode, and Pi sessions receive the ticker through their run child. The box's exclusive `active.lock` prevents a second runtime process from double-ticking; the in-process guard remains a backstop. Shutdown drains in-flight ticks before releasing the box lease, and each tick still persists its start watermark before executing.
- **`go test` can no longer bounce a live box.** The supervisor directory is
  resolved from config, never from ambient environment, so a test run that
  inherits `HOTLINE_SUPERVISOR_DIR` cannot file a restart request against the
  operator's running session.
- **`hotline down` / `status` resolve the box the current directory owns** and
  refuse to SIGTERM the wrong one.
- **Soak fixes on the long-running box.** The harness is recycled when the relay
  rotates its room, journaled-but-unanswered inbound is replayed on harness
  restart (keyed on the delivered-marker high-water rather than the last
  outbound), and connector `/c` lifecycle and close codes are persisted for
  post-mortem.
- **Relay and delivery hardening.** Reconnect races survive, case-variant
  `relay_presence` spoofs are rejected, delivery truthfulness means sink
  acceptance is the receipt, ping validation and a gap bad-frame limit are
  enforced, push targets are set atomically, and a device more than
  `maxDeviceHoles` behind escapes the deaf-mailbox trap instead of wedging.
- **Job cards resolve by the card's message id**, log their start as well as
  their done, and cookie→batch resolution covers batch-less done/update (FB13).
- **The `.env` writer is serialized and replaces atomically**, `set_sdk_config`
  is idempotent under replay, applies serialize, hot applies are fenced to the
  live session, effort replaces rather than accumulates, and a cleared
  model/effort is reported rather than merely absent.

## [0.10.0] - 2026-07-09

### Added
- **Alternate Anthropic-API provider for the Claude Code harness.** `hotline
  setup` gains `--anthropic-base-url`, `--anthropic-token`
  (→`ANTHROPIC_AUTH_TOKEN`), `--anthropic-api-key`, and `--anthropic-model`;
  `hotline start` and `hotline up` inject the allowlisted `ANTHROPIC_*` keys
  (base URL, auth, model, the three `ANTHROPIC_DEFAULT_*_MODEL` role vars,
  custom headers, `API_TIMEOUT_MS`, and `ENABLE_TOOL_SEARCH`) from the shared
  `.env` into Claude Code, with the real environment winning per key. The
  opencode harness is unaffected. A non-`api.anthropic.com` base URL disables
  Claude Code's MCP tool search unless `ENABLE_TOOL_SEARCH=true` is set.
- **Script loops: `hotline loop`.** Operators can register local shell commands
  on intervals (`loop add <label> --every <dur> --cmd "<shell>"`) and have
  `hotline up` run them eagerly at supervisor start and then per-loop on their
  tick. Loops get a private durable state dir via `HOTLINE_LOOP_STATE_DIR`,
  skip overlapping runs, enforce `--timeout` with a process-group kill, write
  size-rotated logs under `<state>/loops/`, and record advisory last-run status
  in `loops.json`. With `--notify-llm`, non-empty stdout is routed through the
  existing notify gate by source label (auto-minted when omitted); empty stdout
  wakes nothing. Without it, the script owns escalation and hotline only logs.
- Added loop approval posture: normal-mode loop creation is pending until
  `hotline loop approve <label>` (or removed with `loop deny <label>`), while
  direct CLI `loop add -y` pre-approves. Yolo sessions create loops approved and
  enqueue an operator heads-up. The runner skips unapproved loops.
- Added MCP tools `setup_loop` and `setup_notify`. They reuse the same loop and
  notify registry surfaces as the CLI; `setup_loop` cannot self-approve, and
  `setup_notify` returns the source label without exposing the minted key.
  `loop run <label> --once` provides the foreground/cron escape hatch.

### Changed
- **Inbound coalescing holds a short grace window.** A complete-looking message
  (terminal punctuation or long) now waits ~500ms before flushing instead of
  firing immediately, so rapid complete-sentence bursts coalesce into a single
  turn. Fragments keep the full window; the message-count and max-wait caps are
  unchanged. Applies to the Telegram, Discord, and Signal adapters.
- **email-sentry plugin.** Ported the live triage tuning, de-personalized the
  templates, taught the init flow to check for the `gogcli` dependency, and
  wired `setup_loop` into the watcher registration.

## [0.9.0] - 2026-07-08

### Added
- **Event-driven notifies: `hotline notify` + `hotline source`.** A third
  ingress leg beside messages and schedules: local scripts and daemons (backup
  jobs, the email sentry, CI, watchers) hand hotline an event with
  `... | hotline notify --source <key>`, and the agent — not the script —
  decides whether it is worth buzzing you. Each source is a capability key
  minted by `hotline source add <label>` (a UUIDv4 bearer credential, compared
  in constant time; `source list` re-shows it, `source revoke` kills it
  instantly since every call reads the registry fresh). The CLI runs the whole
  gate inside a flock'd critical section — key lookup, level clamp to the
  source's cap, payload sanitization (control-char strip and `</channel`
  neutralization so a script-authored line can't forge the envelope), 10-minute
  dedup coalescing, a per-source token-bucket rate limit (burst 5, refill
  1/5min, overridable), and quiet hours (`"HH:MM-HH:MM"`, only urgent bypasses;
  held events release together as one digest at window end). stdin is
  first-class (`tail -1 backup.log | hotline notify --source $KEY --level low`);
  a positional message wins over the pipe. Exit codes are the script contract:
  0 accepted, 3 queued, 4 rejected/suppressed, 2 usage, 1 internal. Accepted
  events land in `notify/spool.json` and inject on the daemon's next tick as
  `kind="notify"` turns — durable across restarts (the eager startup scan is the
  catch-up), framed by a compiled-in preamble as an untrusted machine report,
  explicitly not operator instructions, with silence stated as a valid outcome.
  Wired into both the Claude Code and OpenCode dispatch loops exactly like
  schedules; `hotline notify list` and `hotline source list` are the operator
  views. v1 is local-only: no network listener, no port — writing a file
  through a CLI on a box you are already on is the whole transport.
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
