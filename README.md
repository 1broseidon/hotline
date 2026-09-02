# Toad

**A local-first room for your team of coding agents.**

Most AI coding tools give you one chat, with one assistant, welded to one
computer. Toad gives you a room: a named roster of teammates that belongs to
you — not to a machine, and not to a cloud. Each teammate has its own name,
its own project directory, and its own conversation that keeps going whether
or not you are watching. Some run Toad's built-in agent with a model key you
hold; others drive the tools you already have — Claude Code, `cursor-agent`,
`opencode` — each in its own lane.

This is the ground-up build of Toad in Rust. The previous Toad lives at
[github.com/1broseidon/toad](https://github.com/1Broseidon/toad) and stays
the shipping app until this one replaces it. How it is put together and why
is [docs/design.md](docs/design.md); how to change it is [AGENTS.md](AGENTS.md).
How to run it is [docs/development.md](docs/development.md); the wire is
[docs/wire.md](docs/wire.md); the log is [docs/log.md](docs/log.md).
