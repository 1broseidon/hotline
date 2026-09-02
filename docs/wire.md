# The wire

One WebSocket per client, carrying commands and subscriptions. Frames are
JSON text. Types are defined in `crates/toad-core/src/contract.rs`; the
door that reads them is `crates/toad-core/src/wire/`.

The door binds `127.0.0.1` on an ephemeral port. `Door::run` serves until
the listener itself is gone. A client that hung up before the handshake,
an interrupted call, or a brief shortage of file descriptors is waited
out — fifty milliseconds on the shortage, so that case is not a spin —
because returning here kills the wire for the life of the process.

One writer task per socket queues whole frames in the order they were
produced, so answers, snapshots and events never interleave. Commands are
answered on the read loop, in the order they arrived: the append a
command makes *is* its answer, and a client that creates a teammate and
then subscribes must see it.

A frame with no `id` is dropped. `id` is a JSON number (`i64`).

## Three frames a client may send

A command:

```json
{"id": 1, "cmd": "persona.create", "params": {"draft": {"name": "Ada"}}}
```

answered exactly once with success, or with an error:

```json
{"id": 1, "ok": true, "result": { … }}
{"id": 1, "ok": false, "error": "There is no teammate nobody."}
```

A command whose result is JSON `null` — delete, stop, prompt, cancel,
revoke, `session.answer_permission`, `human.answer`, `schedule.cancel`,
`schedule.set_quiet`, and a successful unsubscribe — is answered
`{"id": n, "ok": true}` with no `result` field. `teammate.tools` is not
on that list: when there is no ledger it is answered
`{"id": n, "ok": true, "result": null}`, because that null is a value,
not a void. Absent `params` and `"params": {}` are the same thing.

A subscription:

```json
{"id": 7, "sub": "room"}
```

answered `{"id": 7, "ok": true}`, then one snapshot, then events as they
land:

```json
{"sub": 7, "snapshot": [ … ]}
{"sub": 7, "event": { … }}
```

The broadcast is subscribed to **before** the fold is loaded for the
snapshot, so nothing can land in the gap. An event that lands in both is
harmless: every client folds by event id, and the second copy of a line
supersedes the first with itself. A stream subscriber that falls behind
is sent a second snapshot rather than the events it missed.

An unsubscribe, naming the subscription's id:

```json
{"id": 8, "unsub": 7}
```

answered `{"id": 8, "ok": true}`. An id that is not open is an error.

Anything else with an id is refused: `"A frame is a command, a subscription or an unsubscribe."`

## Commands

`cmd` is the enum tag, `params` the content. Field names on the wire are
camelCase. The table is the `Command` enum in `contract.rs` and what
`wire/commands.rs` returns.

| cmd | params | result |
| --- | --- | --- |
| `persona.create` | `{draft}` | the created `Persona` |
| `persona.update` | `{id, patch}` | the teammate after the patch |
| `persona.delete` | `{id}` | none — the agent is stopped, its peer sessions dropped, its tape kept |
| `settings.update` | `{patch}` | every setting, defaults included |
| `credential.create` | `{providerId, label, secret}` | the `Credential` (no secret) |
| `credential.revoke` | `{id}` | none |
| `credential.delete` | `{id}` | none |
| `backends.list` | `{}` | `BackendChoice[]`: Toad Agent first, then the ACP catalogue |
| `credential.list` | `{}` | `Credential[]`, never a secret |
| `providers.list` | `{}` | `Provider[]` Toad Agent can hold a key for, whether or not the desk holds one |
| `models.list` | `{}` | `ConfigChoice[]` the desk's keys can reach |
| `session.start` | `{personaId}` | `SessionInfo` |
| `session.stop` | `{personaId}` | none |
| `session.prompt` | `{personaId, text, replyTo?, attachments?}` | none |
| `session.cancel` | `{personaId}` | none |
| `session.set_model` | `{personaId, modelId}` | `SessionInfo` |
| `session.set_mode` | `{personaId, modeId}` | `SessionInfo` |
| `session.answer_permission` | `{personaId, requestId, optionId}` | none |
| `human.answer` | `{personaId, actionId, status: "done"|"declined", note?}` | none |
| `search.thread` | `{personaId, query, limit?}` | `{hits, truncated}` |
| `search.all` | `{query, limit?}` | `{hits, truncated}` |
| `chapter.list` | `{personaId}` | chapter summaries, newest first |
| `room.import` | `{from}` | an import `Report` |
| `chapter.start_fresh` | `{personaId}` | the chapter that closed, with its note |
| `chapter.resume` | `{personaId}` | the chapter that reopened, with the earlier note |
| `teammate.tools` | `{personaId}` | a `TeammateToolLedger`, or JSON `null` |
| `schedule.create` | `{personaId, kind, when?, every?, prompt, quiet?}` | the created `ScheduledJob` |
| `schedule.list` | `{}` | `ScheduledJob[]`, soonest first |
| `schedule.cancel` | `{id}` | none |
| `schedule.set_quiet` | `{id, quiet}` | none |
| `peers.list` | `{personaId}` | `PeerThreadSummary[]`, newest first |
| `peers.mark_read` | `{key, eventIds}` | how many messages moved to read |

`backends.list` is every harness this machine can start, and the ones it
knows of but cannot, with the reason. Toad Agent (`id` `"pi"`) is always
first. `unavailable` is absent when the row can be started here and a
sentence naming what is missing when it cannot.

`session.answer_permission` is refused when nothing is waiting behind that
request any more — the turn ended, the session stopped, or somebody else
answered first — so a stale card cannot silently let an agent through.

`human.answer` is the same fact for a `request_human` card: `done` or
`declined`, with an optional `note` the agent receives word for word,
refused when the deadline passed, the session stopped, the room restarted,
or somebody else answered first. The tape still writes `dismissed` for a
decline, which is the previous Toad's word for that afterlife.

`PersonaDraft` is `{name, goal?, team?, backendId?, cwd?, reach?,
modelId?, computer?}`. Create fills what the draft leaves blank: a fresh
uuid, name `"Untitled"` if blank, empty goal, `backendId` from the room's
`defaultBackendId` or `"pi"`, a workspace under the data directory,
`mcpPolicy` `{mode: "all", serverIds: []}`, and no `reach` unless the
draft asked for `"machine"`. The whole teammate is written as one room
event; a patch is folded over the record and the whole record is written
again, because a stream folds by id and a partial line would leave half a
teammate.

`settings.update` writes one event per key. JSON `null` is a tombstone
and puts that key's default back. The result is the room's settings after
the patch.

`session.prompt`'s `replyTo` is the id of the message this one answers.
`attachments` are `{kind: "image"|"file", name, path, mimeType?, size?}`.
The command returns as soon as the turn is started; what the turn
produces reaches the client as tape events and ephemeral deltas.

`search.thread` defaults `limit` to 20 and clamps it to 1–40.
`search.all` defaults `limit` to 30 and clamps it to 1–60. A query is cut at 200 UTF-16 code
units. Hits are chapters first, then messages; a thread hit has no
`personaId`, a global hit does. `truncated` is true when more messages
matched than the limit. A missing index or an empty query is
`{hits: [], truncated: false}`.

`chapter.list` is the tape's chapter markers: `{id, startedAt, endedAt?,
title?, note?, status?, closedBy?, messages}`.

`peers.list` is every thread this teammate has with another teammate:
`{threadKey, withPersonaId, withName, exchanges, lastAt, waiting,
workingPersonaId?, preview}`, newest first. `preview` is JSON `null` when
nothing has been said, rather than omitted, so a thread that exists is
still a row. The events of one of them are a `{"thread": key}`
subscription, which is a stream like any other, so there is no command
that loads a thread. `peers.mark_read` says that
those messages have been read and answers how many actually moved: an id
naming nothing, an event that is not a message, and a message that is
already read all move nothing, which is what makes a repeated receipt
harmless. A message's `receipt` is `sent` when it enters the thread and
`read` once the recipient's session has proved a turn on it; nothing ever
un-reads a message.

`room.import`'s `from` is a path to an existing Toad data directory. The
source is never written. `Report` is `{teammates, tapes, threads,
schedules, settings, keys, skipped: [{item, reason}], notes: [{item,
reason}]}`. `skipped` is left behind (a setting this Toad does not have, a
teammate already in the roster, a thread whose sides are both strangers, a
job whose teammate is not here, a teammate that lives on another desk);
`notes` is imported with a caveat (a backend this registry has no
counterpart for, an MCP server this tree cannot read, a workspace that
still lives under the old data directory) or a rebuildable piece that
failed (the search index will catch up on the next start). A
`store.sqlite` that exists but cannot be read is an error, not a report
of zeros.

`teammate.tools` is what tools this teammate was given the last time it
started, where they came from, and — for anything absent — why. JSON `null`
when it has never started under a Toad that keeps a ledger, sent as
`result: null` rather than by omitting the field. The ledger itself is
[sessions.md](sessions.md).

`schedule.create`'s `kind` is `"schedule"` (once) or `"loop"` (every
interval until cancelled). Times are milliseconds: `when` is a one-shot's
fire, milliseconds since epoch; `every` is a loop's interval. `quiet`
defaults to false. A one-shot cannot carry `every`; a loop cannot carry
`when`. The job is an event on the room stream; the clock that fires it is
[sessions.md](sessions.md).

A command the room cannot read is `"This room cannot read that command:
…"`. A seat that may not run one is `"That seat may not run this
command."` A seat that may not subscribe to a target is `"That seat may
not subscribe to that."`

## Subscriptions

`sub` is a `Target`: `"room"`, `{"tape": "<personaId>"}`,
`{"thread": "<key>"}`, or `{"view": "roster"}`.

| target | snapshot | then |
| --- | --- | --- |
| `"room"` | the room stream's fold | each room event as it lands |
| `{"tape": id}` | that tape's fold | each tape event; `ephemeral` for streaming deltas |
| `{"thread": key}` | that thread's fold | each thread event |
| `{"view": "roster"}` | every living teammate's row | `event` for a changed row, `removed` for a tombstone |

A tape subscription also forwards `StreamDelta`s for that teammate, never
written down:

```json
{"sub": 1, "ephemeral": {"type": "agent_delta", "personaId": "ada", "messageId": "m2", "text": "hel"}}
```

`type` is `agent_delta` or `thought_delta`. A delta for another teammate
is ignored. A closed delta channel is not recovered: the durable line
carries the characters anyway.

A view row that goes away:

```json
{"sub": 3, "removed": "<personaId>"}
```

Opening the same subscription id twice is `"Subscription n is already
open."` A subscription whose task has ended — the stream closed, with no
unsubscribe — frees the id, so a client that reuses the number is not
told it is already open for a subscription that will never deliver.
Unsubscribing an id that is not open is `"Subscription n is not
open."`

## The roster view

Nothing logs this. It is a join of the room stream, each teammate's tape,
and the live sessions. Each row is a `RosterEntry`:

```json
{
  "persona": { … },
  "preview": {"from": "me", "text": "morning", "at": 5},
  "latest": 5,
  "activity": "read note.txt",
  "session": { "personaId": "…", "state": "thinking", … }
}
```

`preview` is absent for a teammate that has never spoken. `from` is `"me"`
for a user line and `"them"` for an agent line; only those two kinds
count. `latest` is that line's `at`, kept beside it so the window can count
unread without opening every tape. `activity` is the title of the tool
still running, only while the session is thinking — absent, not null, when
there is none. `session` is a `SessionInfo` (`state` is `idle`, `starting`,
`ready`, `thinking`, `error`, or `stopped`). A persona tombstone on the
room stream emits `removed` rather than a row. A session that reports
itself after its teammate was deleted is not put back. If the view falls
behind on the room stream it reloads every row; a lagged burst of
session-info is ignored.

## The seat

A seat is what a socket may do: a set, not a routing table. The only
variant is `Desk`. The desk seat may run every command and subscribe to
every target. The token that opened the socket is what seated it.

## The token gate

The handshake is `GET /ws?token=<token>`. Any other path is 404 (`"This
door serves the room's wire only."`). A token that does not match is 401
(`"unauthorized"`). Comparison is constant-time on the bytes; a
different-length token is refused without leaking how much matched.

The shell generates a 32-byte hex token per launch and injects it as
`window.__toadDesk.token`. The harness in `crates/toad-core/tests/desk.rs`
uses a token of its own. There is no other way through the door.

The streams these subscriptions read are [log.md](log.md). What a session
does with a command is [sessions.md](sessions.md). How to run the room is
[development.md](development.md).
