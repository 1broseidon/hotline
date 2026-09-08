# Toad

**A local-first room for your team of coding agents.**

Most AI coding tools give you one chat, with one assistant, welded to one
computer. Toad gives you a room: a named roster of teammates that belongs to
you — not to a machine, and not to a cloud. Each teammate has its own name,
its own project directory, and its own conversation that keeps going whether
or not you are watching. Some run Toad's built-in agent with a model key you
hold; others drive the tools you already have — Claude Code, `cursor-agent`,
`opencode` — each in its own lane.

This is the ground-up build of Toad in Rust; it replaced the Electrobun
edition on this repository on 2026-09-08. How it is put together and why
is [docs/design.md](docs/design.md); how to change it is [AGENTS.md](AGENTS.md).
How to run it is [docs/development.md](docs/development.md); the wire is
[docs/wire.md](docs/wire.md); the log is [docs/log.md](docs/log.md); a
session is [docs/sessions.md](docs/sessions.md).
