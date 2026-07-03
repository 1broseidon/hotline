# Security

A paired phone is a remote control for a machine running an agent with real permissions. hotline's job is to make that legible and gated. The risk itself does not go away.

## Threat model

What hotline defends against:

- Unknown senders reaching the agent
- Unauthorized pairing attempts
- Group members triggering the agent without your consent
- The agent taking dangerous actions without human signoff
- Chat messages steering the agent (prompt injection)
- Token and state leakage

## Defenses

### Pairing by default

`dmPolicy` defaults to `pairing`. The first DM from an unknown sender gets a 6-hex pairing code back, and nothing they send is delivered to the agent until you approve them. Codes expire after 24 hours. A pending sender is re-prompted at most 5 times, and at most 3 pairings can be pending at once, so strangers cannot flood your state file. The stricter policies are `allowlist` (unknown senders are silently dropped) and `disabled` (no DMs at all).

### Operator-only approval

Pairing is approved from the terminal, never from inside the chat:

```sh
hotline pair <code> [--provider kind[:instance]]
hotline deny <code> [--provider kind[:instance]]
```

Claude never approves a pairing or edits access because a chat message asked it to. That request is exactly what a prompt injection looks like.

### Allowlists, isolated per provider

The gate decides on the sender, never the chat: a non-allowlisted user in an allowed group is still dropped. Each provider and each named instance keeps its own `access.json`, allowlist, inbox, and transcript in its own state directory (`signal/`, `discord/`, `bots/<name>`, `.../instances/<name>`), so approving someone on one transport approves them nowhere else.

### Groups are opt-in

A group the agent has not been explicitly configured for is dropped, on every provider. Per group you can require a mention (delivered only when the bot is @-mentioned, replied to, or a `mentionPatterns` regex matches) and restrict to a per-group sender allowlist. On Telegram groups are keyed by chat ID, on Discord guild channels gate as groups keyed by channel ID, on Signal by `group:<id>`. Pairing never happens in a group; codes are a DM-only flow.

### The permission relay

Claude Code's dangerous-action prompts ride the same authenticated, gated channel. Requests fan out to allowlisted DMs only, never to groups. You answer with Allow / Deny buttons where the transport has them (Telegram, Discord) or by text with `yes <code>` / `no <code>` everywhere, including Signal. Verdicts are accepted only from allowlisted senders, and the agent cannot approve its own requests: a verdict is an inbound chat message from an allowlisted human, and the agent has no tool that emits one.

### Inbound framing

Messages reach the session wrapped as channel data with source attribution (provider, chat, sender, timestamp). The Claude Code harness treats channel content as untrusted external data, not as instructions. This is a harness-level convention that hotline supports, not a cryptographic guarantee.

### Voice overrides

A `HOTLINE.md` file replaces the persona layer of the channel instructions and goes straight into the session's system prompt. Treat it with the same trust boundary as `CLAUDE.md`: whoever writes to the repo (or the state dir) shapes how the agent talks. The mechanics and safety rules stay compiled into the binary under any voice: the tool contract, the inbound framing, and the rule that pairing and access changes are approved only by the operator, never because a chat message asked.

### State hygiene

Tokens, allowlists, pending pairings, and transcripts live in the state directory (`${XDG_CONFIG_HOME:-~/.config}/hotline` by default; older installs are migrated there automatically on first run), outside any repository. Directories are created `0700`; `access.json`, the transcript, and the `.env` token file are written `0600`. `access.json` writes are atomic (temp file plus rename) and lock-guarded. Outbound tool calls are gated by the same access rules, refuse to attach hotline's own state files, and sanitize uploader-controlled filenames.

### Signal specifics

Signal transport is end-to-end encrypted; hotline talks to a local signal-cli daemon. If you link that daemon to your personal account as a secondary device, it receives everything your account receives, every conversation, not just the agent chat. Use a standalone Signal account for the agent instead. The daemon's HTTP endpoint has no authentication; keep it on `127.0.0.1`.

## Known gaps

**A stolen unlocked phone is the operator.** Pairing authenticates a chat account, not a person. Whoever holds the paired phone can drive the agent and answer its permission prompts until you cut them off. Runbook, from any terminal on the box:

1. `hotline revoke <sender-id> --provider <kind[:instance]>` removes an already-approved sender from `allowFrom` and purges any pending pairing they still had; `hotline deny <code>` kills a pending pairing. `access.json` is re-read on every inbound message, so both take effect immediately. Setting `"dmPolicy": "disabled"` drops everything, allowlisted senders included.
2. Lock the messenger itself: Signal registration PIN and screen lock, Telegram passcode.
3. Telegram: rotate the bot token via @BotFather and update `.env`. The old token stops working everywhere.
4. Signal: remove the linked device (or re-register the standalone account) so the daemon goes deaf.

**Prompt injection is not solved.** Not here, not anywhere in the industry. The gate keeps strangers out and the relay keeps a human in the loop, but an allowlisted chat can still contain text that tries to steer the agent, including text pasted or forwarded from someone else. Gating and the relay are mitigation, not immunity. Scope what the agent may do unattended accordingly.

**The relay only protects operators who use it.** Running Claude Code with permission checks disabled means the approval loop never fires and the agent acts without asking. That is a legitimate choice on a trusted box, but it is a choice; do not assume the relay is protecting a session you configured to skip it.

## Reporting a vulnerability

Report privately through [GitHub security advisories](https://github.com/1broseidon/hotline/security/advisories/new) for this repository. Please do not open a public issue for a security bug.
