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
`schedule.set_quiet`, `computer.stop`, `computer.remove`, and a successful
unsubscribe — is answered
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
| `credential.login` | `{providerId}` | `LoginPrompt` `{loginId, userCode, verificationUri}` |
| `credential.login_status` | `{loginId}` | `LoginStatus` `{state, credential?, error?}` |
| `credential.refresh_models` | `{providerId}` | `CatalogModel[]` that provider's catalogue after a re-fetch |
| `credential.revoke` | `{id}` | none |
| `credential.delete` | `{id}` | none |
| `backends.list` | `{}` | `BackendChoice[]`: Toad Agent first, then the ACP catalogue |
| `credential.list` | `{}` | `Credential[]`, never a secret |
| `mcp.auth_start` | `{serverId}` | secret free OAuth status plus authorization URL and native callback |
| `mcp.auth_callback` | `{loginId, callbackUrl}` | secret free OAuth status |
| `mcp.auth_status` | `{serverId}` | secret free OAuth status |
| `mcp.auth_reconnect` | `{serverId}` | secret free OAuth status plus authorization URL and native callback |
| `mcp.auth_sign_out` | `{serverId}` | none; invalidates live sessions and clears the protected registration, and a pasted token with it |
| `mcp.secret_set` | `{serverId, url, secret}` | none; saves a bearer or header server's token in the protected vault, bound to the URL |
| `providers.list` | `{}` | `Provider[]` Toad Agent can hold a key for, whether or not the desk holds one |
| `models.list` | `{}` | `ConfigChoice[]` the desk's keys can reach |
| `models.catalog` | `{providerId}` | `CatalogModel[]` that provider's catalogue, newest first |
| `models.efforts` | `{modelId}` | `ConfigChoice[]` that model's effort levels, empty when it has none |
| `session.start` | `{personaId}` | `SessionInfo` |
| `session.stop` | `{personaId}` | none |
| `session.prompt` | `{personaId, text, replyTo?, attachments?}` | none |
| `session.cancel` | `{personaId}` | none |
| `session.set_model` | `{personaId, modelId}` | `SessionInfo` |
| `session.set_mode` | `{personaId, modeId}` | `SessionInfo` |
| `session.set_config` | `{personaId, configId, value}` | `SessionInfo` |
| `session.answer_permission` | `{personaId, requestId, optionId}` | none; `collab:` request ids are room-owned collaboration cards |
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
| `computer.runtimes` | `{}` | `RuntimeReport[]`: detection, rootless-available first |
| `computer.status` | `{personaId}` | `{state, url?, viewer?}` — a peek, never a wake |
| `computer.stop` | `{personaId}` | none |
| `computer.remove` | `{personaId}` | none |

`backends.list` is every harness this machine can start, and the ones it
knows of but cannot, with the reason. Toad Agent (`id` `"toad"`) is always
first. `unavailable` is absent when the row can be started here and a
sentence naming what is missing when it cannot.

`session.answer_permission` is refused when nothing is waiting behind that
request any more — the turn ended, the session stopped, or somebody else
answered first — so a stale card cannot silently let an agent through.

The same command answers a room-owned `collab:<id>` card before any driver is
asked. Its options are `allow_session`, `allow_always`, and `deny`; the card
names both teammates and says that the recipient can use its workspace and
enabled tools to fulfill the caller's requests and return results. A session
choice is held against both live capability leases. An always choice appends
the caller's stable id to the recipient's `allowedSenders` list. A workspace
caller gets a card on first contact in each direction; an explicit Whole
machine Toad Agent caller does not. ACP mode and Computer access do not imply
Whole machine authority. `list_teammates` returns only each other teammate's
`personaId` and `name`.

`human.answer` is the same fact for a `request_human` card: `done` or
`declined`, with an optional `note` the agent receives word for word,
refused when the deadline passed, the session stopped, the room restarted,
or somebody else answered first. The tape still writes `dismissed` for a
decline, which is the previous Toad's word for that afterlife.

`PersonaDraft` is `{name, goal?, team?, backendId?, cwd?, reach?,
modelId?, effortId?, computer?}`. Create fills what the draft leaves blank: a fresh
uuid, name `"Untitled"` if blank, empty goal, `backendId` from the room's
`defaultBackendId` or `"toad"`, a workspace under the data directory,
`mcpPolicy` `{mode: "none", serverIds: []}`, and no `reach` unless the
draft asked for `"machine"`. Background work defaults off, and
`allowedSenders` defaults to an empty list. The whole teammate is written as one room
event; a patch is folded over the record and the whole record is written
again, because a stream folds by id and a partial line would leave half a
teammate. A patch that names `cwd`, `reach`, `goal`, `mcpPolicy`,
`computer`, `backgroundWork`, `allowedSenders`, `backendId` or `harnessOverride` invalidates
current main and peer execution before writing the record, clears queued
turns, and then reattaches a live main session. The old driver cannot keep
using the previous grant while the new one is being installed.

`settings.update` writes one event per key. JSON `null` is a tombstone
and puts that key's default back. The result is the room's settings after
the patch. A patch that names `mcpServers` invalidates all main and peer
sessions before persistence and reattaches live main sessions afterward.
Policy updates are serialized across client sockets. One failed restart
does not prevent the remaining teammates from applying the change.
`computerRuntime` is the user's pick of `"docker"`, `"podman"` or
`"container"`; absent means the first available runtime. `computerImage`
is the room's default image, under a teammate's own. Neither reattaches
anything: both are read when a computer wakes, as are the teammate's own
`persona.computer.memory`, `pids` and `mounts` through `persona.update`.

`computer.runtimes` is detection for the window: every CLI Toad knows,
whether it is on PATH, why not, and whether it is rootless. `computer.status`
is a peek at one teammate's container (`running`, `stopped`, `absent`).
`url` is the MCP endpoint and `viewer` is `http://127.0.0.1:<host port for
8787>/#<token>` when it is running and this process woke it, so the window
can open the desktop later. The token rides in the fragment, which the page
reads and the server never sees in a request line.
`computer.stop` and `computer.remove` do not wake anything.

`enabledModels` is an object from provider id to an array of model ids
(bare catalogue keys, so OpenRouter keeps its own slash). A provider
absent from the object shows every model; a present one shows only the
listed ids. A value that is not an object, or an entry that is not an
array of strings, reads as absent — a bad setting costs its own filter,
never the picker.

`defaultModelId` is the person's standing model for Toad Agent, set in
Settings, a `provider/model` string. `lastModelId` is the model a Toad
Agent teammate most recently ran on or was set to, written by the wire.
Both are absent until someone writes them. JSON `null` on
`defaultModelId` puts the last-used fallback back.

`session.set_model` writes the teammate's `modelId` first and switches a
live session second. It works on an idle teammate: the persona is the
truth, and the live switch is a courtesy to the turn already running. A
Toad Agent id the desk's `models.list` does not name is refused; an ACP
teammate accepts any non-empty id, and the harness validates when live.

`session.set_config` is the same shape for a setting that is not the
model or the mode. For a Toad Agent teammate with `configId` `"effort"`,
it writes `effortId` (or `null` when `value` is empty) first and
switches a live session second. A value the teammate's effective model
does not list is refused; when no model is known yet, the value is
accepted. An ACP teammate goes only to the live session — idle is "That
teammate is not running." `models.efforts` is the idle picker's list for
one catalogue id, each choice labelled (`low` → "Low", `xhigh` →
"Extra high"). A new teammate starts on the model's default effort.

`effortId` on the persona is the stored effort, optional like `modeId`.
An ACP teammate does not store one: the harness owns its config ids.

`models.catalog` lists a provider's discovered models when available, with
manual additions and bundled metadata matched by exact provider/model ID.
Without a discovered list, the bundled catalogue supplies the fallback.
Models absent from models.dev remain visible; `metadataKnown` distinguishes
exact catalogue matches and `manual` identifies user-added IDs. Live names
and limits take precedence over bundled values; the bundle fills missing
metadata. Unknown optional limits and capabilities are omitted. Each row's `enabled` flag reflects the
saved filter. An unwired provider is an error.

`credential.refresh_models` uses the active connection and Rig's native
listing client, then answers with `models.catalog`. An absent connection or
unsupported discovery method is an error. Failed discovery preserves the
last successful list; manual IDs remain separate. Refresh never updates
selected models or enabled-model filters. `providers.list` exposes
`modelDiscovery` so the window offers Refresh only for supported providers.

`models.manual_set {providerId, modelIds}` replaces the manual IDs for an
active non-custom connection and returns its updated `models.catalog`.
The input is a list of exact model IDs, not display labels or URLs. It never
changes credentials, endpoints, filters, or the current model. Copilot manual
IDs must also occur in its successfully fetched account list. Custom
connections continue to edit their IDs through `credential.custom_save`.

`providers.list` also exposes `credentialKinds`, the methods each provider
offers (`api_key`, `oauth`, or `local`). `credential.create` accepts only
providers offering `api_key` and refuses blank keys.
`credential.connect_local {baseUrl}` validates an Ollama HTTP/HTTPS URL,
discovers its models, and then records a `local` credential with that
`baseUrl`. It writes no secret. The chosen server may have a reverse-proxy
path, but the URL cannot contain credentials, a query or a fragment.

`credential.custom_save {id?, draft: {name, baseUrl, api, models, secret?}}`
creates a custom connection or edits the given active custom credential.
`api` is `responses` or `chat_completions`; `models` holds raw model IDs.
Omitting `secret` preserves the existing key; an empty string removes it.
A saved key cannot be reused at a changed URL without explicitly re-entering
it. The result is credential metadata with `custom: {api, models}` and a stable
`providerId` of `custom-<credential-id>`. Its kind is `api_key` or `local`.
The generic `openai-compatible` provider row uses these commands, not
`credential.create`.

`credential.custom_models {id?, baseUrl, secret?}` discovers model IDs through
Rig's OpenAI client, using the same key semantics. It returns a sorted list
without saving anything. Errors leave manual models usable. Custom model
catalogues and enabled-model filters are keyed by the unique provider id.
Custom keys live privately beside their credential, bound to the normalized
base URL, and are never returned in credential metadata.

`credential.login` starts sign-in for a provider offering `oauth`, and
answers with a login id and browser URL. ChatGPT, Copilot and xAI also provide
a `userCode`; OpenRouter leaves it empty and uses a loopback PKCE callback.
It is start-then-poll because commands run sequentially per socket.
`credential.login_status` reports `pending`, `done`, or `failed`; an unknown
id is an error, and a finished login stays queryable until process exit.
The login id is the credential id. `credential.login_cancel {loginId}`
stops a pending attempt, closes its callback listener or polling future,
and discards its unfinished vault files. A completed attempt stays completed.
OpenRouter expires after five minutes waiting for the browser; all login
attempts are bounded by ten minutes overall. Only credential metadata is
returned through the wire; acquired keys remain in the private vault.

`mcp.auth_start` and `mcp.auth_reconnect` discover an HTTP MCP server's
protected-resource and authorization-server metadata, register a native
client only when the advertised DCR endpoint exists, and return an
authorization URL. The browser returns to the native loopback listener;
`mcp.auth_callback` is available for an embedding that delivers that URL
itself. `mcp.auth_status` reports `signed_out`, `pending`, `signed_in` or
`failed`; its result never includes an access token, refresh token or client
secret. `mcp.auth_sign_out` revokes the live gateway capability before it
deletes the registration and tokens from the protected vault. Signing in
does not change a teammate's MCP policy.

`mcp.secret_set` is the other credential: the token a server in `bearer` or
`header` auth mode sends on every request. The window saves it before it
writes the server into settings, so the reattach that write causes finds
the token in the vault; the settings entry carries only the mode and, for
`header`, the header name. A server can be saved before it exists in
settings, which is how a new one is added in one go. `mcp.auth_status`
answers `signed_in` when the vault holds the token and `signed_out` when
it does not, and `mcp.auth_sign_out` forgets it.

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

The desk's `schedule.create` is an operator instruction and writes
`operatorCreated: true`; callers do not supply that field. These jobs may
run with the teammate's `backgroundWork` off. Agent-created jobs and older
jobs without provenance require that grant. `schedule.list` retains the
operator's room-wide view; the agent tool `list_schedules` can only return
its own jobs.

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
