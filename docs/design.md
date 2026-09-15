# Toad, from the ground up — the design record

Written 2026-09-01, the day George chose a fresh tree over the strangler-fig
migration of `../toad`. This is the decision record: what is kept from Toad
as it exists, what is built differently, and why. It is the document every
phase is checked against. The product story is the README; the contract for
agents changing this tree is `AGENTS.md`; the method for changing what a
teammate can reach, and the index of the tests that prove each boundary, is
[security.md](security.md).

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
  open. An agent is there to go and do things. On Linux and macOS the built-in shell
  exposes selected host toolchains read-only, without exposing other host data;
  its persistent home lives inside the workspace. Network access and granted
  integrations remain separate capabilities (see [sessions](sessions.md)).
  macOS uses default-deny Seatbelt with an enforcement probe and private TMPDIR,
  without claiming Linux mount/PID namespace parity. Its deprecated launcher
  and undocumented policy language require platform regression testing.
  Reach, the gateway, collaboration, background work and the computer are
  independent standing grants, revoked through one capability lease; the
  method and the regression matrix are [security.md](security.md).
- **Collaboration crosses a capability boundary by explicit direction.** A
  Whole machine Toad Agent may ask a colleague without another card. A
  workspace caller first asks the operator for a recipient-specific session
  grant or a standing sender grant; the latter is stored on the recipient by
  stable sender id. ACP mode and Computer access do not widen reach. Grants
  expire or revoke with their sessions and never imply reverse or transitive
  authority.
- **The tool ledger.** Every tool a teammate was given, where it came from,
  and for anything absent, why. A tool that vanishes silently is the worst
  failure the old app shipped.
- **Quiet scheduled runs by construction**, not by asking the model to be
  quiet: a window over the run demotes its words to thoughts by event kind.
- **Persistent work is a grant.** Background work defaults off for teammates.
  It authorizes their own schedules and loops; jobs the person creates in the
  desk carry explicit operator provenance. Revocation pauses agent-created
  jobs without deleting them. Own conversation memory stays available, while
  the built-in schedule listing cannot reveal another teammate's prompts.
- **External harnesses own their permissions.** Choosing ACP is explicit
  trust in that harness. Its runtime mode lives in the Reach card, while
  model and effort stay in the chat header. Toad-mediated file callbacks
  remain confined to the workspace; a harness mode does not widen them.
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
is a tombstone event. Secrets are never events: Toad-owned provider and MCP values live in the
operating system credential store; the vault files hold opaque references.
Rig-owned ChatGPT and Copilot OAuth caches remain permission-restricted files.
The log holds only credential metadata. Replication, when it returns, is shipping a stream's
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

Settings → Remote owns an opt-in TLS listener, separate from the desk's loopback
door. Its default is all host IPs, with a live option to restrict it to one local
IPv4 or IPv6 address. Loopback, wildcard address literals, and scoped link-local
addresses are not choices; old loopback settings migrate to all host IPs. The
all-address listener accepts IPv4 and IPv6, falling back to IPv4 on hosts without
IPv6 support. The active route is preferred for the pairing QR.

A two-minute QR invitation carries the certificate fingerprint; its claim grants
one phone a bearer credential. The TLS key lives in the OS credential store, and
only credential hashes and device metadata live on disk. A new certificate covers
all advertised host IPs. The certificate, port, and grants are reused when the
selected addresses are already covered, including toggling Remote off and on.
Enabling an address absent from that certificate replaces it and requires pairing
again. A phone using an address excluded by a new restriction also needs a fresh
pairing QR. Disable and revoke close the affected sockets immediately.

Manual pairing is the same two-minute session for a phone that cannot scan: the
panel shows the address, port, and a six-digit code next to the QR. Six digits
cannot carry a certificate fingerprint, so the code is never a bearer secret; it
is the password of a CPace-shaped PAKE over ristretto255 (`remote/pake.rs`
spells every byte). `POST /pair/manual/start` carries the phone's share and
returns the desktop's; `POST /pair/manual/finish` carries a confirmation whose
key binds the certificate the phone actually connected to, so a relay with its
own certificate fails on both sides. A wrong guess is one online guess, the
fifth kills the code, and the first phone through either path closes the other.
A fresh install listens on 8788 so a typed address can be a bare host; a busy
port falls back to an ephemeral one that is saved and reused.

The first phone seat reads the roster and a bounded recent tape window, sends
text and uploaded attachments through `mobile.prompt`, and cancels a response.
It cannot administer the room or answer approvals. A mobile prompt starts the teammate's session
when needed and uses the same core prompt path as the window. Its device-scoped
operation UUID has a durable receipt before execution: an identical retry returns
the recorded result, and an interrupted acceptance returns unknown rather than
executing twice. The companion is a separate Expo repository, `../toad-mobile`.
It reconnects by subscribing to fresh snapshots; commands never replay
automatically. Attachments upload in repeatable 32 KiB chunks over the same socket,
with at most four 10 MiB files per prompt. Only completed device-scoped upload IDs
resolve to paths in the core; a phone never supplies a desktop path. Declared sizes
reserve space within a 256 MiB / 1,024-file limit per device, including abandoned
uploads. Files remain because tape attachments refer to their paths; retention
cleanup, push, offline history, and mobile approval actions are later work.

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
  and its loop. A shared loop makes ordinary Rig model requests and admits
  operator steering between them, independently of the selected provider.
  Owned shell jobs outlive individual model requests; waits yield to operator
  input, and cancellation observes exit before reporting completion.
- `Driver::Child`: an external harness over the Agent Client Protocol on
  Zed's `agent-client-protocol` crate, a child on tokio.

The built-in loop classifies provider failures before rendering them. Transient
failures retry only inference from committed in-memory history, up to three
retries with backoff, jitter and bounded provider retry hints. Completed tool
calls are not dispatched again. Stop and revocation remain terminal; input
admitted but not consumed when a failure closes admission returns to the session
queue. Quota, authentication and configuration failures require intervention.
Explicit replay failures get one fresh continuation from execution facts;
model switches also reset opaque provider state. Neither path rebuilds an
automatic retry from the tape's text-only history. Oversized execution records
remain in local tool-output files, with a bounded excerpt and path in context.
The earlier global colon-ID filter is removed. Native reasoning IDs, encrypted
Responses data, Anthropic signatures and Gemini signatures survive normal
continuations unchanged. A backend rejection of a Responses input ID or explicit
signature/replay error instead enters the fresh boundary. This deliberately
drops opaque state as a unit, with a notice; clearing a Responses reasoning ID
alone would silently make Rig omit its encrypted content too.

Known model context limits trigger chapter rotation at a conservative threshold
before the next inference, including within a long tool turn. The session drains
prior updates before closing the chapter and returning its wake note. Unknown
limits remain unknown; a provider context-limit refusal can request the same
boundary once. A continuation that still cannot fit fails explicitly. ACP owns
its own internal request loop and context management; Toad does not interrupt an
opaque child turn using a guessed token count.

An ACP prompt failure never automatically reissues that prompt. The next operator
message replaces the failed child only after the old process exits, opens a fresh
session with the same granted servers and disposition, and supplies a briefing
that distinguishes uncertain execution from completed work. Failed checkpoints
are withdrawn. A failed first briefing is not treated as a successful restoration.

Error cards use sanitized structured details carried inside the existing notice
text, after a plain-language title and summary. The JSONL event shape, segment
layout and importer remain byte-compatible; older clients can still read the
notice. The desktop expands details on demand, including plain-text legacy
errors. Image normalization and provider research are in [image input](image-input.md).

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

The global MCP configuration is the operator's gateway. New teammates get
no gateway servers; the operator grants selected servers or all servers on
each teammate. All includes servers added later. A saved choice is preserved,
including on import; a missing or invalid imported policy grants nothing.
Reach governs local workspace tools, while an MCP grant authorizes that
server's own capabilities and permissions. The shell sandbox does not confine
granted servers. These are standing choices, without per-call approval cards.

Teammate collaboration uses the same capability boundary. A Whole machine
Toad Agent has implicit authority to ask another teammate to work. A workspace
caller gets only public teammate names and ids from discovery, then
needs a first-contact operator decision for each direction. The card names the
caller and recipient and offers a session grant, a standing sender grant, or
denial. A standing grant is an `allowedSenders` id on the recipient; it
survives rename and restart, while a session grant is tied to both live
capability leases and expires on session or chapter replacement. Revocation
clears waits, queued work and cached peer sessions. An authorized reply does
not grant the reverse direction or any third party.

HTTP MCP servers may opt into OAuth 2.1 in the gateway. Toad follows
protected-resource and authorization-server metadata, requires authorization
code plus PKCE S256, and uses the advertised DCR endpoint when no saved or
configured client exists. Credentials and registration secrets stay in the
private vault, bound to the server URL and issuer. Toad Agent uses rmcp's
refreshing client; ACP receives a capability checked loopback proxy so its
child process never sees OAuth tokens. Operator sign-in does not alter a
teammate's MCP grant, and sign-out invalidates live sessions before clearing
the vault record.

### 5. No fleet in the first version

The mesh, admission, membership, replication and hop are a quarter of Toad
and the least felt in daily use. They are not ported. When moving a
teammate between machines earns its place, it is shipping its streams and
starting the session elsewhere, designed then on the log.

### 6. One process, one language, a thin shell

`toad-core` is a Rust library with no Tauri dependency and every behaviour
in it. `toad-desktop` is the Tauri 2 shell: the window, the desk door, the
menus. The window remembers its size, place and maximised state, and posts
a desktop toast when a teammate finishes or blocks while the window is not
focused — the first through a plugin, the second through a plugin on Linux
and Windows and through the notification center on macOS, where a click
comes back as an event and the toasts thread by teammate; the judgement is
the page's either way. The window
is built fresh in Phase 1 against the generated contract and the
subscription wire; `../toad/src/mainview` is the reference for which
screens exist and how they behave, and a component is lifted from it only
where that is cheaper than writing it. Bun and Node exist only as the UI's
build tools; nothing runs on them.

Providers use Rig's native clients for inference. Rig also owns ChatGPT and
Copilot login and refresh. OpenRouter offers pasted keys and browser PKCE
sign-in; Toad exchanges the code for a private API key and hands that key to
Rig's OpenRouter client. Ollama Local takes a server URL, while Ollama Cloud
takes an API key for `https://ollama.com`. Both use Rig's Ollama client for
native chat and model discovery. A discovered list belongs to its connection,
so replacing a server or account cannot reuse another connection's list.
Grok offers subscription device sign-in alongside xAI API keys. Toad uses
the OAuth device flow and a private token store; a shared refresh lock keeps
teammates from spending the same rotated refresh token. Rig's xAI client
still owns inference, with a bearer-refresh HTTP client that retries a 401
once and never falls back to an API key. Z.ai Standard and Coding Plan are
separate API-key connections using Rig's native Z.ai endpoints.
Custom OpenAI-compatible connections have stable, separate provider ids, a
name, base URL, optional key, API choice, and an editable model list. Rig's
native OpenAI Responses and Chat Completions clients own inference and model
discovery. A small Rig provider builder makes bearer authentication optional;
no custom inference transport is added. Keys are privately bound to the exact
endpoint, so editing a URL requires explicitly supplying or removing its key.
Discovery fills the form without changing a saved connection until Save.
Provider discovery supplies current model IDs wherever Rig has a supported
listing client. The bundled models.dev snapshot enriches exact provider/model
matches with metadata; it does not gate newly discovered IDs. Manual model
IDs belong to the existing connection and survive refreshes and releases.
Unknown metadata remains unknown. Refresh changes the available list, never
the selected model, connection endpoint, or credentials. Persisted connection
data is separate from the application's bundled snapshot.
Claude subscription access stays with Claude Code through ACP.

Application updates follow Prism's Tauri updater: a six-hour GitHub check,
brief notes in Settings, and user-triggered signed download, install and restart.
Installer-specific manifest keys preserve the installed package format. The
core has no Tauri dependency; it provides an idle restart lease so the desktop
cannot discard queued work or race a new turn. A held lease pauses admission
and leaves due schedules durable; dropping it after failure resumes the room.
User data stays outside replaced application assets. Development builds never
check or install updates.

### 7. Skills, as files every harness reads

A skill is a procedure a teammate reads when the task calls for it: how to
work a computer, how to cut a release, the thing the person asked for twice
last week. It is the Agent Skills format: a directory named for the skill,
holding `SKILL.md` with `name` and `description` frontmatter and a Markdown
body, and whatever `scripts/`, `references/` and `assets/` the body points
at. The directory is `.agents/skills/<name>/`, the convention Codex already
scans; Toad invents no harness directory of its own until one earns its
place. `name` matches its directory, is lowercase with hyphens and at most
64 characters; `description` is at most 1024 and says *when* to use the
skill, because it is all an agent sees before deciding to read the body. A
folder that breaks those rules is listed as invalid with the reason, never
silently skipped.

A skill has three sources and one channel. **Built-in** skills are bundled
in `toad-core` under `skills/` and are always on: Toad's own procedures for
its room and, when a teammate has one, its computer. The **gateway** is the
operator's folder, `skills/` in the data directory, granted per teammate
with the same none / some / all policy MCP servers use; a new teammate gets
none, and all includes skills added later. **Workspace** skills are whatever
is in the teammate's own `.agents/skills`, put there by the person or by the
teammate itself. The channel is the workspace: on session start and on a
grant change, Toad copies the built-ins and the granted gateway skills into
`<cwd>/.agents/skills/`, so Toad Agent reads them with its workspace tools
inside its reach and an ACP child reads them with its own. Copies, not
links, because a workspace mounted into a computer has to carry them. What
Toad copied it marks, and the `AGENTS.md` rule applies: Toad replaces and
removes only an entry carrying its marker, so a skill the person or the
teammate wrote is never touched, and revoking a grant leaves nothing of
Toad's behind.

The preamble carries the index — each skill's name, description and path —
and nothing else about skills. That is the progressive disclosure the format
asks for, and it rides the block both drivers already hear, so a harness that
does not scan `.agents/skills` itself still knows what is there and reads the
file it is pointed at; Codex scans the directory natively and hears it twice,
which is harmless. A skill is something the agent decides to read, so nothing
a teammate must know before its first word is a skill: identity, reach, the
date, the names of Toad's own tools and the house style stay in the preamble
as sentences. Built-in skills hold procedure, not standing. The first ones
are the room's workflows — chapters, schedules, asking a colleague, asking
the person — and the habit of keeping a skill for anything the teammate will
be asked for again, offering to write one before repeating work.

The computer's guide is a skill with a fourth provenance and no folder of its
own. The running container serves it with its release and checksum; Toad
lists it under the teammate as `toad-computer` at that version, copies it into
the workspace like a grant, and refreshes it when the container's checksum
changes. It is never in the gateway, because the right guide is the one the
running release ships, not one the operator keeps.

Not yet: gateway sources from git or a registry, promoting a workspace skill
to the gateway from the pane, and honouring `allowed-tools` — a skill runs
with the teammate's reach and grants, no more.

## Module map

```
crates/toad-core/src/
  contract.rs            the wire's types (serde + ts-rs)
  paths.rs               the data directory's layout
  log/                   streams: append, fold, segments, subscribe
  store/                 FTS5 over tapes and chapters; chapter list; previews
  room.rs                the roster fold, settings, and schedules
  vault.rs               secrets beside the room stream
  session/               Session, the funnel, quiet, chapters, the scheduler, the ledger, peer threads
  driver/                InProcess (Rig) and the ACP child, with its agent registry
  mcp/                   the client of granted servers, and Toad's own teammate tools
  skills/                the catalog: built-in, gateway and workspace skills, copied into a workspace
  tools/                 workspace tools on cap-std, shell command
  desk.rs                the room, vault and log behind the wire
  import.rs              copies an existing Toad data directory
  import/                the previous Toad's roster and records, read-only
  wire/                  the door: seats, commands, subscriptions
  bin/toad-import.rs     the importer as a binary
  bin/toad-mcp-echo.rs   a one-tool stdio server the MCP harnesses spawn
crates/toad-desktop/     the Tauri shell: plugins, the menu, the door
ui/                      the window (React, built against the generated contract; ui/src/ui is the design system)
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
  the computer, the client seat, subscription logins, updater and signing.

## Not in scope

The iOS app is untouched until Phase 3 gives it a door. The old repository
is read-only reference and is not modified by work here.
