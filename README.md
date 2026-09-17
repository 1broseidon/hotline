# Hotline

**A local-first room for your team of coding agents.**

Most AI coding tools give you one chat, with one assistant, welded to one
computer. Hotline gives you a room: a named roster of teammates that belongs to
you — not to a machine, and not to a cloud. Each teammate has its own name,
its own project directory, and its own conversation that keeps going whether
or not you are watching. Some run Hotline's built-in agent with a model key you
hold; others drive the tools you already have — Claude Code, `cursor-agent`,
`opencode` — each in its own lane.

Hotline shipped as **Toad** through 0.13.0 and took its present name in
0.14.0; the repository, its history and the app are the same ones.

This is the ground-up build in Rust; it replaced the Electrobun
edition on this repository on 2026-09-08. How it is put together and why
is [docs/design.md](docs/design.md); how to change it is [AGENTS.md](AGENTS.md).
How to run it is [docs/development.md](docs/development.md); the wire is
[docs/wire.md](docs/wire.md); the log is [docs/log.md](docs/log.md); a
session is [docs/sessions.md](docs/sessions.md).
