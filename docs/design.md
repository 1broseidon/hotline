# Hotline, from the ground up — the design record

Written 2026-09-01, the day George chose a fresh tree over the strangler-fig
migration of `../hotline`. This is the decision record: what is kept from Hotline
as it exists, what is built differently, and why. It is the document every
phase is checked against. The product story is the README; the contract for
agents changing this tree is `AGENTS.md`; the method for changing what a
teammate can reach, and the index of the tests that prove each boundary, is
[security.md](security.md).

Single decisions too large for a section here are recorded as ADRs in
`docs/adr/`:

- [ADR 0001](adr/0001-threads-are-a-primitive.md): threads are a primitive.
- [ADR 0002](adr/0002-a-served-desk-relays-sealed-records.md): a served desk
  relays sealed records.
- [ADR 0003](adr/0003-linked-desks.md): linked desks trust each other's
  keys, not each other's owners.
- [ADR 0004](adr/0004-live-calls-get-a-voice-front.md): live calls get a
  voice front.
- [ADR 0005](adr/0005-front-is-a-full-agent.md): the live voice is a full
  agent on the call.
- [ADR 0006](adr/0006-relay-only-servers.md): a server can just relay, and a
  desktop can pair with it only to relay.
- [ADR 0007](adr/0007-split-hotline-core.md): split hotline-core into
  crates, leaves first.
- [ADR 0008](adr/0008-one-agent-thread-layer.md): one agent thread layer;
  modalities attach to it, and the chat is the log of truth (proposed).

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
- **Extra folders widen the wall without removing it.** A teammate keeps one
  working directory and may be given a short list of other folders, each
  read-only unless the person said it may be changed. Under workspace reach
  they are holes in the wall cut by the person, enforced by the same cap-std
  handles and shell sandbox as the workspace; they are not a second policy
  axis to tune per tool, and there is no per-path ACL beyond read or change.
  Whole-machine reach makes them moot. A harness is handed them as ACP
  `additionalDirectories` when it says it takes them and enforces its own
  reach; Hotline's callbacks for it honour them. A computer mounts them.
  Old records carry none; nothing older is translated into one.
- **Collaboration crosses a capability boundary by explicit direction.** A
  Whole machine Hotline Agent may ask a colleague without another card. A
  workspace caller first asks the operator for a recipient-specific session
  grant or a standing sender grant; the latter is stored on the recipient by
  stable sender id. ACP mode and Computer access do not widen reach. Grants
  expire or revoke with their sessions and never imply reverse or transitive
  authority.
- **The tool ledger.** Every tool a teammate was given, where it came from,
  and for anything absent, why. A tool that vanishes silently is the worst
  failure the old app shipped.
- **Jobs are subagents on a schedule** ([ADR 0008](adr/0008-one-agent-thread-layer.md)
  §7). One schema, `session::Brief` (the task, now / at / every, the report
  policy, an optional end), is written by the `subagent`, `schedule` and
  `loop` tools. Each job is an arm with a thread of its own
  (`ThreadKind::Job`, `jobs/<id>.jsonl`), every firing is a turn on it and
  never on the home thread, and its agent is parked between firings and
  starts from a running note of its last turns rather than the thread. A
  quiet job is quiet by construction, not by asking the model: its words stay
  on its thread, and only `post_to_chat` reaches the chat. A plain reminder
  is one post and no arm. A subagent and a job are clones of the home agent
  (§0): the teammate on the home session's model, with the home thread's
  permission set and computer under a lease derived from the home thread's,
  narrowed only by what the home agent leaves out (`without`), starting from
  the brief and reaching back with `search_thread` and `list_chapters`.
  `schedule.create` on the wire takes no `without`: a job the person makes
  runs as their teammate, with the reach they already granted it, so the
  person narrows a teammate by changing its grants, or asks it to make the
  job narrower, and the Jobs group shows "Runs without …" either way. A
  wire field with no window or phone to send it would be a second door to
  the same rule with nothing behind it. What came back to a job (a card's
  answer, a colleague's reply) is written on its thread before it is
  queued, so a restart hands it over again, or to the DM once the job has
  ended.
- **One conversation, no machinery in it.** The tape holds what the person
  and the teammate said and did. The room does not narrate its own work: a
  computer downloading, joining or updating, a provider switch, a retry, a
  context rotation or a restart leaves no line. A download shows as a ring
  on the computer's button; the rest goes to the desk's log. What stays is
  a failure the person has to know about (a reply that could not be made)
  and the outcome of something they did themselves.
- **Persistent work is a grant.** Background work defaults off for teammates.
  It authorizes their own schedules and loops; jobs the person creates in the
  desk carry explicit operator provenance. Revocation pauses agent-created
  jobs without deleting them. Own conversation memory stays available, while
  the built-in schedule listing cannot reveal another teammate's prompts.
- **External harnesses own their permissions.** Choosing ACP is explicit
  trust in that harness. Its runtime mode lives in the Reach card, while
  model and effort stay in the chat header. Hotline-mediated file callbacks
  remain confined to the workspace and the teammate's granted folders; a
  harness mode does not widen them.
- **The house discipline.** Headless harnesses drive the real thing end to
  end; commits are one line stating an invariant; delete, don't disable;
  fewer moving parts beats fewer lines; write for the next reader.

## What is built differently, and why

### 1. One log, not three stores

Hotline today keeps the roster in `store.sqlite` with an oplog beside it,
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
is a tombstone event. Secrets are never events: Hotline-owned provider and MCP values live in the
operating system credential store; the vault files hold opaque references.
ChatGPT and Copilot sign-ins use that same vault through Hotline's token
store for Rig. Their login directories hold references; existing plaintext
records migrate on first use, preserving the original on a failed store write.
The log holds only credential metadata. Replication, when it returns, is shipping a stream's
bytes, which the tape's segment model already does.

What breaks without it: every stored thing needs its own writer, its own
reader, its own change notification and its own replication story. With it,
there is one writer per stream, one fold, one subscription and one shipping
mechanism.

Tapes keep today's event shape and file layout (`transcripts/<id>/<epoch>.jsonl`),
so importing an existing Hotline data directory copies them unchanged. The
importer reads a data directory the shipping Hotline is still using and never
writes into it: George runs the old app daily until Phase 1 replaces it, and
this tree becomes the `hotline` repository when it does.

The log owns a shared in-memory fold under its writer lock. The room stays
resident; other streams share a least-recently-read cache of at most 64 streams
and 64 MiB of estimated retained memory, including JSON and id-index overhead.
An oversized stream is read cold. Appends replace an id in its original slot
and update the latest chapter and message positions before publishing. Prompt
and idle checks read that chapter state without copying the tape. Compaction
and migrations invalidate affected folds; restart rebuilds them from disk.
Unterminated files and partial reads are not retained. Failure-sensitive and
strict reads still inspect disk, so caching cannot turn a storage error into
an authoritative answer. Only the log writes streams during its lifetime;
imports finish before it opens. Event shapes and segment layout are unchanged. Model listing
resolves vault connections and their discovery/manual records once per request,
using one room snapshot for credentials and enabled models. No secret survives
in a listing cache. Thread listing scans the side and pair directories once,
reads each selected tape once, and asks the shared fold for thread links without
copying unrelated side histories. Pair working state comes from live sessions,
not from rebuilding the peer history list.

### 2. A wire of commands and subscriptions

Hotline's contract is 135 request methods and 14 pushes, gated per method per
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

The live roster keeps a small projection per watched tape: the last visible
message, the current tool title and pending card ids. Live work and peer
sessions likewise keep their pending-card projection for their own lifetime.
Log seeds these from the folded stream and hands over a broadcast under the
same writer lock; a lagged receiver replaces both together. This is derived
state, not another folded-history cache: ordinary tool and session updates
read no tape tails or peer/side files. The cold tape seed also reads its raw
tail for tool activity, whose append order differs from folded id order;
this prevents an overlapping completed tool from resurrecting an older title
on restart. Message and card recovery is not limited by the old tail windows.

Clients declaring `rosterOptions` keep `session.models` and
`session.slashCommands` when a row omits them. Every snapshot and new row
includes both; later rows send each only when it changes, including an empty
array to clear it. Other fields still replace the previous row. Older clients
continue receiving complete rows, and the window restores the two lists in
its wire layer before delivering rows to its callers.

The socket reader dispatches independent reads concurrently with global
mutations. Global writes, `ping` and subscription controls keep their arrival
order in a worker; commands naming a teammate keep that teammate's lane.
Reads observe committed state when they run, so a client needing a prior
write waits for its reply first. Negotiation stays on the reader, and neither
command work nor ordering waits suspend cancellation or the silence timer.
Remote work is cancelled with its seat; accepted desktop writes finish.
A seat whose frame waits past the write deadline resets its TCP transport,
discarding queued bytes. Only that timeout selects zero linger: ordinary
WebSocket closes must drain final pairing and authentication replies, including
on Windows.

A remote socket's 256-frame queue sheds tool updates and live deltas before
disconnecting. Pending roster rows coalesce per subscription and teammate,
carrying forward any omitted catalogs; required frames are ordering barriers.
Chat, cards, snapshots and replies can evict tool chatter, but an outbox full
of required frames still disconnects. The log keeps every tool event. After
stream lag, a phone receives its full 200-line window within the existing byte
budget, even if it opened with a smaller background window. The fold and a
new broadcast receiver are captured under the log's writer lock, so older
queued revisions cannot undo that snapshot.

A **seat** is what a socket may do: the desk seat (the window) may do
everything; a paired owner has the same command and subscription set. A
companion keeps the limited phone seat. A linked desk's channel has the link
seat, which is its roster, its messages and nothing else ([Linked
desks](#linked-desks)). The stored grant determines authority, not whether
the client is a phone or desktop. There is no per-method routing table; a seat is a set.

Settings → Remote owns an opt-in TLS listener, separate from the desk's loopback
door. Its default is all host IPs, with a live option to restrict it to one local
IPv4 or IPv6 address. Loopback, wildcard address literals, and scoped link-local
addresses are not choices; old loopback settings migrate to all host IPs. The
all-address listener accepts IPv4 and IPv6, falling back to IPv4 on hosts without
IPv6 support. The active route is preferred for the pairing QR.

Settings → Remote has one **Link a phone** action, showing a two-minute,
single-use sealed QR and a copyable pairing link. It grants the phone's public
key, never a bearer credential. The TLS key lives in the OS credential store;
device public keys, roles and metadata live on disk. A new certificate covers
all advertised host IPs. The certificate, port, and grants are reused when the
selected addresses are already covered, including toggling Remote off and on.
Enabling an address absent from that certificate replaces it, and so does a
certificate the desk can no longer read: corrupt, gone from the OS store, or
written by an earlier edition into a store this build cannot name. A store
that is merely locked or unavailable is reported, not replaced. A phone using
an address excluded by a new restriction needs a reachable address from a
fresh pairing QR. TLS rotation retains grants and phones keep trusting the
independent desk identity. Disable and revoke close the affected sockets
immediately.

A served desk instead requires one explicit fixed listen address and public
HTTPS URL. It retries that exact socket if the address is late, never widening
or falling back to an ephemeral port. Its only phone routes use Noise IK inside
TLS WebSockets, authenticated by a persistent X25519 desk identity in the
selected SecretStore and the phone's granted public key. TLS certificates can
rotate without changing this trust. Desktop and served desks use the same
sealed pairing and session protocol.

`hotline pair` opens a two-minute, atomic single-use QR invitation and keeps
it open only while its local desk-seat connection lives. No empty grant list
bootstraps an owner. Owner grants carry full desk authority; companion grants
retain the limited phone allowlist. `--json` and `--link` expose the same
invitation for desktop clients. Remote administration uses owner and local
desk wire commands, shared by the window and CLI, not Tauri wrappers.
Neither listener mounts manual or legacy bearer routes. Stored grants with
only a token hash and no device public key cannot authenticate or receive
push notifications. They stay listed as needing a re-pair, so the operator can
revoke them. Phones paired with the six-digit code before 0.33 scan the new QR
once; existing sealed grants keep working.

All sealed application frames, including computer traffic, travel inside
Noise. A viewer's authenticated handshake binds both its purpose and persona,
so an untrusted TLS proxy cannot redirect controls by rewriting the HTTP path.
The desk alone resolves the viewer address and supplies its bearer. The proxy
still sees paths, timing and sizes and can deny service. Protocol and limits
are in `docs/wire.md`; listener setup is in `docs/serve.md`.

A desktop may also keep a Public address: the https origin of a tunnel,
proxy or forwarded port that reaches its listener. It is one saved value,
read by both the pairing QR (as `x`, tried after the network address `u`)
and the hello (as `publicEndpoint`), so a phone away from home pairs through
it and then knows which of the desk's endpoints it is. The Remote settings
hint points the tunnel at `localhost` on all host IPs, and at the one
address and port when Remote listens on a single address.

A phone that cannot scan opens the pairing link instead. There is no typed
short code. A fresh desktop install listens on 8788; a busy port falls back to
an ephemeral one that is saved and reused.

The first phone seat reads the roster and a bounded recent tape window, sends
text and uploaded attachments through `mobile.prompt`, and cancels a response.
It cannot administer the room or answer approvals. A mobile prompt starts the teammate's session
when needed and uses the same core prompt path as the window. Its device-scoped
operation UUID has a durable receipt before execution: an identical retry returns
the recorded result, and an interrupted acceptance returns unknown rather than
executing twice. The companion is a separate Expo repository, `../hotline-mobile`.
It reconnects by subscribing to fresh snapshots; commands never replay
automatically. Attachments upload in repeatable 32 KiB chunks over the same socket,
with at most four 10 MiB files per prompt. Only completed device-scoped upload IDs
resolve to paths in the core; a phone never supplies a desktop path. Declared sizes
reserve space within a 256 MiB / 1,024-file limit per device, including abandoned
uploads. Files remain because tape attachments refer to their paths; retention
cleanup, push, offline history, and mobile approval actions are later work.

`file.read {personaId, eventId, index?, offset?}` reads a teammate's kept file
or a user's retained image, never a caller-supplied path. `index` defaults to
zero and names the original position in `attachments`, including non-images;
a user non-image or out-of-range position is refused, and a teammate file only
accepts zero. Chunks contain at most 512 KiB of original bytes as base64, with
`name`, byte-sniffed `mimeType`, the whole `size`, `offset`, and `next` only
when more bytes remain. Files over 25 MiB are refused before any chunk is sent.

At prompt time the core keeps byte-for-byte copies of user PNG, JPEG, GIF and
WebP attachments under `files/<teammate>/<message>/user/<index>`. This is a
readback copy, not model image preparation: nothing is transcoded and the
original attachment fields and driver input remain unchanged. Completed copies
are published without overwriting, before the user event; source changes or
deletion afterwards cannot change what the phone reads. A source unavailable,
unsupported, changing during capture or over the cap is not retained, but the
prompt still goes through. Readback then says it has no retained copy. Legacy
user events also fail closed: their recorded path, size and MIME cannot prove
which bytes were sent, so the core never retroactively snapshots or reopens
those paths. Resending makes a new readback copy. No tape shape or importer
changes are needed. Copies share the existing kept-file retention policy (no
automatic cleanup); they are additional storage, including for phone uploads.

Types are defined once in Rust with serde and the TypeScript is generated
(ts-rs), so the window and the core cannot drift. They are all in the
`hotline-contract` crate, which depends on no other Hotline crate and which
`hotline-core` re-exports as `contract`;
[development.md](development.md#the-generated-contract) says why none may
live anywhere else.

### 3. One session kind, two drivers

Today Hotline Agent and ACP backends are two implementations of a hundred-line
seam, and every feature is decided twice. Here there is one `Session` with
the supervisor's rules in it once — reply stamping, reaction notes, hop
notices, chapters' gate, receipts, the ledger — and a
`Driver` beneath it that speaks ACP semantics:

- `Driver::InProcess`: Hotline Agent on Rig, in this process, owning its tools
  and its loop. A shared loop makes ordinary Rig model requests and admits
  operator steering between them, independently of the selected provider.
  Owned shell jobs outlive individual model requests; waits yield to operator
  input, and cancellation observes exit before reporting completion. A
  command still running when the model replies without calling `wait_jobs`
  is handed to the DM session as a running process and the turn ends: there
  is no flag and no timer, because the model's choice not to wait is the
  signal. A process lives as long as the session's driver, counts toward the
  teammate's four commands, is inspected or cancelled from any later turn,
  and when it ends on its own its result wakes the teammate once, as a line
  of Hotline's queued like any other, said on a live call to the teammate
  as well as written. A stop the teammate or the person asked for wakes
  nobody. The desk shows each beside the session's state, never as part of
  it (`SessionInfo.processes`, sent when the set changes), with its output
  tail read on demand (`process.output`) and a Stop (`process.stop`).
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

Known model context limits trigger a fresh continuation at a conservative
threshold before the next inference, including within a long tool turn. The
driver rebuilds its history from the turn's committed facts under the same
preamble and wake; the chapter stays open, with its collaboration authority, so
a long handoff reads as one chapter. Chapters close only when idle, when the
agent asks (`new_chapter`) or when the person starts fresh. Unknown
limits remain unknown; a provider context-limit refusal can request the same
boundary once. A continuation that still cannot fit fails explicitly. ACP owns
its own internal request loop and context management; Hotline does not interrupt an
opaque child turn using a guessed token count.

The loop writes each round's request itself (`turn::round_request`), so a
turn sends the same settings an agent's answer would (`build::RequestSettings`:
the output ceiling, the effort, the cache it asks for). Rig keeps the system
prompt as a request's first message, and the loop rebuilds each round's
conversation from its history, so the prompt is put back on every round
rather than carried from the first.

Claude over the API caches nothing unless a request asks, and every round of a
turn resends the tools, the preamble and the history. A turn therefore asks: the
tools and the preamble carry cache markers, and the request's cache is a
top-level breakpoint that moves forward with the conversation. Claude through
OpenRouter gets that request-level marker and, on its way out, one on the
preamble, which OpenRouter passes on (`build::openrouter_model`). A one-shot
answer such as a chapter's note asks for nothing, because a cache write costs
more than a plain read that nothing follows. Rig counts every provider's cache
reads and writes inside its input tokens, Anthropic's included, so a round's
usage is already whole when it meets the context threshold or the turn's
record, which keeps cache reads and writes as counts of their own; adding
Anthropic's again would price each cached token twice and fill the context
early. That count is `pricing::Tokens::of`, the one place Rig's usage is read,
so turns and the call assistant meter it alike. A response counted at no
tokens at all reported nothing, and keeps what it reserved.

Every Hotline request accepts a finish reason Rig does not know as an ordinary
stop. Rig 0.44 began failing a reply whose finish reason was outside its list,
and marking it never to be sent again. A gateway that passes its upstream's own
reason through would then fail a spoken answer that arrived whole, and a
turn's reply would drop out of every later request while the results of its
tool calls stayed in. Hotline ended a reply on any reason it was not told was
a truncation or a filter before, and still does: a length or content-filter
finish, or a reply the provider says failed, runs none of its tool calls.

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

An ACP harness that reports `AgentAuth` can offer **Sign in** on the desktop
failure card. Initialize retains the harness's supported authentication methods;
the card holds only their names and ids. Agent methods use ACP `authenticate`.
On Linux and macOS, terminal methods run the configured harness with its advertised argument suffix
and environment overrides in a private interactive terminal. They never pass
through `authenticate`, a shell command string, or a conversation draft. The
terminal and its input are transient; the harness, not Hotline's provider vault,
owns the account and credentials. A successful login reopens the agent session
without replaying the failed message. The person can put that message back in
the composer for review; a failed turn may already have performed work.

A turn that failed on the network, at the provider, on a rate limit, in a
harness or for no reason known can be tried again: the failure card on the
latest turn offers **Try again**, which is `session.retry`. The room checks
the tape (the last turn failed and no message came after it) and queues the
same line again as a retry, writing nothing new. Hotline Agent re-runs on
the history it kept, which already ends with the person's line and any reply
shown before the failure, so the model is not asked twice; a history that no
longer ends there, and every harness, is prompted with the same words. A key,
quota, context or request failure fails the same way again, so its card does
not offer it. The driver's own backoff (three tries for transient errors)
has already run by the time the card appears.

This recovery is desktop-only. Claude's `NO_BROWSER` method starts its interactive
CLI, not a structured URL/device-code protocol. Hotline does not scrape that TUI
into a phone login form or relay Claude session tokens. Unsupported methods and
older failure notices keep the manual sign-in guidance. Proactive auth-status
badges are not inferred from missing notifications or offered by this recovery.

### 4. Tools as MCP, served in-process

Hotline's own teammate tools (search the thread, chapters, message a teammate,
react, ring, schedule, ask the human) are one MCP server hosted in-process
on `rmcp`. Hotline Agent gets its workspace tools natively plus every MCP
server the teammate's policy grants, connected by Hotline as the client. A
child driver is handed the same list, Hotline's server included, in the form
ACP takes. One tool surface, one policy, one ledger.

`read_thread(thread, after?, limit?)` reads a teammate's own thread from its
beginning, in folded log order, with full events and an event-id cursor for
following pages (50 events by default, at most 100). A call's thread **is its
transcript**, including the spoken and written versions of replies and each
tool call in its own position. Search hits and posts name the same key and
say this explicitly, so the home agent can read what “like we said on the
call” refers to. Both drivers use the same lease-checked handler; ownership
comes from the thread's parent, or membership for a pair. The window shows
a call's speakers and tools inline, with the written version available when
it differs from what was spoken, and keeps the composer for continuing it.
No tape events or segment layouts change.

The global MCP configuration is the operator's gateway. New teammates get
no gateway servers; the operator grants selected servers or all servers on
each teammate. All includes servers added later. A saved choice is preserved,
including on import; a missing or invalid imported policy grants nothing.
Reach governs local workspace tools, while an MCP grant authorizes that
server's own capabilities and permissions. The shell sandbox does not confine
granted servers. These are standing choices, without per-call approval cards.

Teammate collaboration uses the same capability boundary. A Whole machine
Hotline Agent has implicit authority to ask another teammate to work. A workspace
caller gets only public teammate names and ids from discovery, then
needs a first-contact operator decision for each direction. The card names the
caller and recipient and offers a session grant, a standing sender grant, or
denial. A standing grant is an `allowedSenders` id on the recipient; it
survives rename and restart, while a session grant is tied to both live
capability leases and expires on session or chapter replacement. Revocation
clears waits, queued work and cached peer sessions. An authorized reply does
not grant the reverse direction or any third party.

A card a teammate raised to ask the person is the teammate's to take back.
`request_human` returns the card's id, and `withdraw_request` supersedes the
caller's own pending card as `withdrawn` and delivers nothing; a card that is
neither answered nor withdrawn expires after two hours, or the up-to-a-day
`wait_hours` the teammate chose, and the teammate is told once. A forgotten
card otherwise kept the teammate reading "needs you" on the phone for as long
as it stayed up. The card carries `expiresAt` so the choice survives a
restart. The status and the field are additions to the `human_action` event:
every tape the importer brings over still reads as it did, and a card without
`expiresAt` lives the default from its `ts`.

Whatever a teammate says somewhere other than its DM reaches the DM through
one door, `Room::post` ([ADR 0008](adr/0008-one-agent-thread-layer.md) §3):
a call's summary and its `post_to_chat`, a job's report, final word or
reminder, and every file `send_file` or `generate_image` sends, from any
thread; a job's file is its turn's one post. Each is one `agent` event stamped with where it came from, in
two optional fields: `from` (the thread and its kind, the shape a delivery's
`from` already has) and `scheduled` (the job, the shape a user event's
`scheduled` already has). They replace the `call:<id>:` event id and the
"From the call: " text prefix, which the window now draws from the stamp. Both
are additions to the `agent` event and written only when present: a tape
written before them has neither, the importer copies it unchanged and reads
it as it did, and it reads an agent event that carries them, which the
previous edition never wrote. A post reaches the person as a reply does,
except from a call they are on. The home agent is not woken by a post the person asked for (a call, a work
thread, a job); its live session is handed the post ahead of the person's
next line, and a session that starts later reads it from the tape with where
it came from.

An agent asks for that door with `post_to_chat(text, wake?)`, one of
Hotline's own tools, which Hotline Agent calls in process and an ACP child
over the desk's MCP server, so a quiet job on either driver can be heard. It
is listed in every session but a subagent run's and gated by the handler,
not the model: a work thread, a pair and a job may post, once a turn;
the DM's own turn and a subagent run may not; `wake` hands the post to
the DM on a turn of its own when an arm needs the home agent to act. The
rows are in [security.md](security.md#posting-to-the-chat-from-any-thread-adr-0008).
It replaces the quiet run's `tell_person`, which only Hotline Agent offered,
and the per-turn arming behind it.

HTTP MCP servers may opt into OAuth 2.1 in the gateway. Hotline follows
protected-resource and authorization-server metadata, requires authorization
code plus PKCE S256, and uses the advertised DCR endpoint when no saved or
configured client exists. Credentials and registration secrets stay in the
private vault, bound to the server URL and issuer. Hotline Agent uses rmcp's
refreshing client; ACP receives a capability checked loopback proxy so its
child process never sees OAuth tokens. The page the browser lands on after
sign-in is one escaped card under a Content-Security-Policy that loads
nothing but its inline style, with no script. Operator sign-in does not alter a
teammate's MCP grant, and sign-out invalidates live sessions before clearing
the vault record.

### 5. No fleet in the first version

The mesh, admission, membership, replication and hop are a quarter of Hotline
and the least felt in daily use. They are not ported. When moving a
teammate between machines earns its place, it is shipping its streams and
starting the session elsewhere, designed then on the log.

What did earn its place is narrower: two desks the person links so their
teammates can talk ([Linked desks](#linked-desks)). Nothing is replicated
and nobody moves; each desk keeps its own teammates, and a message crosses.

### 6. One process, one language, a thin shell

`hotline-core` is a Rust library with no Tauri dependency and every behaviour
in it. `hotline-app` is the Tauri 2 shell: the window, the desk door, the
menus. The window remembers its size, place and maximised state, and posts
a desktop toast when a teammate finishes or blocks while the window is not
focused — the first through a plugin, the second through a plugin on Linux
and Windows and through the notification center on macOS, where a click
comes back as an event and the toasts thread by teammate; the judgement is
the page's either way. The window
is built fresh in Phase 1 against the generated contract and the
subscription wire; `../hotline/src/mainview` is the reference for which
screens exist and how they behave, and a component is lifted from it only
where that is cheaper than writing it. Bun and Node exist only as the UI's
build tools; nothing runs on them.

The window's shared thread store keeps raw streaming text privately and
publishes whole reply bubbles, every growing thought, and the presence of a
reply even before its first bubble is whole (the mark reads that as writing).
A frame that changes none of those wakes no reader. Queued words and the
durable event that settles them publish together; only the raw fold settles
bubble remainders, so a hidden final paragraph cannot be lost. Live timestamps
are fixed on first arrival. Cadence still controls when whole bubbles appear;
header controls and work-card follow layout do not rerun for unrelated body
updates. `ui/tests/streaming-store.test.tsx` exercises this subscription boundary
and the final rendered reply, alongside the pacing and cadence tests.

Room-wide picker and inspector metadata (backend names, model efforts and
voice capability choices) shares in-flight and settled reads within one desk
connection. Room credential/model events, settings changes other than the
last-used model, and reconnect snapshots discard those reads. An unobserved
room reloads before serving them again; a replaced endpoint or token owns a
new cache. No credential read or persona computer status enters this cache.
The latter still refreshes per teammate and on its existing timer.
`ui/tests/room-reads.test.tsx` measures twenty teammate selections: model
efforts 10 → 1, backends 20 → 1, voice status 20 → 1, capabilities 20 → 1,
with computer status unchanged at 20. It also checks mounted invalidation,
remote desk isolation and late replies.

Providers use Rig's native clients for inference. Rig also signs ChatGPT and
Copilot in and refreshes them, through Hotline's own token store over each
login's directory (`providers/rig_login.rs`): ChatGPT's record is the
`auth.json` it has always been, and Copilot's is `copilot.json`, into which a
login made before Rig 0.45 is moved once from its `access-token` and
`api-key.json`, those removed only after the new record is written. Both
records use the vault's `CredentialFile`: plaintext moves to the configured
credential store on first read, verified before a reference replaces it. A
failed migration preserves the plaintext and refuses its use; it never
falls back from the keychain to a file. One
authenticator per credential serves turns, voice, image generation and
discovery, sharing one refresh and one in-memory token, and the store locks
each record file, never a provider, for any other authenticator over it (a
sign-in), so a rotated refresh token is spent once and a Copilot device flow
waiting on a person holds only its own login. Their models go through a
transport (`providers/outgoing.rs`) that signs each request with the live
token as it leaves. OpenRouter offers pasted keys and browser PKCE
sign-in; Hotline exchanges the code for a private API key and hands that key to
Rig's OpenRouter client. Ollama Local takes a server URL, while Ollama Cloud
takes an API key for `https://ollama.com`. Both use Rig's Ollama client for
native chat and model discovery. A discovered list belongs to its connection,
so replacing a server or account cannot reuse another connection's list.
Grok offers subscription device sign-in alongside xAI API keys. Hotline uses
the OAuth device flow and a private token store; a shared refresh lock keeps
teammates from spending the same rotated refresh token. Rig's xAI client
still owns inference, with a bearer-refresh HTTP client that retries a 401
once and never falls back to an API key. A 401 or 403 on a ChatGPT, Copilot
or xAI sign-in is checked against that sign-in's model list before the card
goes out (`Failure::checked_against_sign_in`, once per provider a minute):
if the list answers, the card says the provider is refusing requests
(`provider_refused`) rather than asking for a sign-in that would not help.
Z.ai Standard and Coding Plan are
separate API-key connections using Rig's native Z.ai endpoints.
Custom OpenAI-compatible connections have stable, separate provider ids, a
name, base URL, optional key, API choice, and an editable model list. Rig's
native OpenAI Responses and Chat Completions clients own inference and model
discovery, on a dialect of the connection's own that makes bearer
authentication optional and keeps Rig's catalogue of OpenAI's models from
describing a model that only shares an id with one of them. Inference runs
over a thin transport (`providers/compat.rs`) that repairs Responses frames
compatible servers send more loosely than OpenAI (missing sequence numbers or
item ids and statuses, a float `created_at`, echoed `instructions` items, an
empty `error`) before Rig reads them. Keys are privately bound to the exact
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

### Voice calls: desk first, teammate second


Legacy voice attaches to an agent thread (ADR 0008 §5). A new call has a
Call thread; `voice.call_start.thread` attaches to an existing work or call
thread owned by the selected teammate. Both use the work-thread occupant,
`Room::thread_agent` and `Room::run_queue`: one queue, independent revocable
lease, tools and checkpoint restoration, on Hotline Agent and ACP. Spoken
lines never enter the home DM. Hang-up detaches speech and leaves accepted
work running; only the turn in flight at hang-up posts its completed reply
as one chat message, and only if this call has not already delivered a result
or file to chat. Otherwise the closing note accounts for its late reply.
Replies over 900 characters or three paragraphs post
a short subject-first summary, with the full text kept on the call thread;
if summarising fails, a short notice points there instead. Later typed turns
stay on the thread unless they explicitly post. Calls and work threads
can be continued by text and called again.

Two minutes after hang-up (`CALL_CLOSING_DELAY`), once all the run's queues
are idle, the chapter summariser writes one subject-first account of the
discussion, decisions and finished work. A new call to the same teammate
invalidates the pending note, including one being generated, and retains its
source threads for the next note. One post links every source thread through
optional `from.sources`; older readers still use `from.thread`. Each source
then parks. These additive fields leave tape segments and the importer unchanged.
The summary receives the results already delivered to chat. Interrupted speech
or clipped model context is never evidence of a missing written result; a note
claiming a result was cut off is replaced with a factual pointer to the results.
A notice in the thread marks the covered interval by log position, so a later
note includes only new work even when timestamps tie. New input during the
summary causes it to be retried before publication; queue admission and final
parking share a gate. If no summariser answers, the post explicitly says so
and links to the transcript. The front's Digest and five-minute summaries
are gone. Live voices use `work_on_it` to start or steer one clone on the
call thread, on the teammate's selected model and harness. The whole call
so far is its brief. Reads still run beside it; the home session is never
interrupted. Stop cancels the call's work and drops queued changes. Speech
from both sides and each read/work tool are written in order on the thread.
Live text is buffered until completion or a work boundary; repeated updates
still fold by id. An interruption keeps the generated text before its cut
notice, and hang-up or provider loss flushes any remaining text. The clone's
agent messages never become call speech: its tools remain tool entries and
its completed answer is one `Work result` tool entry. Only the answer after
the last tool is offered to the live voice; the clone's opening is never
spoken. The actual voice response is recorded under its own speech id.
The clone posts drafts and files with a subject, then returns a short spoken
result. GPT-Live's client delegation uses this same work tool, without a
call-assistant delegation loop.

The call's chat row reads parked once its thread parks. The roster carries
`workingCalls`, the call thread ids with a turn in flight, so the row says
running only during a turn; an open thread between turns reads open.

Posts carry optional `from.title` and `silent` fields. A post made while the
person is on that thread's call is kept silently even after hang-up; later
posts update the preview, unread rail and notifications. These fields are
additive, leave the segment layout unchanged and are ignored by older tape
readers, including the reference importer. Replies to a post route to its
source thread. A fresh home-chat line stays with the home agent, receiving
all posts since it last spoke and source thread ids for its reading tools.

A call belongs to one owner connection and its selected desk. The desk is the
primary entry point; an optional teammate target uses that teammate's standing grants and its
call thread. Call and utterance ids correlate replies,
so unrelated work or an old reply never speaks on a direct call. Hold, hangup,
interrupt and connection revocation cancel voice input and output; an agent
instruction already accepted by the session keeps working.

Audio remains the default input for existing clients. It requires a
transcription provider, a connected one or a model on the desk, and accepts
whole WAV/AAC clips, or negotiated PCM.
An owner with `voiceTextInput` may request `inputMode: "text"` on `voice.status`
and `voice.call_start`; the latter echoes that mode and advertises only
`text/plain`. This mode resolves speaking independently of transcription.
The phone commits one finalized on-device transcript with
`voice.text {callId, seq, text}`; partial recognition never becomes an
instruction. Empty or oversized text, repeated sequences, another input mode,
hold and an unfinished preceding turn are refused before work is queued.
There is no remote STT request or STT budget reservation for device text.
Dispatcher and speaking costs keep their existing budget and ledger rules.

A conversational call answer is said once: its `<spoken>` words are also
its written record. Only a result speech cannot preserve needs a standalone
`<written>` version. The core recognizes code, tables, links, sent files,
lists of four or more items and write-ups over 120 words, and posts written
results through `Room::post`. Short conversation does not generate a second
chat message. The existing in-flight hang-up post and one closing note remain.

The core permits a contextual chat pointer at most once in three caller turns.
The teammate streams its own opening without a pointer; once the result is
known, the narrator can add a brief pointer in its own words. No stock phrase
or fixed word cap is requested. Legacy and narrator instructions carry the
turn count, last pointer turn and three recent generated spoken lines; live
voice receives the same state. Reservations count even if speech is cut or
generation fails, conservatively avoiding a repeated reminder. A conversational
turn never reserves one. Generated speech remains distinct from proof of hearing.
The optional `spoken` field and tape segment layout are unchanged.

Legacy speaks the teammate's own first sentence as it arrives, including an
untagged prose opening, and streams the spoken reply. It never waits for a
rewrite before saying that opening. If no speech has begun after 1.8 seconds,
the core sends one soft, wordless WAV on the ordinary clip wire, to desk and
phone alike. Only an audio clip suppresses that cue, not text awaiting TTS.
There is no repeating client chime or stock opening acknowledgement. While a
teammate is still working, twenty seconds without audio queues a short core-owned
first-person line in the same speech queue as its answer. Neutral lines rotate
without consecutive repeats; they claim no result, impending completion or
specific tool activity. The queue checks the turn again;
finished, interrupted, held or superseded turns cannot start that line.
Interruption cancels queued speech as before. Live work that has no spoken
version can still be rewritten before the live model says it.

The live model is the teammate: its name and goal (including manner), up to
4,000 characters of chapter context and 32 recent chat lines within 12,000
characters. The prompt teaches the kinds of turn, fresh lookups for state,
code and results, short answers and an explicitly requested longer answer.
Call-state refreshes replace the instructions through `session.update`, at
turn/tool boundaries and when durable state changes (checked every half
second). They include budgeted call speech, interruption notices, running
work, eight recent already-said lines and eight confirmed chat posts from
this thread. All injected prose is scrubbed as the starting context is.
The prompt and tool framing forbid relay language and repeated acknowledgements;
the fixed spoken regression deck checks all three live models when credentials
are supplied. Speech-to-speech audio cannot be filtered before it is heard.

A finalized transcript that is wholly a goodbye speaks `Goodbye.` and ends
either kind of call. Audio goodbye still requires enough genuine audio to
avoid a tiny hallucinated farewell. Replies use progressive independently
playable clips when negotiated, with ordinary whole-clip output preserved.
Clients cannot silently switch a text call to a paid audio provider; input
mode stays fixed for that call and a fallback requires an explicit new call.

The desk can also hear on its own, so a person on Windows, Linux or a
headless server, or a phone calling it, need not send what they say to a
provider. It is sherpa-onnx with NVIDIA's Parakeet transducers: one C library
with onnxruntime linked in statically, prebuilt for every platform Hotline
ships, where whisper.cpp would mean building C++ in every job and a model that
is slower on a CPU for the same accuracy. The same library runs Whisper, for
the languages Parakeet does not hear, and Moonshine, so a second engine is
never needed. A transducer is told the names it should expect (teammates',
the person's own words) with each utterance, rather than its transcripts
being corrected afterwards: a correction can only guess at a word the model
already lost. A model is half a gigabyte, so it is
never bundled: the owner downloads it once, the archive is pinned by hash, and
until then nothing is installed. An installed model is automatic's first
choice for hearing, before any paid key, because it is free and private and
was the owner's own act; it costs zero in the ledger.

Grok subscription speech is a separate, explicit provider choice backed by the
stored xAI login. Automatic speech keeps selecting API providers. A missing
login or subscription refusal returns an error; it never selects a paid speech
fallback. Subscription STT and TTS record zero estimated paid API spend, while
the desk dispatcher and agent work retain their ordinary spending rules.

### Interruptions describe playback, not generation

The desk keeps its microphone open during Legacy speech as it does for live
voice. Confirmed local speech onset and a tap stop playback locally before
waiting for the wire. Late clips from the cut Legacy line are discarded;
live audio remains muted until the caller's next transcript arrives. Browser
capture requests echo cancellation, and native Mac recognition enables voice
processing. Physical speaker cancellation, especially across the native
recognizer and webview playback, still needs the task-206 scenario pass;
these switches alone do not prove that the teammate cannot hear itself.

An interruption writes a notice on the call thread with the generated line,
its speech id and the played milliseconds when the live client supplies them.
It explicitly says that the line was not heard to its end and that the last
heard word is unknown: generation and elapsed audio do not establish a word
boundary. The notice metadata is additive; older readers keep its text, and the tape
segment layout and importer are unchanged. The written reply remains intact.
The next Legacy thread turn gets
the interruption as playback context, separately from the saved caller text,
and live providers receive truncation or interruption context. Cutting a
streamed reply also prevents its late completion from starting speech again.
The phone uses the same wire commands; its client-side onset and speaker
checks are a separate task-206 verification, not established by desk tests.

The Grok Voice and GPT Realtime socket adapters leave VAD in charge of
committing caller audio and starting its answer. From `speech_started`
through `input_audio_buffer.committed` until the automatic `response.created`,
a late tool result or line to say adds context without sending `response.create`.
The automatic answer consumes that context once. Playback truncation after
provider-detected speech does not send a second cancellation; a tap without
provider-detected speech still cancels active generation. No interruption
clears or manually commits the microphone buffer. Boundary traces identify
the socket session, elapsed milliseconds and turn state without audio,
transcripts, tool contents or provider identifiers. They show whether a stalled
turn reached speech stop, commit and response creation; scripted ordering tests
do not establish real-provider VAD timing, which needs the Linux audio QA pass.

Live work admission also checks the caller's finalized words. English dictation
opened by “draft”, “say” or “write” and ending in a dangling connector, article
or verb (such as “saying that” or “the fog horn should”) stays unfinished across
provider turns. A `work_on_it` attempt receives a silent wait result, even if
its arguments invent a complete sentence. Silence alone never releases it;
caller continuation closes the phrase, and the front requests the complete
work again. Stop remains available. When a tool beats transcription after
speech onset, admission waits for that transcript. This is a bounded lexical
check, not a general sentence parser; complete requests keep ordinary latency.
Scripted fronts prove the four-second pause on both Grok Voice and GPT Realtime;
real microphone timing and other languages still require audio QA.

### A transport drop holds the call for twenty seconds

A lost client socket holds its call for twenty seconds. Replaying the call
subscription from the same paired device resumes the existing call and thread,
with one “We're back on the same call.” line. A different device cannot take
it over, and another subscription does not transfer an active call's lifetime.
Revoking the pairing ends the call immediately, including during the hold.
Expiry uses the ordinary hang-up transition, so S5 detaches voice and preserves
ongoing thread work and its closing note. An explicit user hold stays held on
reconnection. The window shows Reconnecting during its socket retry or the
provider's reported recovery; terminal snapshots clear it. Phone adoption and
the physical relay scenario remain to be verified in the phone tree.

### Live calls keep the selected model

Settings selects the call mode in the core, including when a client omits
`voice.call_start.mode`. `front` requires a live model; `pipeline` is the
person's explicit Legacy choice. A missing login, spent budget or failed
provider never turns a live request into Legacy. `voice.status.liveSelected`
keeps selection separate from availability. A failed live call keeps its
one-sentence reason as `state.liveError`, including in its snapshot, so a
client arriving after the failure still explains it and offers **Call with
Legacy voice**. That choice starts a new call on the same conversation.

The provider adapters renew and reconnect with their bounded transcript.
If a session exits, the call runtime makes a bounded replacement attempt
with the conversation preserved; a replacement that fails before making
progress ends the live call visibly. Fixed local recordings explain a failed
live connection or spent Voice budget once and offer an explicit Legacy
choice, without asking the unavailable provider to speak. The call card keeps
the reason, while spoken notices contain no provider error codes. A revoked login or spent budget ends
it without reopening the revoked connection. Start, recovery and refusal
logs identify their reason. Refusal logs correlate provider `event_id` to
request type and redact credentials from the provider's message; request
bodies are never logged. An absent correlation is reported as unknown.

### 7. Skills, as files every harness reads

A skill is a procedure a teammate reads when the task calls for it: how to
work a computer, how to cut a release, the thing the person asked for twice
last week. It is the Agent Skills format: a directory named for the skill,
holding `SKILL.md` with `name` and `description` frontmatter and a Markdown
body, and whatever `scripts/`, `references/` and `assets/` the body points
at. The directory is `.agents/skills/<name>/`, the convention Codex already
scans; Hotline invents no harness directory of its own until one earns its
place. `name` matches its directory, is lowercase with hyphens and at most
64 characters; `description` is at most 1024 and says *when* to use the
skill, because it is all an agent sees before deciding to read the body. A
folder that breaks those rules is listed as invalid with the reason, never
silently skipped.

A skill has four sources and one channel. **Built-in** skills are bundled
in `hotline-core` under `skills/` and are always on: Hotline's own procedures for
its room and, when a teammate has one, its computer. The person's **own**
folder is `~/.agents/skills`, the standard place other agents on the machine
read too, or another folder the room's `skillsHome` names; Hotline reads it
where it is and copies nothing in, and each entry is offered to teammates by
a switch under Settings → Skills (the `offeredSkills` setting), so a skill the
person keeps for their other agents is one switch away from their teammates
and an edit there is what the teammate reads at its next start. The
**gateway** (Settings → Skills calls it **From other folders**) is the
operator's folder, `skills/` in the data directory, for a skill from anywhere
else: a folder the person picks is copied in under its
name, and everything valid there is offered. What is offered — the gateway's
and the person's own switched on — is granted per teammate with the same
none / some / all policy MCP servers use; a new teammate gets none, and all
includes skills offered later. **Workspace** skills are whatever is in the
teammate's own `.agents/skills`, put there by the person or by the teammate
itself. The channel is the workspace: on session start and on a grant
change, Hotline copies the built-ins and the granted skills into
`<cwd>/.agents/skills/`, so Hotline Agent reads them with its workspace tools
inside its reach and an ACP child reads them with its own. Copies, not
links, because a workspace mounted into a computer has to carry them. What
Hotline copied it marks, and the `AGENTS.md` rule applies: Hotline replaces and
removes only an entry carrying its marker, so a skill the person or the
teammate wrote is never touched, and revoking a grant leaves nothing of
Hotline's behind. A name the gateway has is the gateway's: the person's own of
the same name is listed, but not the one a grant of that name copies.

The preamble carries the index — each skill's name, description and path —
and nothing else about skills. That is the progressive disclosure the format
asks for, and it rides the block both drivers already hear, so a harness that
does not scan `.agents/skills` itself still knows what is there and reads the
file it is pointed at; Codex scans the directory natively and hears it twice,
which is harmless. A skill is something the agent decides to read, so nothing
a teammate must know before its first word is a skill: identity, reach, the
date, the names of Hotline's own tools and the house style stay in the preamble
as sentences. Built-in skills hold procedure, not standing. The first ones
are the room's workflows — chapters, schedules, asking a colleague, asking
the person — and the habit of keeping a skill for anything the teammate will
be asked for again, offering to write one before repeating work.

The computer's guide is a skill with a fourth provenance and no folder of its
own. The running container serves it with its release and checksum; at every
start Hotline asks the computer it just woke, writes the answer into the
workspace as `hotline-computer` under the marker with that release recorded in
it, and lists it under the teammate at that version. Freshness is by
construction, not by comparison: the marked folder is removed with the other
stale grants at the next start and written again from whatever is running,
so a container recreated on another release is read correctly the next time
the teammate starts, and a computer too old to serve a guide leaves no skill
and a preamble that tells the teammate to ask it directly. It is never in the
gateway, because the right guide is the one the running release ships, not
one the operator keeps.

Not yet: gateway sources from git or a registry, promoting a workspace skill
to the gateway from the pane, and honouring `allowed-tools` — a skill runs
with the teammate's reach and grants, no more.

## Module map

The tree, and what each module holds, is mapped once in
[development.md](development.md#layout).

## Phases

This is the order the tree was built in, kept as history. Each phase ended
with the app usable for what it covered, `make check` green, and a harness
proving the phase's behaviour over the wire. Nothing needed to be live
between phases; `../hotline` on `main` stayed the daily driver until Phase 1
replaced it.

- **Phase 0 — the core, headless.** The log with folds and subscriptions;
  the room stream with teammates and settings; the vault; the index; the
  moved contract, tools and Rig loop; the wire with the desk seat. Proof: a
  harness creates a teammate, runs a Hotline Agent turn with a real key, and
  watches the tape and the roster view over the wire.
- **Phase 1 — the room you can live in.** Session funnel, chapters and the
  summariser, search, the Tauri shell, the React window on the new wire,
  the importer for an existing Hotline data directory. Exit: George daily-drives
  it for Hotline Agent teammates.
- **Phase 2 — the other driver and the tools.** ACP child driver; rmcp
  client and Hotline's own MCP server; the ledger; MCP settings and OAuth.
  Exit: Cursor and Claude Code teammates work; the tool ledger reads true.
- **Phase 3 — the room's edges.** Peer threads and receipts, the scheduler
  with quiet runs, attachments, notifications, the phone seat with pairing
  and push. Exit: the phone joins the room.
- **Later, each when it earns its place**: the fleet as stream shipping,
  the computer, the client seat, subscription logins, updater and signing.

## Not in scope

The iOS app is untouched until Phase 3 gives it a door. The old repository
is read-only reference and is not modified by work here.

### Teammate messages: ask or hand off

`message_teammate(to, message, intent)` defaults to `ask`: a bounded answer or
review in a side session with none of the recipient's main conversation. Ask
is not tool-free: the recipient uses its own workspace and granted tools.
`handoff` opens a work thread of the recipient's own, which starts from its
main conversation's context, to implement or continue work there; the main
conversation is never interrupted by it.
The recipient never silently promotes an Ask. Neither intent overrides the
person or expands the recipient's permissions.

One directional collaboration approval explains and covers both intents. Old
standing approvals remain valid for Ask, but the first Handoff requires informed
consent. The room records that consent separately; removing authority invalidates
queued work too. There is no room link, link/unlink tool, roster link or chapter
link lifecycle. Old link records are inert.

A room `exchange_pair` record keeps each pair's durable request queue, result,
and message count together. A Handoff delivery retains sender, thread and
`requestId`; its final turn result returns automatically as a correlated Peer
delivery, even when the sender has moved on. Both intents remain readable in
the pair's thread. Twelve automatic messages, including replies and across
intents, pause the pair. Receipts do not count. Queued requests and completed
results wait without loss. The person's Keep going resets the counter; Stop
exchange settles outstanding work and writes an explicit notice. Hearing from
the person also resets the pair. These actions do not create new grants.

Restart recovery preserves paused queues and completed results. Work known to
have started is not blindly replayed: an interrupted turn returns an explicit
failure advising inspection before retrying. An unstarted handoff resumes from
its saved request. This chooses at-most-once side effects over pretending that a
crash can safely repeat arbitrary agent work. Stable delivery ids prevent a
saved result from appearing twice. Acceptance is reported only after the request
is saved. Storage failures are logged; startup treats any running request without a saved result as uncertain
rather than replaying possible side effects.


A Handoff that asks the person keeps the human action ids in its pair record.
Only a clean end of that turn suspends the request: its waiting message is not
its final result. The later human-answer turn inherits the request's reply
route, including after restart, and checks collaboration consent again before
running. Answer deliveries have stable ids; recovery can deliver a saved card
answer even if the desk stopped before dispatch. Starting that continuation
is recorded before calling the driver, so an interrupted continuation is still
uncertain work, never a replay. Old requests without action ids retain the old
settlement rules; no route is guessed from conversation text.

### The team lead

A desk has at most one lead: a teammate's id, kept in the room stream as
the `teamLead` setting and carried on that teammate's roster row. The core
alone names one unasked, by one rule: a desk with exactly one teammate and
no lead makes that one lead, at create, at delete and when the desk opens.
So onboarding's first teammate leads, a desk upgraded with one teammate gets
its lead, and deleting down to one leaves that one leading. Deleting the lead
clears it, and among two or more nobody is ever picked: the person makes a
teammate lead (which moves the role) or clears it with `persona.lead`, owner
only. Clearing a desk of one is refused, since the next open would put it
back, and `settings.update` cannot name a lead around these rules. Being lead
changes nothing else about a teammate: its own model, memory and tools stay
its own. A desk call opens the lead's own call thread and voice front. The
front answers roster questions with `list_teammates` and passes named work
straight through `message_teammate`, intent `handoff`, saying who has it. The
live desk call and current lead together grant local handoffs without a
first-contact card; a direct call, DM, stopped call or former lead does not
carry that grant. The queue checks it again before starting work. Stopping or
reassigning colleagues remains the person's choice. With no lead the fallback
call assistant remains, with the roster and optional `personName` setting,
using bounded named tools rather than the wire command table. No new
`voice-dispatcher` tape is written; each fallback call keeps its own transcript
([ADR 0008](adr/0008-one-agent-thread-layer.md#5-voice-under-this-design)).

### Linked desks

The window can show several desks; linking two of them lets their
*teammates* see and message each other (ADR 0003). Connecting a window to a
desk and linking two desks are different acts: the first is the person's
client reaching a desk as an owner, the second a grant between the two
desks themselves, made deliberately from Settings › Desks and ended from
either side.

A link trusts the other desk's own Noise key, not a device key, and its
channel is answered under the link seat: a linked desk publishes its
teammates and delivers messages, and every other command and subscription
is refused before it runs. The desk that made the link dials the other's
`/v2/link`, directly or through its relay, and redials until the link ends;
the channel then carries both directions, so a laptop behind NAT is never
dialled.

A teammate on a linked desk is `deskId/personaId` here and never a persona
in this roster. `list_teammates` lists it with its desk's name, from the
roster rows the other desk publishes (exactly what a local row shows), and
`message_teammate` reaches it by name, as "Name on Desk" when a name is
shared, or by id; a shared name is refused with every choice listed, and a
local teammate never wins a tie. A message rides the sender desk's exchange
queue, crosses when it is taken up or waits until the link is back, and
enters the recipient desk's queue as a request from a participant with
nothing granted: the recipient's `allowedSenders` and the person's
first-contact card decide it there, the card on the recipient's tape, and
a whole-machine teammate on another desk is no more trusted for it. The
answer comes back the same way, saved until it arrives, and each desk keeps
its own copy of the pair's thread. Ending a link fails what waited on it
with a sentence, withdraws the cards its teammates were waiting on and ends
the standing grants they held, so a link made again starts from nothing.

The window lists every desk's teammates in one rail, with a desk filter
where it had a switcher. The screen follows the teammate: opening one on
another desk makes that desk the one the window talks to, so its settings,
computer and files are that desk's, and nothing is switched by hand. A desk
that cannot be reached keeps its teammates listed, dimmed, as they last
were.

### Cards use the call without moving their authority into the model

A card is announced once at a break on the matching active call instead of
sending a phone push. The caller's finalized yes/no, not a model tool,
answers the last announced card and leaves a chat record from the call.
The handler allows core-owned collaboration approvals for this session only
and informational `ask_question` cards under the requesting session's live
lease. `set_person_name` adds one core-owned operation: the proposed name
and lease are bound to its card before publication, and only approval writes
`personName`. Settings → General also reads and writes it as **Your name**.
The home agent receives the current name with each prompt; live voice reads
it when the call opens and hears the name card's answer during the call.
An informational answer never grants an operation. Existing action
cards, passkeys and opaque driver approvals stay at the desk: their words
cannot establish that an action is safe for voice. This includes credentials,
2FA, screen interaction, payment and deletion. Replayed cards without their
live authority cannot regain it from a saved marker.

`human_action.question` is an optional additive field; absence means an
ordinary action, and the tape segment layout and importer are unchanged.
Call-thread cards live in the home chat, stamped `from` the call with its
title and thread key. This optional additive field on permission,
human-action and passkey cards leaves older tapes and the importer unchanged.
The call stream keeps the same id as a linked status line; settlement updates
both views. Answers from the home chat still reach the original call agent,
and the voice handler keeps exactly the same authority checks. The call
queue keeps request identities across provider reconnection and removes
settled cards; interrupted generation is still not proof of what was heard.


### Files produced by call work reach the home chat

The core collects successful write/edit records from the call's own work turn.
Hotline Agent preserves its workspace mutation's path and created/edited status;
ACP preserves diff paths and creation status (location-only edit reports count
as edited). The optional `files` field on a tool event and `created` field on an
attachment are additive; existing tapes, segment layout and importer are unchanged.
The core reads through the existing workspace reach under the call's revocable
lease. It never scans directories or infers paths from the conversation or shell
commands. Missing, oversized or inaccessible files leave a warning on the call.

At work completion, explicit `send_file` calls join the recorded mutations in
one batch. Each file is delivered once per work result, stamped From the call: up to
three as separate attachments, more as one message with a compact list. Every
entry opens and saves its stored copy. Text and other documents open in an inert
text or binary preview; PDFs retain the native viewer. Indexed storage keeps equal basenames
separate and addresses readback by message and attachment position, never by a
caller-supplied source path. A later correction can deliver the updated file; a later call owns its own results. Work that survives
hang-up still delivers while its lease is valid; revocation prevents delivery.

### Credentials are entered in a card, never in a conversation

`request_credential {site, reason, kind, name?}` proposes a login or passkey
for one full origin. The core binds the card to its requesting teammate,
thread and a child of that session's capability lease. It grants nothing.
The tool returns immediately; a later delivery names the stored key, its
site and how to fill it, or says why the request stopped. A thread can ask
for an origin once per turn. A pending card expires after thirty minutes;
withdrawal or a revoked session also ends its authority. A saved card cannot
recreate a lease after restart.

The desk form sends `secrets.capture.answer` once. It carries the action id,
chosen name, optional login values and Ask me choice. No name means decline;
a name without values grants a matching existing key or arms a passkey.
Neither a different origin nor a different teammate can be sent. The phone
seat may send this command, but no other secret write, including the generic
thread-answer equivalent. A request lock settles racing seats once. An accepted answer finishes even
if the phone disconnects during delivery. The
form clears typed fields on submit, dismiss and settlement, and expires
locally even when its connection has gone away. The transcript's additive
`credential_ask` event holds only metadata and outcome; tape segments and
the importer are unchanged.

A login is stored with `sites: [requested_origin]`, granted to the requester
and delivered through the existing whole-set computer door before the agent
hears the outcome. Existing keys keep their values and policy. Ask me defaults
to false on old and new records; capture saves it with a login or minted
passkey and includes it in delivery. Use-time approval is the separate
credential-policy work (task-267 and task-268), not a capture operation.
The computer currently allows same-scheme/port subdomains of a configured
login host; capture does not tighten that computer-side rule. Strict
origin-only filling remains a counterpart acceptance gap for task-266.

A passkey has no value fields. Make a passkey arms the existing computer
flow, then the site's separate approval card still needs the person. A
captured passkey requires that operator approval and the card's exact origin;
a computer's report of approval cannot grant it. Minted storage uses the
capture's lock and live lease too, so withdrawal cannot later become a grant.
The card settles when the passkey is stored, or expires with an unused arming.

On a call, the card lives in home chat and the call mirrors its status. Voice
announces once that the desk and phone are waiting, without reading the
request's free-form reason. Spoken yes or done has no authority here. A
stored outcome delivered to the call says only `Stored as NAME`; its card
queues that fixed spoken notice once, independently of the agent’s reply.
