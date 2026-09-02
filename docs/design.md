# Toad, from the ground up — the design record

Written 2026-09-01, the day George chose a fresh tree over the strangler-fig
migration of `../toad`. This is the decision record: what is kept from Toad
as it exists, what is built differently, and why. It is the document every
phase is checked against. The product story is the README; the contract for
agents changing this tree is `AGENTS.md`.

George's reasons, in his words: the proof that the thing works is in the
core of the Bun edition; Rust is for sheer speed and far more control, with
Rig and our own custom tools; and Tauri is much more actively maintained
than Electrobun. The Bun edition's specs are carried onto this tree's board
as plans tagged `carried`, each headed by what the new design changes.

## What is kept, because it is right

These come over as ideas and, where the code was already Rust, as code.

- **The room.** A named roster of teammates that belongs to a person, not to
  a machine and not to a cloud. Local-first, no accounts, no telemetry.
- **A teammate is four axes**: identity (`goal`, materialised as `AGENTS.md`
  in its workspace), workspace (`cwd`), capability (which tools), disposition
  (model and mode). Its transcript is one long conversation, forever.
- **The tape is the record and SQLite is an index.** Append-only JSONL,
  folded by event id on load, so a tool call moving from pending to
  completed is one line superseding another. An index is rebuilt from the
  tape whenever they disagree and can always be deleted.
- **Chapters** are markers in the tape, not a second thread: one working
  context each, closed on long idle, on request or by the agent, leaving a
  handoff note. Restore in three tiers: the note, reopening the previous
  chapter, full-text search.
- **Honesty about memory.** *Restored* means the agent recalls the
  conversation; *Fresh* means it is reading saved history. Never pretend.
- **Reach, not approval cards, for the built-in agent.** A teammate's one
  policy is binary: its working directory is a wall, or the whole machine is
  open. An agent is there to go and do things.
- **The tool ledger.** Every tool a teammate was given, where it came from,
  and for anything absent, why. A tool that vanishes silently is the worst
  failure the old app shipped.
- **Quiet scheduled runs by construction**, not by asking the model to be
  quiet: a window over the run demotes its words to thoughts by event kind.
- **The house discipline.** Headless harnesses drive the real thing end to
  end; commits are one line stating an invariant; delete, don't disable;
  fewer moving parts beats fewer lines; write for the next reader.

## What is built differently, and why

### 1. One log, not three stores

Toad today keeps the roster in `store.sqlite` with an oplog beside it,
tapes as JSONL, settings as JSON, peer threads as more JSONL, and replicates
records and tapes by two different mechanisms. Here everything the room
remembers is an event on a **stream**, and a stream is an append-only JSONL
file folded by id, exactly the tape's model:

| Stream | Holds | One per |
| --- | --- | --- |
| `room` | teammates, settings, schedules, credential *metadata*, devices | room |
| `tape/<teammate>` | the conversation: messages, tools, chapters, notices | teammate |
| `thread/<a~b>` | a conversation between two teammates | pair |

The roster is a fold over `room`. Settings are a fold over `room`. A delete
is a tombstone event. Secrets are never events: they live in the vault (a
`0600` file in a `0700` directory) and the log holds only the fact that a
credential exists. Replication, when it returns, is shipping a stream's
bytes, which the tape's segment model already does.

What breaks without it: every stored thing needs its own writer, its own
reader, its own change notification and its own replication story. With it,
there is one writer per stream, one fold, one subscription and one shipping
mechanism.

Tapes keep today's event shape and file layout (`transcripts/<id>/<epoch>.jsonl`),
so importing an existing Toad data directory copies them unchanged. The
importer reads a data directory the shipping Toad is still using and never
writes into it: George runs the old app daily until Phase 1 replaces it, and
this tree becomes the `toad` repository when it does.

### 2. A wire of commands and subscriptions

Toad's contract is 135 request methods and 14 pushes, gated per method per
seat. Here the wire is one WebSocket carrying three things:

- **Commands**: `{id, cmd, params}` answered by `{id, ok, result|error}`.
  Commands change something or ask a question the log cannot answer
  (search, a model list from a provider).
- **Stream subscriptions**: `{id, sub}` naming a stream, answered by that
  stream's whole fold as one snapshot and then every event that lands after
  it, live. The transcript view is a subscription to a tape. Ephemeral frames
  that are not events, such as streaming deltas, ride the same subscription
  marked as such and are never written.
- **View subscriptions**: a few materialised views the core maintains and
  nobody logs, because they are derived: `roster` (each teammate with its
  last line, that line's ts, the live tool title while thinking, and live
  session state). Unread is the window's: it remembers the latest ts it
  has shown per teammate. A view is a snapshot followed by updates.

A **seat** is what a socket may do: the desk seat (the window) may do
everything; a phone seat may command its own device's things and subscribe
to what it is shown; later seats (peer, client) are the same mechanism with
smaller sets. There is no per-method routing table; a seat is a set.

Types are defined once in Rust with serde and the TypeScript is generated
(ts-rs), so the window and the core cannot drift. `toad_core::contract`
already holds the room's types from the migration and moves over as is.

### 3. One session kind, two drivers

Today Toad Agent and ACP backends are two implementations of a hundred-line
seam, and every feature is decided twice. Here there is one `Session` with
the supervisor's rules in it once — reply stamping, reaction notes, hop
notices, the quiet window, chapters' gate, receipts, the ledger — and a
`Driver` beneath it that speaks ACP semantics:

- `Driver::InProcess`: Toad Agent on Rig, in this process, owning its tools
  and its loop. The Rig turn loop from the migration moves over as is.
- `Driver::Child`: an external harness over the Agent Client Protocol on
  Zed's `agent-client-protocol` crate, a child on tokio.

The session vocabulary is ACP's: a prompt is content blocks, an update is a
session update, a permission is a request with options. The in-process
driver produces the same updates from Rig's stream. A driver has no idea
what a tape is.

### 4. Tools as MCP, served in-process

Toad's own teammate tools (search the thread, chapters, message a teammate,
react, ring, schedule, ask the human) are one MCP server hosted in-process
on `rmcp`. Toad Agent gets its workspace tools natively plus every MCP
server the teammate's policy grants, connected by Toad as the client. A
child driver is handed the same list, Toad's server included, in the form
ACP takes. One tool surface, one policy, one ledger.

### 5. No fleet in the first version

The mesh, admission, membership, replication and hop are a quarter of Toad
and the least felt in daily use. They are not ported. When moving a
teammate between machines earns its place, it is shipping its streams and
starting the session elsewhere, designed then on the log.

### 6. One process, one language, a thin shell

`toad-core` is a Rust library with no Tauri dependency and every behaviour
in it. `toad-desktop` is the Tauri 2 shell: the window, the desk door, the
menus. The window is built fresh in Phase 1 against the generated contract
and the subscription wire; `../toad/src/mainview` is the reference for
which screens exist and how they behave, and a component is lifted from it
only where that is cheaper than writing it. Bun and Node exist only as the
UI's build tools; nothing runs on them.

Providers are Rig providers: API keys first (Anthropic, OpenAI, OpenRouter),
subscription logins later as custom providers.

## Module map

```
crates/toad-core/src/
  contract.rs      the wire's types (serde + ts-rs)
  paths.rs         the data directory's layout
  log/             streams: append, fold, segments, subscribe
  store/           FTS5 over tapes and chapters; chapter list; previews
  room.rs          the roster fold and settings
  vault.rs         secrets beside the room stream
  session/         Session, the funnel, quiet, chapters
  driver/          InProcess (Rig)
  tools/           workspace tools on cap-std, shell command
  desk.rs          the room, vault and log behind the wire
  import.rs        copies an existing Toad data directory
  wire/            the door: seats, commands, subscriptions
  bin/toad-import.rs  the importer as a binary
crates/toad-desktop/   the Tauri shell
ui/                    the window (React, built against the generated contract)
crates/toad-core/tests/  headless end-to-end proofs (integration tests driving the wire)
```

## Phases

Each phase ends with the app usable for what it covers, `make check`
green, and a harness proving the phase's behaviour over the wire. Nothing
needs to be live between phases; `../toad` on `main` stays the daily driver
until Phase 1 replaces it.

- **Phase 0 — the core, headless.** The log with folds and subscriptions;
  the room stream with teammates and settings; the vault; the index; the
  moved contract, tools and Rig loop; the wire with the desk seat. Proof: a
  harness creates a teammate, runs a Toad Agent turn with a real key, and
  watches the tape and the roster view over the wire.
- **Phase 1 — the room you can live in.** Session funnel, chapters and the
  summariser, search, the Tauri shell, the React window on the new wire,
  the importer for an existing Toad data directory. Exit: George daily-drives
  it for Toad Agent teammates.
- **Phase 2 — the other driver and the tools.** ACP child driver; rmcp
  client and Toad's own MCP server; the ledger; MCP settings and OAuth.
  Exit: Cursor and Claude Code teammates work; the tool ledger reads true.
- **Phase 3 — the room's edges.** Peer threads and receipts, the scheduler
  with quiet runs, attachments, notifications, the phone seat with pairing
  and push. Exit: the phone joins the room.
- **Later, each when it earns its place**: the fleet as stream shipping,
  the computer, the client seat, subscription logins, updater and signing,
  sandboxed commands for workspace reach.

## Not in scope

The iOS app is untouched until Phase 3 gives it a door. The old repository
is read-only reference and is not modified by work here.
