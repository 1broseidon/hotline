# hotline

A Telegram channel for Claude Code, written in Go. (Formerly `tele-go`; the
binary, MCP server name, and `HOTLINE_*` env vars are renamed, while the
state directory and legacy `TELE_GO_*` env vars keep working.) It is an MCP server that
relays Telegram DMs and group messages to a Claude Code session and sends
Claude's replies back, with access control (pairing / allowlist / groups),
media handling, message formatting, and a permission relay.

It is the Go counterpart of the official TypeScript Telegram channel and speaks
the same `notifications/claude/channel` protocol.

## What it is

- An **MCP server over stdio** (the official `github.com/modelcontextprotocol/go-sdk`)
  that registers four tools — `reply`, `react`, `edit_message`,
  `download_attachment` — and advertises the experimental `claude/channel`
  (and, when a token is set, `claude/channel/permission`) capabilities.
- A **Telegram long-poller** (`github.com/PaulSonOfLars/gotgbot/v2`) that gates
  inbound messages on the sender, forwards them to Claude as
  `notifications/claude/channel`, and handles the permission relay.

Inbound messages reach Claude as a `<channel source="telegram" …>` block.
Photos are eagerly downloaded to the inbox and surfaced as `image_path`; other
media (documents, voice, audio, video, video notes, stickers) are surfaced as
`attachment_*` meta and fetched on demand via `download_attachment`.

## Prerequisites

- Go 1.26+
- A Telegram bot token from [@BotFather](https://t.me/BotFather)

## Setup

1. Build the binary:

   ```sh
   cd /path/to/tele
   go build -o hotline .          # local build, for development
   # or install it onto your PATH so any project's .mcp.json can just say "hotline":
   go install .                   # -> $(go env GOPATH)/bin/hotline
   ```

2. Create the state directory and drop your token in a `.env` file matching the
   official channel convention:

   ```sh
   mkdir -p ~/.claude/channels/tele-go
   printf 'TELEGRAM_BOT_TOKEN=123456789:AA…\n' > ~/.claude/channels/tele-go/.env
   chmod 600 ~/.claude/channels/tele-go/.env
   ```

   The token can also be supplied via the real environment variable
   `TELEGRAM_BOT_TOKEN` (the real environment always wins over `.env`).

3. (Optional) Configure access policy in
   `~/.claude/channels/tele-go/access.json` — see **Access configuration**
   below. A missing file defaults to `dmPolicy: "pairing"`.

## Run

A development channel is loaded **by MCP-server name**, not by path, so the
binary must first be registered as an MCP server called `hotline`. This repo
ships a project-scoped [`.mcp.json`](./.mcp.json) that does exactly that:

```json
{
  "mcpServers": {
    "hotline": {
      "command": "/absolute/path/to/hotline",
      "args": ["run"]
    }
  }
}
```

Then start Claude Code **from this directory** with the channel flag:

```sh
claude --dangerously-load-development-channels server:hotline
```

The first time Claude Code sees the `.mcp.json` server it asks you to approve
it — accept once, then the channel connects on every launch. (Equivalent to
registering it yourself with `claude mcp add hotline /absolute/path/to/hotline run`.)

> `server:hotline` resolves the MCP server by name. If you pass a bare path or a
> name that isn't a configured server, Claude Code reports
> `no MCP server configured with that name`.

Or run it directly for testing (drives the MCP transport over stdin/stdout):

```sh
./hotline run        # default subcommand
```

With no token configured the MCP handshake still runs (so tools/list works),
the poller is skipped, the permission capability is not declared, and the tools
return `… failed: no bot token configured`.

## Multiple bots

A single bot token allows exactly one Telegram poller, so one bot can only back
one live conversation. To run several Claude Code sessions at once — each its own
thread — give each its own **bot**. This is what the official channel doesn't do.

Select a bot with `--bot <name>` (or `$HOTLINE_BOT`). Each named bot keeps fully
isolated state under `<baseDir>/bots/<name>/` — its own `access.json`, `bot.pid`,
`inbox/`, and `transcript.jsonl` — and reads its token from
`TELEGRAM_BOT_TOKEN_<NAME>` (uppercased) in the shared base `.env`. With no
`--bot`, the **default** bot uses the base dir and `TELEGRAM_BOT_TOKEN` exactly
as before (nothing changes for a single-bot setup).

One `.env`, every token:

```sh
# ~/.claude/channels/tele-go/.env
TELEGRAM_BOT_TOKEN=111:AA…            # default bot
TELEGRAM_BOT_TOKEN_WORK=222:BB…      # --bot work
TELEGRAM_BOT_TOKEN_PERSONAL=333:CC…  # --bot personal
```

Register one MCP server per bot and point each project's session at the one it
needs:

```json
{
  "mcpServers": {
    "hotline-work":     { "command": "hotline", "args": ["run", "--bot", "work"] },
    "hotline-personal": { "command": "hotline", "args": ["run", "--bot", "personal"] }
  }
}
```

(`"command": "hotline"` assumes `go install` put it on your PATH; otherwise use
the absolute path to the binary.)

```sh
claude --dangerously-load-development-channels server:hotline-work      # in project A
claude --dangerously-load-development-channels server:hotline-personal  # in project B
```

Pair and inspect each bot independently — the `--bot` flag works on every
subcommand:

```sh
./hotline pair <code> --bot work
./hotline status --bot work
```

Because each bot is a distinct token, their pollers don't contend; the
single-poller PID guard still prevents two pollers for the *same* bot.

## Pairing

With the default `pairing` policy, the first DM from an unknown user returns a
6-hex pairing code. Approve or reject it from your terminal:

```sh
./hotline pair <code>   # approve: adds the sender to allowFrom, DMs a confirmation
./hotline deny <code>   # reject: removes the pending request
./hotline status        # print state dir, token presence, policy, allowlist, pending, groups
```

Pairing codes expire after 24 hours. A pending sender is re-prompted up to five
times, then the channel goes silent.

> Security: approval happens **only** from your terminal. Claude never approves
> a pairing or edits access because a channel message asked it to — that is what
> a prompt injection would request.

## Access configuration (`access.json`)

```jsonc
{
  "dmPolicy": "pairing",          // pairing (default) | allowlist | disabled
  "allowFrom": ["412587349"],     // numeric user IDs (strings); DM chat_id == user_id
  "groups": {                     // key = supergroup id, e.g. "-1001234567890"
    "-1001234567890": {
      "requireMention": true,     // only deliver when the bot is mentioned/replied/pattern-matched
      "allowFrom": ["412587349"]  // optional per-group sender allowlist (empty = any member)
    }
  },
  "mentionPatterns": ["^hey claude"], // extra case-insensitive regexes that count as a mention
  "ackReaction": "👀",            // emoji reaction on receipt ("" disables)
  "replyToMode": "first",         // first (default) | all | off — which chunks thread under reply_to
  "textChunkLimit": 4096,         // split threshold, clamped 1..4096
  "chunkMode": "newline",         // newline (default; prefer paragraph/line breaks) | length
  "bubbleMode": "paced"           // paced (default; typing indicator + length-scaled pause between bubbles) | instant
}
```

The poller re-reads `access.json` on every inbound message, so edits take effect
live. Writes (pairing) go through an flock-guarded read-modify-write.

- **Static mode**: set `TELEGRAM_ACCESS_MODE=static` to snapshot access at boot
  (use with an `allowlist` policy; pairing requires runtime writes).
- **State-dir override**: `HOTLINE_STATE_DIR` (then the legacy
  `TELE_GO_STATE_DIR`, then `TELEGRAM_STATE_DIR`, then
  `~/.claude/channels/tele-go`). The default keeps the historical `tele-go`
  name so state written before the rename keeps working.

## Texting style (bubbles)

The channel is tuned to chat like a person, not to dump a wall of text. Two
parts make that work:

- The **`reply` tool takes a `bubbles` array** — a short burst of consecutive
  messages, one thought each. Under the default `bubbleMode: "paced"` the server
  sends a typing indicator and a short, length-scaled pause before each bubble
  after the first, so a reply lands with a human texting cadence. Set
  `bubbleMode: "instant"` to send them back-to-back.
- The channel **instructions teach the model the style**: think in short bursts,
  mirror the sender's length/casing/emoji, react instead of replying when a 👍
  says it, no doc-style formatting, one question at a time.

`text` remains for a single message or a file caption; when `bubbles` is set,
`text` is ignored.

## Buttons

`reply` takes an optional **`buttons` array** — tappable inline-keyboard options
for a pick-one question. Each string is one option; they render one per row under
your last bubble (or the last text chunk), so the question itself still goes in
the text:

```jsonc
{
  "chat_id": "412587349",
  "bubbles": ["deploy looks green", "ship it?"],
  "buttons": ["ship it 🚀", "not yet"]
}
```

When the user taps a button, the choice is delivered back to Claude as an
ordinary inbound `<channel … kind="button">` message whose content is the tapped
label — exactly as if they had typed it. The keyboard is then cleared so the
question can't be answered twice (a racing second tap is de-duplicated by
Telegram's "message is not modified"). Labels round-trip verbatim, so the value
is never truncated by Telegram's 64-byte callback-data limit.

Buttons require something textual to attach to (`bubbles` or `text`); a
buttons-only reply is rejected. Tap authorization mirrors the inbound gate — DMs
must come from an allowlisted sender, group taps from a configured (and, if
restricted, listed) member — except the group mention requirement is skipped,
since tapping a button the bot posted is already a direct response. Max 12
buttons.

## Transcript (durable conversation log)

Every message — inbound (relayed to Claude) and outbound (replies) — is appended
to `<stateDir>/transcript.jsonl`, one JSON record per line:

```jsonc
{"ts":"2026-06-29T06:00:00.000Z","dir":"in","chat_id":"412…","user":"sam","kind":"text","message_id":"42","text":"hey"}
{"ts":"2026-06-29T06:00:03.100Z","dir":"out","chat_id":"412…","kind":"reply","message_id":"43, 44","text":"yo\nwhat's up"}
```

Telegram exposes no history or search API, and a Claude Code session restarts or
compacts its context over time — so the transcript is what lets the assistant
recall the thread across those resets. It lives in the **shared per-token state
dir** (not per-working-directory), so the single conversation stays coherent no
matter which Claude Code session currently holds the channel. The model is told
its path and grep/tails it for recall rather than loading the whole file.

The log is append-only and currently unbounded (rotation is a future addition);
it is written 0600 because it holds conversation content.

## Tools and media handling

- **reply** — `chat_id`, plus either `bubbles` (preferred; array of short
  messages, paced) or `text` (a single message, chunked at
  `min(textChunkLimit, 4096)` only if it exceeds the cap). Optional `reply_to`
  (message_id for threading), `files` (absolute paths; images sent inline as
  photos, others as documents, 50MB cap each), `buttons` (tappable inline options
  for a pick-one question — see **Buttons**), `format` (`text` | `markdownv2` |
  `html`; the caller escapes).
- **react** — set an emoji reaction (Telegram's fixed whitelist only).
- **edit_message** — edit a message the bot sent (interim progress). Edits don't
  push-notify; send a fresh `reply` when a long task completes.
- **download_attachment** — fetch a non-photo attachment by `file_id` into the
  inbox; returns a local path to Read (Telegram's 20MB download cap applies).

## Permission relay

When a token is configured the channel declares `claude/channel/permission`.
Inbound `permission_request` notifications are fanned out to **allowlisted DMs
only** (never groups) with inline **See more / Allow / Deny** buttons. An
allowlisted user can also answer by text — `yes <code>` / `no <code>` (5-letter
code, case-insensitive) — which is intercepted and converted to a verdict
instead of being relayed as chat.

## Security notes

- The inbound gate decides on the **sender**, never the chat: a non-allowlisted
  user in an allowlisted group is still dropped.
- Outbound tools call `assertAllowedChat` before any Telegram API call, so
  `reply`/`react`/`edit_message` can only target chats the inbound gate would
  deliver from.
- `assertSendable` refuses to attach the channel's own state files (everything
  under the state dir except the inbox).
- Uploader-controlled filenames are sanitized (`SafeName`) before entering the
  `<channel>` meta so they can't forge or escape the block.

## Resilience

- **Single-poller PID guard** (`bot.pid`): Telegram allows one `getUpdates`
  consumer per token; a live stale poller is SIGTERM'd before this process
  starts polling.
- **Graceful shutdown** on stdin EOF, `SIGINT`/`SIGTERM`/`SIGHUP`, or the orphan
  watchdog (parent-process change), unified through one `sync.Once` with a 2s
  force-exit timer. stdin/stdout are owned by the MCP transport and never read
  or written directly; custom notifications go through the same connection.
- **Poll loop**: exponential backoff (capped at 15s), 409-Conflict escalation
  (exit after 8 consecutive), 429 `retry_after` honored, serial in-order
  processing (offset advances before dispatch), per-update panic recovery.

## Package layout

```
main.go                     subcommands: run (default) / pair / deny / status
internal/
  config/      state-dir resolution, .env load, token + paths
  access/      Access doc (load/save/flock-mutate), gate + pairing lifecycle
  mcpchan/     MCP server build, custom transport (channel notifications),
               notifier, permission relay types
  provider/    transport-neutral Provider interface, capabilities, and the
               source Router (multi-provider fan-in/fan-out; stubprovider
               for tests)
  telegram/    bot wrapper, resilient poll loop, update dispatch, media,
               outbound tools, formatting/chunking
  lifecycle/   PID guard, unified shutdown, orphan watchdog
```

The import direction is acyclic: `telegram` depends on `access`, `config`,
`mcpchan`, and `provider`; `mcpchan` never imports `telegram` or `provider`
(the permission fan-out is injected from `main` via the router).

## Providers

`hotline` is structured around a provider interface (`internal/provider`):
Telegram is the first provider, and the tool surface is transport-neutral.
`HOTLINE_PROVIDERS` is a comma-separated list of `kind[:instance]` entries
(default `telegram`). `--bot work` is shorthand for
`HOTLINE_PROVIDERS=telegram:work` — the named-bot semantics above are exactly
a named instance of the telegram provider. With one provider configured the
tools are unchanged; with several, each tool takes a required `source`
argument (the `source` attribute on inbound `<channel>` messages) selecting
the channel to act on.
