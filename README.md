# hotline

A messaging channel for Claude Code. hotline is an MCP server that relays a Claude Code session to Telegram, so you talk to your agent the way you text a friend: short bubbles, reactions, tappable buttons, photos.

It speaks Claude Code's experimental two-way channel protocol (`claude/channel`). That makes it Claude Code specific today. Other harnesses work if they adopt the protocol, or when hotline adds support for them.

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
go install github.com/1broseidon/hotline@latest   # -> $(go env GOPATH)/bin/hotline
```

Requires Go 1.26+. No prebuilt binaries yet.

## Quickstart

1. Get a bot token from [@BotFather](https://t.me/BotFather).

2. Drop it in the channel's `.env`:

   ```sh
   mkdir -p ~/.claude/channels/tele-go
   printf 'TELEGRAM_BOT_TOKEN=123456789:AA…\n' > ~/.claude/channels/tele-go/.env
   chmod 600 ~/.claude/channels/tele-go/.env
   ```

   A real `TELEGRAM_BOT_TOKEN` environment variable wins over the `.env` file.

3. Register hotline as an MCP server in the project you want to text with, via `.mcp.json`:

   ```json
   {
     "mcpServers": {
       "hotline": { "command": "hotline", "args": ["run"] }
     }
   }
   ```

4. Start Claude Code with the channel flag (channels are experimental and loaded by MCP server name):

   ```sh
   claude --dangerously-load-development-channels server:hotline
   ```

5. DM your bot. The first message from an unknown sender returns a 6-hex pairing code. Approve it from your terminal:

   ```sh
   hotline pair <code>
   ```

That's it. Your session is now a Telegram chat.

## Access model

Nobody talks to your agent without your say-so. The inbound gate decides on the sender, never the chat: a stranger in an allowed group is still dropped.

| Policy | Behavior |
|---|---|
| `pairing` (default) | Unknown DM sender gets a pairing code; you approve or deny from the terminal |
| `allowlist` | Only listed user IDs are delivered; everyone else is silent |
| `disabled` | No DMs at all |

Groups are opt-in per group ID, with optional `requireMention` (deliver only when the bot is mentioned, replied to, or a `mentionPatterns` regex matches) and an optional per-group sender allowlist.

Configuration lives in `~/.claude/channels/tele-go/access.json` and is re-read on every inbound message, so edits take effect live:

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

Pairing codes expire after 24 hours. Approval happens only from your terminal: Claude never approves a pairing or edits access because a chat message asked it to. That request is exactly what a prompt injection looks like.

Outbound is gated too. Every tool call checks the target chat against the same rules before touching the Telegram API, so Claude can only message chats that could message it. The channel also refuses to attach its own state files, and sanitizes uploader-controlled filenames before they enter the message metadata.

## The tools

| Tool | What it does |
|---|---|
| `reply` | Send `bubbles` (array of short messages, paced) or `text` (single message, chunked past 4096 chars). Optional `reply_to`, `files` (images inline, others as documents, 50MB each), `buttons` (up to 12 inline options), `format` (`text`, `markdownv2`, `html`) |
| `react` | Set an emoji reaction (Telegram's fixed whitelist) |
| `edit_message` | Edit a message the bot sent, for interim progress. Edits don't push-notify |
| `download_attachment` | Fetch a non-photo attachment by `file_id` into the inbox; returns a local path (Telegram's 20MB download cap applies) |

Button taps come back as ordinary inbound messages whose content is the tapped label, verbatim, so values are never truncated by Telegram's callback-data limit. Tap authorization mirrors the inbound gate.

Every message in both directions is appended to `<stateDir>/transcript.jsonl`. Telegram has no history API and Claude Code sessions compact or restart, so the transcript is how the assistant recalls the thread across resets. It's written 0600 and currently unbounded.

## Multiple providers, multiple bots

hotline is built on an internal provider interface with a source router. Telegram is the first provider. Configure providers with `HOTLINE_PROVIDERS`, a comma-separated list of `kind[:instance]` entries:

```sh
HOTLINE_PROVIDERS=telegram              # the default
HOTLINE_PROVIDERS=telegram:work         # a named instance
```

With one provider configured, the tool schemas are byte-identical to the single-provider ones above. With several, each tool takes a required `source` argument matching the `source` attribute on inbound messages.

Named instances are how you run several sessions at once. One bot token allows exactly one Telegram poller, so each concurrent session gets its own bot. `--bot work` is shorthand for `HOTLINE_PROVIDERS=telegram:work`. Each named bot keeps isolated state under `<stateDir>/bots/<name>/` and reads its token from `TELEGRAM_BOT_TOKEN_<NAME>` in the shared `.env`:

```sh
# ~/.claude/channels/tele-go/.env
TELEGRAM_BOT_TOKEN=111:AA…            # default bot
TELEGRAM_BOT_TOKEN_WORK=222:BB…       # telegram:work
```

`--bot` works on every subcommand (`hotline status --bot work`, `hotline pair <code> --bot work`).

When a future transport lacks a feature, the adapter degrades it, never the agent: on a transport without inline buttons, buttons render as numbered text options and the numbered choice routes back the same way. The tool contract stays the same everywhere.

## Permission relay

When a token is configured, hotline declares the `claude/channel/permission` capability. Claude Code's permission prompts are relayed to allowlisted DMs (never groups) with **See more / Allow / Deny** buttons. You can also answer by text with `yes <code>` or `no <code>`; those replies are intercepted and converted to a verdict instead of being relayed as chat.

## CLI

```
hotline [run]        start the MCP server + Telegram poller (default)
hotline pair <code>  approve a pending pairing code
hotline deny <code>  reject a pending pairing code
hotline status       print state-dir / token / access summary
```

## State and environment

State lives in `~/.claude/channels/tele-go` (the directory keeps its pre-rename name, so existing pairings, transcripts, and inboxes carry over). hotline was formerly `tele-go`; `TELE_GO_*` variables keep working as fallbacks for one release.

| Variable | Purpose |
|---|---|
| `TELEGRAM_BOT_TOKEN` | Bot token (real env wins over `.env`); `TELEGRAM_BOT_TOKEN_<NAME>` per named instance |
| `HOTLINE_PROVIDERS` | Provider list, `kind[:instance]` comma-separated (default `telegram`) |
| `HOTLINE_BOT` | Named-bot selector, same as `--bot` (legacy: `TELE_GO_BOT`) |
| `HOTLINE_STATE_DIR` | State-dir override (legacy: `TELE_GO_STATE_DIR`, then `TELEGRAM_STATE_DIR`) |
| `TELEGRAM_ACCESS_MODE` | `static` snapshots access at boot; use with `allowlist` (pairing needs live writes) |

Operationally, hotline holds its lane: a PID guard SIGTERMs a stale poller before starting (Telegram allows one `getUpdates` consumer per token), the poll loop backs off exponentially and honors 429s, and shutdown is unified across stdin EOF, signals, and an orphan watchdog.

## The protocol

hotline is a stdio MCP server (official `github.com/modelcontextprotocol/go-sdk`) that additionally declares Claude Code's experimental `claude/channel` capability and, with a token configured, `claude/channel/permission`. Inbound messages reach Claude as `notifications/claude/channel` with a `<channel source="telegram" …>` block; permission prompts flow the other way over the same connection.

The protocol is experimental and Claude Code's. Today that means hotline works with Claude Code and nothing else. If other harnesses adopt the channel protocol, or hotline grows adapters for theirs, that changes.

## Roadmap

Discord is next as a second provider. Matrix is under consideration. No dates.
