# mission-control

A persistent agent you text, with memory on disk. This template turns one Claude Code session into a long-running assistant that tracks everything you throw at it: projects, errands, research rabbit-holes, random questions. You reach it over Telegram (or Signal, or Discord). It files everything so that nothing depends on the session staying alive.

This is the pattern behind a real production setup, generalized. Copy it, launch it, then grow it by chatting.

## Setup

From the [hotline quickstart](../../README.md#quickstart):

```sh
hotline setup --telegram-token <token>   # once, machine-wide
cd your-mission-control-folder           # a copy of this template
hotline init
hotline start
```

DM your bot, approve the pairing code with `hotline pair <code>`, and you're in.

## The filing system

The whole design answers one question: can you drop a thread for two weeks, or restart the session, and pick it back up cold? Sessions compact. Processes die. So state lives on disk, never in the agent's head.

```
mission-control/
  CLAUDE.md          # the agent's playbook
  HOTLINE.md         # its texting voice
  INDEX.md           # master registry of every active thread
  threads/<slug>/    # one folder per project or topic
    README.md        # what it is, status, next action, pointers
  inbox/             # quick captures not yet filed
  archive/           # finished or dead threads
```

- **INDEX.md** is the map. One row per active thread. The agent reads it first in any fresh session, so a cold start still knows the whole landscape.
- **threads/\<slug\>/README.md** is the memory of one topic. It always answers: what is this, where does it stand, what happens next, where are the real files. Real projects stay where they live on your machine; the thread just points at them.
- **inbox/** is for capture speed. A stray link or idea gets dropped there in one line, no filing decision needed. The agent sweeps it into threads later.
- **archive/** is where finished work goes. Moved, not deleted. Six months later you can still see how something ended.

## Operating habits

The `CLAUDE.md` teaches the agent three habits:

1. **Write as you go.** Every meaningful step updates the relevant thread README. If the session died mid-task, the next one could resume from disk alone.
2. **Delegate heavy work.** The main session coordinates. Implementation, long research, multi-file changes go to subagents, which keeps the main thread responsive to your messages while work runs.
3. **Acknowledge, then ping.** Before anything long it sends a quick "on it," and when done it sends a fresh message so your phone buzzes.

## Recurring jobs

Once the agent is persistent, you can hand it standing duties: watch a mailbox, sweep the inbox nightly, check a feed each morning. The pattern that works:

- **Silent unless there's a hit.** A watcher that reports "nothing new" every hour trains you to ignore it. It should message you only when something clears the bar you set.
- **Self-healing.** Ask the agent to make each job re-create itself if it dies, and to check its jobs are alive when a session starts.

Claude Code has scheduling abilities for recurring work; set jobs up by describing what you want to the agent and let it pick the mechanism available in your install.

## Voice

`HOTLINE.md` in this folder sets how the agent texts. This template ships a short starter: friendly, concise, one thought per bubble. Edit it, restart, done. Details and the character budget are in the [voice section](../../README.md#voice) of the main README.

For example, to make it terse for work:

```markdown
Terse and professional. No emoji.
One bubble unless the answer genuinely needs two.
```

## Security defaults

Pairing is on by default: strangers who DM your bot get a code you approve from the terminal with `hotline pair <code>`, and nothing they send reaches the agent before that. Remove someone later with `hotline revoke <id>`. When the agent wants to do something that needs permission, the prompt is relayed to your chat with Allow/Deny buttons. Full threat model in [SECURITY.md](../../SECURITY.md).

## Files in this template

| File | Purpose |
| --- | --- |
| [CLAUDE.md](CLAUDE.md) | The agent's playbook: role, filing rules, communication |
| [HOTLINE.md](HOTLINE.md) | Starter texting voice |
| [INDEX.md](INDEX.md) | Thread registry skeleton |
| [threads/example-thread/README.md](threads/example-thread/README.md) | A filled-in miniature thread |
