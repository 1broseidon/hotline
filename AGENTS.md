# Working in this repository

Toad is a local-first room for a **team** of coding agents. This tree is the
ground-up build of it, in Rust, started 2026-09-01. This file is the contract
for the agent changing Toad; the [README](README.md) is the product story;
[docs/design.md](docs/design.md) is the decision record every change is
checked against; [docs/security.md](docs/security.md) is the method for any
change to what a teammate can reach, and the index of the tests that prove
it. Read the design first. Treat everything here as good
defaults, not hard rules: George's explicit request outranks any line in this
file, and if a rule fights the task in front of you, say so before breaking it.

## Vocabulary

- **you** — the agent reading this file and changing Toad.
- **George** — the maintainer. Who you are talking to.
- **user** — a person running Toad to direct a team of agents.
- **teammate** — one agent in the rail: a goal, a working directory, a reach,
  and a disposition. The code calls it a *persona* where the contract does.
- **stream** — an append-only JSONL log folded by event id. The `room`
  stream holds the roster and settings; a `tape` is a teammate's conversation;
  a `thread` is a conversation between two teammates.
- **session** — one live conversation with one agent, on behalf of one
  teammate. Its **driver** is either Toad Agent in-process on Rig, or an ACP
  child process. The session's rules exist once; a driver knows nothing of
  tapes.
- **the wire** — one WebSocket per client: commands, stream subscriptions,
  view subscriptions. A **seat** is the set of things a socket may do.
- **the reference tree** — `../toad`, the previous Toad. Read it for the rules
  a behaviour must keep (its comments say why); never modify it from here.

## The ways to hurt yourself

1. **Toad develops Toad.** You may be running inside a Toad while you change
   this one. Never kill by matched name, path or port; only a PID you
   captured at spawn. `TOAD_DATA_DIR` overrides the data directory, and
   `make dev` sets it to `.toad-dev` in the checkout so nothing you do
   reaches real data.
2. **One writer per stream.** The core is the only process that appends to
   a stream. A harness that wants state in a stream asks the core over the
   wire, or writes the file before the core starts.
3. **The generated contract is written by the whole test run.** ts-rs writes
   `ui/src/generated/contract.ts` from the export tests, and a filtered run
   (`cargo test session`) writes only the types it matched, leaving a file
   the window cannot compile against. Run `cargo test -p toad-core` unfiltered
   before committing that file; `make check` catches it, so never skip it.
4. **The reference tree is byte-compatible for tapes.** A tape here is
   the same file as a tape there. Do not change the event shapes or the
   segment layout without changing the importer and saying so in the design.

## Running it

```bash
make dev        # the Tauri shell, Vite with hot reload, on .toad-dev
make check      # cargo fmt --check, clippy -D warnings, cargo test, the UI's typecheck
make verify     # the headless harnesses, driving the real core over the wire
```

## Verifying

The house idiom is a headless harness in `crates/toad-core/tests/` that
starts the real core and drives it over the wire (`make verify`). Find the one covering your area and extend it;
new behaviour ships with one. Unit tests live beside the code. `make check`
before calling work done; the smallest proof that the change works, not
everything.

## Releases and taste

- Commits are one line that states the invariant, not the diff: *"A Windows
  tile is 256 across, because one byte cannot count higher."*
- Fight for the smallest model that makes the correct behavior unsurprising.
  Do not preserve complexity because it exists, or add machinery because it
  looks architecturally impressive.
- Comments and docs explain *why*, in sentences, and move when the code moves.
  `docs/` holds only what is; a doc that drifts from the code is a bug. The
  decision record is `docs/design.md` and the board (`.brainfile/`, gitignored).

## Code standards

- **Delete, don't disable.** When you replace or supersede code, remove the old
  path in the same change — no commented-out blocks, no unused helpers or
  exports, no flags guarding a branch nothing takes, no "keeping this for
  reference." If it isn't reachable from a caller or a test, it doesn't ship.
  Git holds the history; the repo holds only what runs.
- **Fewer moving parts beats fewer lines.** Prefer the version with less
  indirection: a function over a class hierarchy, a direct call over an event
  hop, a literal over a config value with one consumer. Don't add an abstraction
  until there's a second real caller — build for the requirement in front of
  you, not a predicted one. If a change introduces a layer, state in one
  sentence what breaks without it.
- **Write for the next reader, who has none of your context.** Obvious names
  over short ones, explicit steps over dense one-liners, boring
  standard-library idioms over clever tricks. If a line needs a comment
  explaining *how* it works, rewrite the line; reserve comments for constraints
  the code can't express. When a clever solution and a plain one both work,
  ship the plain one.
- **Rust that needs Tauri lives in `crates/toad-desktop`.** `toad-core` never
  depends on Tauri.
- **A capability change follows `docs/security.md`.** Anything that alters
  what a teammate can reach — a tool, a tool origin, a grant, a driver path —
  is enforced in the core outside the model, revocable through the session's
  lease, proved allowed and denied through the real handler, and stated in
  the PR as default, grant source, enforcement points, tests, residual risk.
