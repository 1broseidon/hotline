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
brew install 1broseidon/tap/hotline
```

Binaries ship with each tagged release. Or build from source:

```sh
go install github.com/1broseidon/hotline@latest   # -> $(go env GOPATH)/bin/hotline
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

   This writes (or merges into) `.mcp.json`. Add `--providers telegram,signal` for extra transports, `--voice` for a starter `HOTLINE.md`.

3. Launch Claude Code with the channel loaded:

   ```sh
   hotline start              # extra claude flags go after --, e.g. hotline start -- --continue
   ```

4. DM your bot. The first message from an unknown sender returns a 6-hex pairing code. Approve it from your terminal:

   ```sh
   hotline pair <code>
   ```

That's it. Your session is now a Telegram chat.

<details>
<summary>By hand</summary>

The three commands wrap these steps:

```sh
# setup: token in the channel's .env (a real TELEGRAM_BOT_TOKEN env var wins over the file)
mkdir -p ~/.config/hotline
printf 'TELEGRAM_BOT_TOKEN=123456789:AA…\n' > ~/.config/hotline/.env
chmod 600 ~/.config/hotline/.env
```

```json
// init: .mcp.json in the project
{
  "mcpServers": {
    "hotline": { "command": "hotline", "args": ["run"] }
  }
}
```

```sh
# start: channels are experimental and loaded by MCP server name
claude --dangerously-load-development-channels server:hotline
```

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

Button taps come back as ordinary inbound messages whose content is the tapped label, verbatim, so values are never truncated by Telegram's callback-data limit. Tap authorization mirrors the inbound gate.

Every message in both directions is appended to `<stateDir>/transcript.jsonl`. Telegram has no history API and Claude Code sessions compact or restart, so the transcript is how the assistant recalls the thread across resets. It's written 0600 and currently unbounded.

## Voice

The channel instructions ship with a default persona: a sharp, warm friend texting in short bubbles. Drop a `HOTLINE.md` file to replace it. First hit wins:

1. `./HOTLINE.md` in the directory Claude Code runs in, for a per-repo voice
2. `HOTLINE.md` in the state dir (`~/.config/hotline` by default), your global default
3. the built-in voice

The file is read once at startup. Edit it, then restart Claude Code. Files over 16KB are truncated; empty files are skipped.

A voice changes tone only. The tool contract, inbound message handling, and the safety rules (operator-only pairing approval, the injection stance) are compiled in and apply under any voice.

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

`HOTLINE_PROVIDERS` is read from the process environment, not the state `.env`. Set it where the MCP server is launched: the `env` block of your `.mcp.json`:

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
hotline init         register hotline in this repo's .mcp.json
hotline start        launch Claude Code with the channel loaded
hotline [run]        start the MCP server + Telegram poller (default)
hotline pair <code>  approve a pending pairing code
hotline deny <code>  reject a pending pairing code
hotline revoke <id>  remove an approved sender from the allowlist
hotline status       print state-dir / token / access summary
```

`pair`, `deny`, `revoke`, and `status` take `--provider kind[:instance]` to select which provider's state they operate on (default: telegram).

## State and environment

State lives in `${XDG_CONFIG_HOME:-~/.config}/hotline`. On first run, state found at the old default `~/.claude/channels/tele-go` is copied over automatically; the old directory is left in place so sessions still running an older binary keep working. hotline was formerly `tele-go`; `TELE_GO_*` variables keep working as fallbacks for one release.

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

Operationally, hotline holds its lane: a PID guard SIGTERMs a stale poller before starting (Telegram allows one `getUpdates` consumer per token), the poll loop backs off exponentially and honors 429s, and shutdown is unified across stdin EOF, signals, and an orphan watchdog.

## The protocol

hotline is a stdio MCP server (official `github.com/modelcontextprotocol/go-sdk`) that additionally declares Claude Code's experimental `claude/channel` capability and, with a token configured, `claude/channel/permission`. Inbound messages reach Claude as `notifications/claude/channel` with a `<channel source="telegram" …>` block; permission prompts flow the other way over the same connection.

The protocol is experimental and Claude Code's. Today that means hotline works with Claude Code and nothing else. If other harnesses adopt the channel protocol, or hotline grows adapters for theirs, that changes.

## Roadmap

Discord shipped as the second provider, Signal as the third. Matrix is under consideration. No dates.
