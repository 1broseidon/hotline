# hotline

A messaging channel for Claude Code and OpenCode. hotline is an MCP server that relays your agent session to Telegram, so you talk to it the way you text a friend: short bubbles, reactions, tappable buttons, photos.

It drives two harnesses: Claude Code over its experimental two-way channel protocol (`claude/channel`), and OpenCode over a separate HTTP+SSE adapter. The texting experience is the same either way. More harnesses work if they adopt the protocol or when hotline adds support.

## The texting experience

Claude replies in bubbles, one thought each, delivered with a typing indicator and a natural pause between them. It reacts with an emoji when a 👍 says it, edits its own messages to show progress, and offers inline buttons when it wants you to pick one thing.

A real exchange looks like this:

> **you:** the build's failing again 😤
>
> **claude:** ugh again?
> **claude:** lemme look
> **claude:** yeah it's that flaky integration test, not your code
> **claude:** want me to just retry it?
> `[ retry 🔁 ]` `[ dig deeper ]`

You tap a button, the choice lands back in the session as if you had typed it, and the keyboard clears so it can't be answered twice.

Inbound works the same way. You text in bursts, so hotline buffers consecutive messages per chat (1.2s window, 8s max wait) and delivers them as one coherent turn instead of interrupting Claude three times. Photos are downloaded eagerly and handed to Claude as a local path; documents, voice notes, video, and stickers are surfaced as metadata and fetched on demand.

## Install

```sh
brew install 1broseidon/tap/hotline
```

Binaries ship with each tagged release. Or build from source:

```sh
go install github.com/1broseidon/hotline/cmd/hotline@latest   # -> $(go env GOPATH)/bin/hotline
```

Requires Go 1.26+ for the source build.

## Quickstart

1. Get a bot token from [@BotFather](https://t.me/BotFather), then save it (once, machine-wide):

   ```sh
   hotline setup --telegram-token 123456789:AA…
   ```

   Run `hotline setup` with no flags to be prompted, `hotline setup --show` to see what's configured.

2. Register the channel in the project you want to text with:

   ```sh
   cd your-project
   hotline init
   ```

   This installs the hotline Claude Code plugin and enables it for the project (`.claude/settings.json`). Add `--providers telegram,signal` for extra transports, `--voice` for a starter `HOTLINE.md`, `--mcp-json` to register a raw `.mcp.json` server instead.

3. Launch Claude Code with the channel loaded:

   ```sh
   hotline start              # extra claude flags go after --, e.g. hotline start -- --continue
   hotline start --yolo       # adds --dangerously-skip-permissions; the permission relay never fires
   ```

   Or make it always-on — supervised, detached from the terminal, restarted on crash:

   ```sh
   hotline up                 # same flags as start; stop with hotline down
   ```

4. DM your bot. The first message from an unknown sender returns a 6-hex pairing code. Approve it from your terminal:

   ```sh
   hotline pair <code>
   ```

That's it. Your session is now a Telegram chat.

Starting fresh? [templates/mission-control](templates/mission-control/) is our take on what makes a good texting agent: a filing system the agent keeps on disk, an operating playbook, a starter voice. Copy the folder, or install it as a plugin and run `/mission-control:init` in your project:

```sh
claude plugin marketplace add 1broseidon/hotline
claude plugin install mission-control@hotline
```

The same marketplace also has `email-sentry`: a Gmail watcher that buzzes your hotline channel only for mail that matters (`claude plugin install email-sentry@hotline`, then `/email-sentry:init`).

<details>
<summary>By hand</summary>

The three commands wrap these steps:

```sh
# setup: token in the channel's .env (a real TELEGRAM_BOT_TOKEN env var wins over the file)
mkdir -p ~/.config/hotline
printf 'TELEGRAM_BOT_TOKEN=123456789:AA…\n' > ~/.config/hotline/.env
chmod 600 ~/.config/hotline/.env
```

```sh
# init: install the plugin, enable it for the project
claude plugin marketplace add 1broseidon/hotline
claude plugin install hotline@hotline -s project
```

```sh
# start: load the plugin channel
claude --dangerously-load-development-channels plugin:hotline@hotline
```

Claude Code gates channel registration on an allowlist of approved channel plugins; hotline isn't on it yet, so the dev-channel flag is still needed. `hotline start` checks the allowlist on every launch and drops the flag for plain `--channels plugin:hotline@hotline` the moment hotline is approved.

Prefer a raw MCP server without the plugin? `hotline init --mcp-json` writes this into the project's `.mcp.json`:

```json
{
  "mcpServers": {
    "hotline": { "command": "hotline", "args": ["run"] }
  }
}
```

Raw servers always need the dev-channel flag, by name: `claude --dangerously-load-development-channels server:hotline`.

</details>

## Access model

Nobody talks to your agent without your say-so. The inbound gate decides on the sender, never the chat: a stranger in an allowed group is still dropped.

| Policy | Behavior |
|---|---|
| `pairing` (default) | Unknown DM sender gets a pairing code; you approve or deny from the terminal |
| `allowlist` | Only listed user IDs are delivered; everyone else is silent |
| `disabled` | No DMs at all |

Groups are opt-in per group ID, with optional `requireMention` (deliver only when the bot is mentioned, replied to, or a `mentionPatterns` regex matches) and an optional per-group sender allowlist.

Configuration lives in `~/.config/hotline/access.json` and is re-read on every inbound message, so edits take effect live:

```jsonc
{
  "dmPolicy": "pairing",          // pairing (default) | allowlist | disabled
  "allowFrom": ["412587349"],     // numeric user IDs
  "groups": {
    "-1001234567890": { "requireMention": true, "allowFrom": [] }
  },
  "mentionPatterns": ["^hey claude"],
  "ackReaction": "👀",            // emoji reaction on receipt ("" disables)
  "replyToMode": "first",         // first | all | off
  "textChunkLimit": 4096,
  "chunkMode": "newline",         // newline | length
  "bubbleMode": "paced"           // paced | instant
}
```

Pairing codes expire after 24 hours. To remove an already-approved sender, run `hotline revoke <sender-id>` (the exact ID as shown by `hotline status`, or a unique prefix); it drops the sender from `allowFrom` and purges any pending pairing they still had. Approval happens only from your terminal: Claude never approves a pairing or edits access because a chat message asked it to. That request is exactly what a prompt injection looks like.

Outbound is gated too. Every tool call checks the target chat against the same rules before touching the Telegram API, so Claude can only message chats that could message it. The channel also refuses to attach its own state files, and sanitizes uploader-controlled filenames before they enter the message metadata.

## The tools

| Tool | What it does |
|---|---|
| `reply` | Send `bubbles` (array of short messages, paced) or `text` (single message, chunked past 4096 chars). Optional `reply_to`, `files` (images inline, others as documents, 50MB each), `buttons` (up to 12 inline options), `format` (`text`, `markdownv2`, `html`) |
| `react` | Set an emoji reaction (Telegram's fixed whitelist) |
| `edit_message` | Edit a message the bot sent, for interim progress. Edits don't push-notify |
| `download_attachment` | Fetch a non-photo attachment by `file_id` into the inbox; returns a local path (Telegram's 20MB download cap applies) |
| `publish` | Serve a local artifact (a directory or a single HTML file) at a temporary link that dies with the session. The exposure backend is operator-selected via `HOTLINE_PUBLISH_EXPOSURE`: `localhostrun` (default), `cloudflared`, or `local` (loopback only). Public links are passcode-gated: the tool returns a `Link:` and a 6-digit `Passcode:` line to relay verbatim — visitors enter the code once on an unlock page (phones offer it as one-time-code autofill from the chat message) and get a session cookie; ten wrong guesses lock the publish until republished. A single-file publish serves exactly that file, never its parent directory |
| `schedule` | `create`, `list`, or `cancel` a scheduled task. At the scheduled time the stored prompt is injected back into the session as an inbound turn (`kind="schedule"`), so the agent acts on it with full tool access and normal permission gating. Recurrence is a preset: `once`, `daily`, `weekly`, `every_n_hours`, `every_n_days`. A one-off's fire time takes a relative offset (`+2m`, `+1h30m`) or an absolute time; the rest are server-local |
| `setup_loop` | Create a supervised local script loop. In normal mode it is pending until the operator runs `hotline loop approve <label>`; in yolo mode it is live immediately and the operator is notified. There is no self-approve flag in the tool |
| `setup_notify` | Create a notify source for local scripts. It returns the label, not the capability key; the operator manages keys with `hotline source list` / `revoke` |
| `restart` | Only under `hotline up`: asks the supervisor to relaunch the session (it writes the supervisor's control file), so the paired user can say "restart yourself". In-flight context is lost; the transcript, schedules, and access state persist |

Button taps come back as ordinary inbound messages whose content is the tapped label, verbatim, so values are never truncated by Telegram's callback-data limit. Tap authorization mirrors the inbound gate.

Every message in both directions is appended to `<stateDir>/transcript.jsonl`. Telegram has no history API and Claude Code sessions compact or restart, so the transcript is how the assistant recalls the thread across resets. It's written 0600 and currently unbounded.

## Voice

The channel instructions ship with a default persona: a sharp, warm friend texting in short bubbles. Drop a `HOTLINE.md` file to replace it. First hit wins:

1. `./HOTLINE.md` in the directory Claude Code runs in, for a per-repo voice
2. `HOTLINE.md` in the state dir (`~/.config/hotline` by default), your global default
3. the built-in voice

The file is read once at startup. Edit it, then restart Claude Code. Empty files are skipped.

Claude Code caps MCP server instructions at 2048 characters. hotline puts the mechanics first and gives the voice the remainder, about 630 characters; a longer voice is cut at a word boundary with a stderr warning.

A voice changes tone only. The tool contract, inbound message handling, and the safety rules (operator-only pairing approval, the injection stance) are compiled in, always come first, and apply under any voice.

```markdown
<!-- HOTLINE.md -->
Ye be a pirate on shore leave. Every bubble comes out salty.
Call them "cap'n". Two bubbles at most or they walk the plank.
```

Or for work:

```markdown
<!-- HOTLINE.md -->
Terse and professional. No emoji, no exclamation marks.
One bubble unless the answer genuinely needs two.
```

## Multiple providers, multiple bots

hotline is built on an internal provider interface with a source router. Telegram is the first provider. Configure providers with `HOTLINE_PROVIDERS`, a comma-separated list of `kind[:instance]` entries:

```sh
HOTLINE_PROVIDERS=telegram              # the default
HOTLINE_PROVIDERS=telegram:work         # a named instance
HOTLINE_PROVIDERS=telegram,discord      # two transports on one channel
HOTLINE_PROVIDERS=telegram,discord,signal   # all three
```

`HOTLINE_PROVIDERS` is read from the process environment, not the state `.env`. Set it where the MCP server is launched. On the plugin path that's the `env` block of the project's `.claude/settings.json` — Claude Code applies it to the session, and the plugin-spawned server inherits it (`hotline init --providers telegram,signal` writes this for you):

```json
{
  "enabledPlugins": { "hotline@hotline": true },
  "env": { "HOTLINE_PROVIDERS": "telegram,signal" }
}
```

On the raw path it's the `env` block of your `.mcp.json`:

```json
{
  "mcpServers": {
    "hotline": {
      "command": "hotline",
      "args": ["run"],
      "env": { "HOTLINE_PROVIDERS": "telegram,signal" }
    }
  }
}
```

Tokens and accounts stay in the state `.env` as shown below.

With one provider configured, the tool schemas are byte-identical to the single-provider ones above. With several, each tool takes a required `source` argument matching the `source` attribute on inbound messages.

Named instances are how you run several sessions at once. One bot token allows exactly one Telegram poller, so each concurrent session gets its own bot. `--bot work` is shorthand for `HOTLINE_PROVIDERS=telegram:work`. Each named bot keeps isolated state under `<stateDir>/bots/<name>/` and reads its token from `TELEGRAM_BOT_TOKEN_<NAME>` in the shared `.env`:

```sh
# ~/.config/hotline/.env
TELEGRAM_BOT_TOKEN=111:AA…            # default bot
TELEGRAM_BOT_TOKEN_WORK=222:BB…       # telegram:work
```

`--bot` works on every subcommand (`hotline status --bot work`, `hotline pair <code> --bot work`).

When a future transport lacks a feature, the adapter degrades it, never the agent: on a transport without inline buttons, buttons render as numbered text options and the numbered choice routes back the same way. The tool contract stays the same everywhere.

## Discord

Discord runs as a second provider next to Telegram, or on its own. Same tools, same access model, native buttons.

Setup:

1. Create an application at https://discord.com/developers/applications, then add a Bot under it.
2. On the Bot page, enable the **Message Content Intent** under Privileged Gateway Intents. Without it the bot receives empty message bodies.
3. Copy the bot token into the shared `.env`:

```sh
# ~/.config/hotline/.env
DISCORD_BOT_TOKEN=your-bot-token
DISCORD_BOT_TOKEN_WORK=…              # discord:work, if you run named instances
```

4. Invite the bot with an OAuth2 URL using the `bot` scope. For DMs no extra permissions are needed. For guild channels grant Send Messages, Read Message History, Add Reactions, and Attach Files:

```
https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot&permissions=100416
```

5. Enable the provider and run:

```sh
HOTLINE_PROVIDERS=telegram,discord hotline
```

6. DM the bot. It replies with a pairing code; approve it from your terminal:

```sh
hotline pair <code> --provider discord
```

`--provider discord` points pair/deny/status at the Discord state (`<stateDir>/discord/`); named instances use `--provider discord:work` with state under `<stateDir>/discord/instances/work/`.

Notes on behavior:

- Buttons are native Discord message components. A tap comes back as an inbound message with the tapped label, and the buttons are cleared so a question can't be answered twice.
- The permission relay works exactly like Telegram's: allow/deny/more buttons DM'd to allowlisted users, `yes <code>` / `no <code>` text replies also accepted.
- Bubbles are paced with Discord's typing indicator. Messages split at Discord's 2000-char cap.
- Guild channels gate as groups keyed by channel ID: add the channel to `groups` in the Discord `access.json`, with `requireMention` to only wake the bot on @-mention.
- Inbound images download eagerly to the inbox; other attachments surface a CDN URL as `attachment_file_id` for `download_attachment` (Discord CDN hosts only, 50MB cap). Outbound files cap at 10MB, Discord's default bot upload limit.
- Messages from bots (including itself) are never relayed.

## Signal

Signal runs as a provider next to Telegram and Discord, or on its own. Same tools, same access model. There is no bot API: hotline talks to a locally running [signal-cli](https://github.com/AsamK/signal-cli) daemon, linked to your Signal account as a secondary device. signal-cli is a third-party client, not an official Signal product.

Setup:

1. Install signal-cli. On macOS: `brew install signal-cli`. On Linux, download a release from https://github.com/AsamK/signal-cli/releases (it is not in apt) and put `signal-cli` on your PATH. Java 17+ required.
2. Link it to your account as a secondary device. Run:

```sh
signal-cli link -n hotline
```

   It prints a `sgnl://linkdevice?...` URI. Render it as a QR code (`qrencode -t ansiutf8 'sgnl://...'`) and scan it from your phone under Settings → Linked Devices. Registration stays on your phone; hotline never touches it.

3. Run the daemon (keep it running: tmux, or a systemd user service with `ExecStart=signal-cli -a +15551234567 daemon --http 127.0.0.1:8080`):

```sh
signal-cli -a +15551234567 daemon --http 127.0.0.1:8080
```

4. Point hotline at it in the shared `.env`:

```sh
# ~/.config/hotline/.env
SIGNAL_ACCOUNT=+15551234567           # the linked account, E.164
SIGNAL_DAEMON_URL=http://127.0.0.1:8080   # optional, this is the default
SIGNAL_ACCOUNT_WORK=…                 # signal:work, if you run named instances
```

5. Enable the provider and run:

```sh
HOTLINE_PROVIDERS=telegram,discord,signal hotline
```

6. Message the account from another Signal account. Unknown senders get a pairing code; approve it from your terminal:

```sh
hotline pair <code> --provider signal
```

`--provider signal` points pair/deny/status at the Signal state (`<stateDir>/signal/`); named instances use `--provider signal:work` with state under `<stateDir>/signal/instances/work/`.

Notes on behavior:

- Senders are identified by phone number (E.164); the allowlist holds numbers. DM chat_ids are the peer's number, group chat_ids are `group:<id>`; add those to `groups` in the Signal `access.json`.
- Signal has no inline buttons. Buttons render as numbered text options, and replying with the number sends the chosen label back to Claude. Same round trip, typed instead of tapped.
- The permission relay is text-only: prompts arrive as a message, answer with `yes <code>` or `no <code>`.
- Reactions and edits are native (signal-cli `sendReaction` and `send --edit-timestamp`). Bubbles are paced with Signal's typing indicator. Messages split at 2000 chars, where Signal clients switch to long-text attachments.
- Message ids are Signal timestamps (Signal's message identity); inbound ids carry the author as `<timestamp>:<number>` so reactions target correctly.
- Inbound images are fetched from the daemon into the inbox; other attachments surface an id for `download_attachment` (50MB cap both ways).
- The daemon's HTTP endpoint has no authentication; keep it on 127.0.0.1.

## Permission relay

When a token is configured, hotline declares the `claude/channel/permission` capability. Claude Code's permission prompts are relayed to allowlisted DMs (never groups) with **See more / Allow / Deny** buttons. You can also answer by text with `yes <code>` or `no <code>`; those replies are intercepted and converted to a verdict instead of being relayed as chat.

## CLI

```
hotline setup        save credentials to the shared .env (run once)
hotline init         install the hotline plugin and enable it for this repo
hotline start        launch Claude Code with the channel loaded
hotline up           launch Claude Code supervised (always-on; see below)
hotline down         stop the supervised session
hotline [run]        start the MCP server + Telegram poller (default)
hotline pair <code>  approve a pending pairing code
hotline deny <code>  reject a pending pairing code
hotline revoke <id>  remove an approved sender from the allowlist
hotline status       print state-dir / token / access summary
hotline schedule     operator view of scheduled tasks
                     (list | remove <id> | pause <id> | resume <id>)
hotline loop         manage script loops
                     (add <label> --every <dur> --cmd "<shell>"
                     [--notify-llm] [--source <notify-label>] [--level L]
                     [--timeout <dur>] | list | remove | pause | resume |
                     logs | run)
hotline notify       enqueue a machine event from a local script for the agent
                     to triage (--source <key> [--level urgent|normal|low]
                     ["message"|stdin]; exit 0 accepted, 3 queued, 4 rejected).
                     "hotline notify list" shows the spool
hotline source       manage notify capability keys
                     (source add <label> [--cap L] [--burst N] [--refill-mins M]
                     [--chat-id ID] | source list | source revoke <label>)
```

`pair`, `deny`, `revoke`, and `status` take `--provider kind[:instance]` to select which provider's state they operate on (default: telegram).

Schedules are created from chat via the `schedule` tool; the `hotline schedule` CLI is the operator's view over them. `list` shows every schedule with its next fire time; `remove` deletes one by id (or unique prefix); `pause`/`resume` are the operator kill-switch (resuming a recurring schedule recomputes its next fire from now, so a long pause never triggers a stale catch-up burst). Schedules live in `schedules.json` at the state root and are re-read live by a running daemon, so CLI edits take effect without a restart.

Notify is the third ingress leg, beside messages and schedules: event-driven, from local scripts and daemons (backup jobs, an email watcher, CI, monitors) rather than a human or a timer. `hotline source add <label>` mints a UUIDv4 capability key (a bearer credential — every human-facing surface shows the label, never the key) with an optional level cap, rate-limit override, and default chat id; `setup_notify` can create the same source from chat but does not return the key. `revoke` kills a source instantly, since every `notify` call reads the registry fresh. `hotline notify --source <key>` runs the full gate inline before durably enqueuing: level clamp to the source's cap, payload sanitization (control-character stripping, envelope-close neutralization), a 10-minute dedup window, a per-source token-bucket rate limit (burst 5, refill 1 per 5 minutes by default), and quiet hours (`"HH:MM-HH:MM"`, only `urgent` bypasses; events held during the window release together as one digest). Exit codes are the script-facing contract: `0` accepted, `3` queued, `4` rejected or rate-limited, `2` usage error, `1` internal error. stdin is first-class, so `tail -1 backup.log | hotline notify --source $KEY --level low` works. Accepted events inject on the dispatcher's next tick as `kind="notify"` turns, framed by a compiled-in preamble as an untrusted machine report — never operator instructions, and silence is a valid, correct outcome. `hotline notify list` and `hotline source list` are the operator's views into the spool and the key registry; notify's state lives in `notify/spool.json` and `notify/sources.json` at the state root, guarded by the same flock/atomic-write pattern as `schedules.json`.

Loops are the scheduler applied to local scripts. `hotline loop add <label> --every <dur> --cmd "<shell>"` registers a command; in normal mode it starts pending and the runner skips it until `hotline loop approve <label>`, while `hotline loop deny <label>` removes it. Direct operator CLI setup can pass `-y` / `--approve` to create it approved immediately. In yolo mode, loops created by CLI or `setup_loop` are approved immediately and hotline enqueues a non-blocking operator heads-up. Once approved, `hotline up` runs the loop eagerly at supervisor startup and then on its interval. The runner skips overlapping ticks, enforces `--timeout` with a process-group kill, writes a size-rotated per-loop log under `<state>/loops/<label>.log`, and records advisory last-run status in `loops.json`. Each run gets `HOTLINE_LOOP_STATE_DIR=<state>/loops/<label>/state`, `HOTLINE_LOOP_LABEL`, and, when `--source <notify-label>` is set, `HOTLINE_NOTIFY_SOURCE=<key>` in its environment. With `--notify-llm`, non-empty stdout is routed through the existing notify gate using the stored source label; empty stdout enqueues nothing. Without `--notify-llm`, hotline only logs the run and the script owns escalation, usually by calling `hotline notify --source "$HOTLINE_NOTIFY_SOURCE"` itself. If `--notify-llm` is set without `--source`, loop setup creates a notify source named after the loop and stores only that label in `loops.json`. `hotline loop run <label> --once` executes one approved registered tick in the foreground for cron/testing; `list`, `pause`, `resume`, `approve`, `deny`, `remove`, and `logs -n N` are the operator controls.

## Always-on: hotline up

`hotline start` lives and dies with your terminal. `hotline up` is its always-on sibling: a small supervisor that owns the harness process — Claude Code by default, `opencode serve` under `HOTLINE_HARNESS=opencode` — and keeps it alive until you say otherwise.

```sh
hotline up            # detached: supervisor + harness survive this terminal
hotline up --yolo     # claude path: same flags as start, passthrough after -- included
hotline down          # graceful stop (harness first, then supervisor)
hotline status        # gains a supervisor block: phase, pids, restarts, logs
```

What it does:

- **Owns the harness.** On the claude path, Claude Code needs a real terminal (with a non-tty stdin it drops into print mode and exits), so the supervisor allocates a pty and runs claude on it, in its own session and process group. On the opencode path, `opencode serve` is a headless daemon: the supervisor runs it on plain pipes — same session/process-group discipline, no pty — binding the port and hostname from `OPENCODE_SERVER_URL` (default `http://127.0.0.1:4096`), the same source the hotline MCP child reads, so daemon and client always agree. Linux and macOS.
- **Restarts on any exit** with exponential backoff: 2s doubling to a 10-minute ceiling, reset after 5 minutes of healthy uptime. It never gives up — a persistently failing harness costs at most six attempts an hour, each with a logged breadcrumb. Together with the scheduler's catch-up scan, a 3am crash no longer eats your 9am schedule: the session comes back and the overdue fire happens exactly once.
- **Restart from chat.** Under `hotline up` the session gains a `restart` MCP tool, so the paired user can say "restart yourself". The tool only writes the supervisor's control file — what runs (argv, env, cwd) was fixed by you at `up` time, and the reason string is only ever logged — so a prompt-injected restart is at worst a bounced session, a smaller blast radius than tools the channel already has. `kill -HUP <supervisor pid>` does the same from the machine.
- **Logs in the state dir.** `supervisor/supervisor.log` holds the supervisor's event lines (starts, exits, backoff, restart reasons); `supervisor/harness.log` captures claude's pty output (raw, ANSI escapes and all), size-rotated at 5MB with one older generation kept.

`--foreground` skips the detach and runs the supervisor in your terminal — that's the shape a tmux pane or a systemd unit wants (`ExecStart=hotline up --foreground`). A flock under `supervisor/` guarantees one supervisor per state root; liveness is the held lock, not a pid file, so a stale `state.json` can never wedge `up` or `down`.

Restarted sessions start fresh by default: cross-restart memory is the transcript and `schedules.json`, by design. On the claude path, if you want claude itself to resume its conversation, `hotline up -- --continue` re-applies on every respawn (careful: `--continue` with no prior session, or a corrupted one, can crash-loop into the backoff ceiling). On the opencode path, args after `--` go to `opencode serve` verbatim, and session state survives a bounce on its own: opencode persists sessions on disk, and hotline's session pinning / most-recent selection re-attaches after the restart.

Per-harness honesty notes:

- **Claude path:** unattended restarts are only truly unattended once hotline is on Claude's approved channels allowlist (`--channels`). Until then `hotline start`/`up` fall back to `--dangerously-load-development-channels`, whose confirmation prompt is per-launch by design — so each supervised respawn parks on that prompt until someone attaches to the pty (e.g. via the harness log you can see it waiting) and confirms. The supervisor machinery is correct today; the allowlist switch (automatic in `channelArgs`) is what makes it hands-off.
- **OpenCode path:** `--yolo` errors instead of being silently ignored — it maps to a claude flag with no opencode equivalent; opencode's permission policy lives in `opencode.json`'s `permission` block. And the supervisor watches `opencode serve` only: if the hotline MCP child opencode spawns dies while serve stays up, the supervisor doesn't see it (a serve bounce — `restart` tool, SIGHUP, `down`/`up` — recovers it).

## State and environment

State lives in `${XDG_CONFIG_HOME:-~/.config}/hotline`. On first run, state found at the old default `~/.config/hotline` is copied over automatically; the old directory is left in place so sessions still running an older binary keep working. hotline was formerly `tele-go`; `TELE_GO_*` variables keep working as fallbacks for one release.

| Variable | Purpose |
|---|---|
| `TELEGRAM_BOT_TOKEN` | Bot token (real env wins over `.env`); `TELEGRAM_BOT_TOKEN_<NAME>` per named instance |
| `HOTLINE_PROVIDERS` | Provider list, `kind[:instance]` comma-separated (default `telegram`) |
| `HOTLINE_BOT` | Named-bot selector, same as `--bot` (legacy: `TELE_GO_BOT`) |
| `HOTLINE_STATE_DIR` | State-dir override (legacy: `TELE_GO_STATE_DIR`, then `TELEGRAM_STATE_DIR`) |
| `TELEGRAM_ACCESS_MODE` | `static` snapshots access at boot; use with `allowlist` (pairing needs live writes) |
| `DISCORD_BOT_TOKEN` | Discord bot token; `DISCORD_BOT_TOKEN_<NAME>` per named instance (`DISCORD_ACCESS_MODE` mirrors the Telegram one) |
| `SIGNAL_ACCOUNT` | Linked Signal account (E.164); `SIGNAL_ACCOUNT_<NAME>` per named instance (`SIGNAL_ACCESS_MODE` mirrors the Telegram one) |
| `SIGNAL_DAEMON_URL` | signal-cli HTTP daemon base URL (default `http://127.0.0.1:8080`); `SIGNAL_DAEMON_URL_<NAME>` per named instance |
| `ANTHROPIC_BASE_URL` | Alternate Anthropic-compatible API endpoint for the Claude Code harness (see below) |
| `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` | Provider auth: bearer token (`Authorization: Bearer`) or `x-api-key` |
| `ANTHROPIC_MODEL` | Primary model for the alternate provider; per-role overrides `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL` (`ANTHROPIC_SMALL_FAST_MODEL` is deprecated) |

### Alternate Anthropic provider (Claude Code harness)

Point the Claude Code harness at any Anthropic-API-compatible provider without exporting anything by hand. Configure it once into the shared `.env`:

```sh
hotline setup --anthropic-base-url https://provider.example/v1 \
              --anthropic-token   sk-…            # bearer  → ANTHROPIC_AUTH_TOKEN
              # or --anthropic-api-key sk-…       # x-api-key → ANTHROPIC_API_KEY
              --anthropic-model   their-model     # → ANTHROPIC_MODEL
```

`hotline start` and `hotline up` (claude path only) inject these into the Claude Code child process on launch. Only an allowlist of provider keys crosses over — `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL`, the per-role `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL` (and deprecated `ANTHROPIC_SMALL_FAST_MODEL`), `ANTHROPIC_CUSTOM_HEADERS`, `API_TIMEOUT_MS`, and `ENABLE_TOOL_SEARCH` — never the rest of your `.env`. `setup` writes the common four; add any of the others to the `.env` by hand and they're injected too. One alternate provider at a time, and your shell environment always wins over the `.env` per key (so you can override a single run). The opencode harness is unaffected — it does providers via `opencode.json`.

> Gotcha: a non-`api.anthropic.com` base URL disables Claude Code's MCP tool search. Set `ENABLE_TOOL_SEARCH=true` in the `.env` to restore it.

Operationally, hotline holds its lane: a PID guard SIGTERMs a stale poller before starting (Telegram allows one `getUpdates` consumer per token), the poll loop backs off exponentially and honors 429s, and shutdown is unified across stdin EOF, signals, and an orphan watchdog.

## The protocol

hotline is a stdio MCP server (official `github.com/modelcontextprotocol/go-sdk`) that additionally declares Claude Code's experimental `claude/channel` capability and, with a token configured, `claude/channel/permission`. Inbound messages reach Claude as `notifications/claude/channel` with a `<channel source="telegram" …>` block; permission prompts flow the other way over the same connection.

The protocol is experimental and Claude Code's. It is one of two harnesses hotline drives: OpenCode rides a separate HTTP+SSE adapter that needs no channel protocol. See [OpenCode harness](#opencode-harness).

## OpenCode harness

hotline drives two coding-agent harnesses. Select with `HOTLINE_HARNESS`:

| Value | Harness |
|---|---|
| `claude` (default) | Claude Code over the `claude/channel` protocol |
| `opencode` | OpenCode over its HTTP+SSE control plane |

An unknown value is rejected at startup instead of falling back to Claude Code.

OpenCode has no channel protocol, so the wiring splits in two. hotline runs as a plain stdio MCP server that OpenCode launches for the outbound tools (`reply`, `react`, `edit_message`, `download_attachment`). For inbound, hotline dials OpenCode's local server: it injects your texts as a user turn with `POST /session/:id/prompt_async`, tails `GET /event` for `permission.asked` events, and answers them with `POST /session/:id/permissions/:id`. When `HOTLINE_OPENCODE_AGENT` is set, each injected turn is pinned to that agent (the `agent` field on `prompt_async`) so it runs hotline's dedicated agent rather than OpenCode's default `build` assistant; empty leaves the field off and the session's default agent handles the turn. The messaging providers and access model are unchanged.

Config comes from the environment, real env winning over the shared `.env`:

| Variable | Purpose |
|---|---|
| `HOTLINE_HARNESS` | `claude` (default) or `opencode` |
| `OPENCODE_SERVER_URL` | `opencode serve` root (default `http://127.0.0.1:4096`) |
| `OPENCODE_SERVER_PASSWORD` | Basic-auth secret; empty means no auth |
| `OPENCODE_SESSION` | Pinned session id; empty auto-resolves |
| `HOTLINE_OPENCODE_AGENT` | Agent every inbound turn runs as; empty uses the session's default agent |

With `OPENCODE_SESSION` empty, hotline targets the most-recently-active session from `GET /session` and re-pins onto whichever session emits live events. OpenCode sessions are server-wide, so pin `OPENCODE_SESSION` to the session you are driving rather than trust the auto-resolve.

Scaffold the wiring with `hotline init --harness opencode`. It writes a dedicated primary agent to `.opencode/agents/hotline.md` whose entire system prompt is hotline's mechanics + texting voice — the same reply discipline, image/download guidance, and anti-prompt-injection pairing rule the Claude Code path ships. OpenCode ignores the MCP `instructions` field, so a dedicated agent is how those rules reach it: `opencode.json` pins hotline's inbound turns to this agent (`HOTLINE_OPENCODE_AGENT=hotline`) so your voice wins instead of the default `build` coding assistant. The agent file carries a `hotline-managed` marker; re-running `init` regenerates it in place, but a `hotline.md` you wrote yourself (no marker) is left untouched — and `build.md` / `AGENTS.md` are never touched. `init` also creates or merges `opencode.json` with the hotline `mcp` server (`HOTLINE_HARNESS=opencode`, `HOTLINE_OPENCODE_AGENT=hotline`, plus `--providers` if given) and a default `permission` block (`edit`/`bash` ask, `webfetch`/`external_directory` allow); an existing `permission` block or unrelated keys are left as-is. `--voice` drops a starter `HOTLINE.md`, whose contents become the agent's voice. Note opencode's permission model is coarser than Claude Code's — there is no per-tool read-allow equivalent to the plugin path's `Read`/`Grep`/`Glob` pre-approval.

Or wire it by hand. It's one `opencode.json`: your model and provider, a `permission` block, and an `mcp` entry that launches the hotline binary with the harness env. A minimal working config:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "model": "your-provider/your-model",
  "permission": { "bash": "ask" },
  "mcp": {
    "hotline": {
      "type": "local",
      "command": ["hotline"],
      "environment": {
        "HOTLINE_HARNESS": "opencode",
        "HOTLINE_PROVIDERS": "telegram",
        "TELEGRAM_BOT_TOKEN": "123456789:AA…",
        "OPENCODE_SERVER_URL": "http://127.0.0.1:4096",
        "OPENCODE_SESSION": "ses_your_pinned_session"
      }
    }
  }
}
```

Permission prompts relay over the messaging channel exactly like Claude Code's: **See more / Allow / Deny** buttons to allowlisted DMs, or `yes <code>` / `no <code>` by text. What gets gated is OpenCode's own `permission` config: leave `bash` auto-approved for a yolo-style session, or set `"bash": "ask"` to route each shell command through the relay.

OpenCode support is new. The API shapes were verified against opencode 1.17.11.

## Roadmap

Discord shipped as the second provider, Signal as the third. Matrix is under consideration. No dates.
