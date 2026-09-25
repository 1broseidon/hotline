# The wire

One WebSocket per client, carrying commands and subscriptions. Frames are
JSON text. Types are defined in `crates/hotline-core/src/contract.rs`; the
door that reads them is `crates/hotline-core/src/wire/`.

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

A refusal a client should act on by its reason, rather than show, also
carries a `code`: `forbidden` when the seat may not run the command or open
the subscription, `unknown_teammate` when a schedules subscription names a
teammate the room does not hold, and `unreadable` when the room stream could
not be read. The sentence stays for a person; the code is for the client.

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
| `backends.list` | `{}` | `BackendChoice[]`: Hotline Agent first, then the ACP catalogue |
| `credential.list` | `{}` | `Credential[]`, never a secret |
| `welcome` | `{}` | `Welcome`: where a fresh room stands on its way to a first turn |
| `desk.looking` | `{looking}` | none; whether the person is at the window, which keeps the phone quiet ([Pushes](#pushes)) |
| `mcp.auth_start` | `{serverId}` | secret free OAuth status plus authorization URL and native callback |
| `mcp.auth_callback` | `{loginId, callbackUrl}` | secret free OAuth status |
| `mcp.auth_status` | `{serverId}` | secret free OAuth status |
| `mcp.auth_reconnect` | `{serverId}` | secret free OAuth status plus authorization URL and native callback |
| `mcp.auth_sign_out` | `{serverId}` | none; invalidates live sessions and clears the protected registration, and a pasted token with it |
| `mcp.secret_set` | `{serverId, url, secret}` | none; saves a bearer or header server's token in the protected vault, bound to the URL |
| `providers.list` | `{}` | `Provider[]` Hotline Agent can hold a key for, whether or not the desk holds one |
| `models.list` | `{}` | `ConfigChoice[]` the desk's keys can reach |
| `models.catalog` | `{providerId}` | `CatalogModel[]` that provider's catalogue, newest first |
| `models.efforts` | `{modelId}` | `EffortChoices` — `choices` that model's effort levels, empty when it has none, and `defaultId` the one a teammate with none stored runs at |
| `session.start` | `{personaId}` | `SessionInfo` |
| `session.stop` | `{personaId}` | none |
| `session.prompt` | `{personaId, text, replyTo?, attachments?}` | none |
| `session.cancel` | `{personaId}` | none |
| `session.set_model` | `{personaId, modelId}` | `SessionInfo` |
| `session.set_mode` | `{personaId, modeId}` | `SessionInfo` |
| `session.set_config` | `{personaId, configId, value}` | `SessionInfo` |
| `session.answer_permission` | `{personaId, requestId, optionId}` | none; `collab:` request ids are room-owned collaboration cards |
| `human.answer` | `{personaId, actionId, status: "done"|"declined", note?}` | none |
| `file.read` | `{personaId, eventId, offset}` | `FileChunk` `{name, mimeType, size, offset, data, next?}` — at most 512 KiB of the file that message carries, as base64; `next` is where the next part starts, absent at the end |
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
| `computer.runtimes` | `{}` | `RuntimeReport[]`: detection, rootless-available first; Apple's container only in a macOS build |
| `computer.releases` | `{}` | `ComputerReleases`: `floor`, `repository`, `newest?`, `releases` (floor up, newest first), `checkedAt?`, `error?` |
| `computer.releases.check` | `{}` | the same, after asking the releases endpoint now |
| `computer.status` | `{personaId}` | `{state, url?, viewer?}` — a peek, never a wake |
| `computer.stop` | `{personaId}` | none |
| `computer.remove` | `{personaId}` | none |
| `computer.browsers.list` | `{}` | `[{id, name, family, profiles: [{id, name}]}]` — the host browsers cookies could come from; names only |
| `computer.cookies.preview` | `{browserId, profileId}` | `[{domain, cookies}]` — the sites in that profile and their counts, never a value |
| `computer.cookies.import` | `{personaId, browserId, profileId, domains}` | `[{domain, cookies}]` — the sites actually imported |
| `computer.cookies.list` | `{personaId}` | `CookieImport[]` `{browserId, browserName, profileId, profileName, importedAt, sites: [{domain, cookies}]}` — what was brought over to that teammate's computer, by browser and profile; names and counts, never a value |
| `computer.cookies.forget` | `{personaId, browserId, profileId, domain?}` | `CookieImport[]` — what is left, after the computer's browser dropped every cookie for that site or, without `domain`, for every site brought over from that browser and profile |
| `secrets.list` | `{}` | `SharedSecret[]` `{name, updatedAt, kind, sites?, username?, totp?, rpId?, userName?}` — names, kinds and what each is for, never a value |
| `secrets.set` | `{name, value}` | the `SharedSecret` stored, a variable; the value is never answered back |
| `secrets.login.set` | `{name, sites, username, password, totp?}` | the `SharedSecret` stored, a login; the password and seed are never answered back |
| `secrets.delete` | `{name}` | none |
| `secrets.passkey.register` | `{name, personaId, rpId}` | `PasskeyRegistration` `{state: "armed", name, rpId, expiresAt}` — that teammate's computer is armed for ten minutes to make one passkey for `rpId` |
| `secrets.passkey.registration` | `{personaId}` | `PasskeyRegistration` `{state: "idle" \| "armed" \| "asked" \| "approved" \| "stored", name?, rpId?, expiresAt?, ask?, secret?}` — where the making stands; `asked` and `approved` carry the site's request `{id, rpId, origin, rpName?, userName?, userDisplayName?, askedAt}`, which waits for the card on the teammate's tape; the room watches the arming itself and stores the passkey the moment it is made, and this answers `stored` with the record, once |
| `secrets.passkey.answer` | `{personaId, askId, approved}` | `PasskeyRegistration` — the person's answer to the passkey card: approved, the browser makes it (`approved`, then `stored` once the room has it); denied, the arming ends (`idle`). Refused when no such request is waiting. The phone may send this one |
| `secrets.passkey.cancel` | `{personaId}` | none — ends the arming with nothing stored; a card nobody answered expires |

`computer.browsers.list`, `computer.cookies.preview`, `computer.cookies.import`,
`computer.cookies.list` and `computer.cookies.forget` are the operator's cookie
import and its record: reading a browser on the person's own machine, handing
the chosen sites' cookies to a teammate's computer, and taking them back. They
are desk-seat only — the phone allowlist does not name them and no agent tool
reaches them, so the agent can never pull cookies itself. A preview carries
domains and counts, and leaves expired cookies behind, since the browser would
drop them on its next look; a value crosses only on import, host to desk to
container, and never enters the tape, the model, or a log. `browsers.list` and
`cookies.preview` read the host and touch no teammate. `cookies.import` starts
the teammate's computer if it is stopped, the same as opening its screen would,
and records what it carried — browser, profile, time, domains and counts — on
the room stream, one record per teammate, folded into what was recorded before
for the same browser and profile. `cookies.list` answers that record.
`cookies.forget` names the exact domains to the computer's `DELETE
/logins/{name}` door, so a site is taken back whatever the saved login holds by
then, starts a stopped computer to do it, and trims the record; a site never
brought over is refused, and a release from before the door is named with the
pane's Update.

`secrets.list`, `secrets.set`, `secrets.login.set` and `secrets.delete` are
the operator's store of secrets for teammates to use without seeing them,
kept in the OS credential store through the vault. A name is
`[A-Z][A-Z0-9_]*`, not `HOTLINE_*` and not the shell's own. A variable
(`set`) is a value of at least eight characters that becomes an environment
variable in a granted computer. A login (`login.set`) is one or more sites —
`https://` origins, or `http://` on localhost — a username, a password of at
least eight characters, and optionally a TOTP seed, which a granted computer
types into a form only on a page of those sites when the teammate asks for
`NAME.username`, `NAME.password` or `NAME.code` by name. A passkey is never
sent in: `secrets.passkey.register`, from the teammate's pane, arms that
teammate's computer for one site, `rpId` a lower-case host name, for ten
minutes, starting the computer if it is stopped, and the room then watches
that arming by itself, every two seconds until it ends. Under the arming,
the site's own request to make a passkey waits in the browser: the look
that finds it writes a `passkey_ask` card on the teammate's tape — the
site, the origin, the account the site named — and tells the phones, and
`secrets.passkey.answer` is the person's answer to that card, from the
desk or the phone. Approved, the browser makes it, and the look that finds
the credential minted stores it under `name`, ticks `name` on that
teammate's `persona.computer.secrets`, hands the computer its set, ends
the arming and writes a notice on the teammate's tape, whatever pane is
open — the passkey is made from the teammate's screen, or by the teammate,
so no pane's poll is running at that moment. Denied, the site hears no and
the arming ends. A request that leaves with its page, an arming that runs
out or is cancelled, and a computer that stops each leave the card
expired. `secrets.passkey.registration` answers where it stands, `stored`
with the record once; `secrets.passkey.cancel` ends an arming with nothing
stored. The store is write-only from the window: `set` and `login.set`
answer the record, `list` answers names, kinds and what each is for,
`registration` answers the record once stored, and no command,
subscription or room event ever carries a value or a private key. All but
`answer` are desk seat only; `answer` is one answer to one request the
person armed for at the desk, so the phone may give it, as it may answer a
permission card. Which teammate may use which secret is
`persona.computer.secrets` through `persona.update`; a change to a stored
secret also hands every running computer the set its teammate is granted
now, so a rotation or a revocation lands without a restart (see
[security.md](security.md)).

`backends.list` is every harness this machine can start, and the ones it
knows of but cannot, with the reason. Hotline Agent (`id` `"hotline"`) is always
first. `unavailable` is absent when the row can be started here and a
sentence naming what is missing when it cannot.

`welcome` is what the window's welcome pane reads in place of an empty
room: the providers with a live credential, by name; the ACP harnesses this
machine can start; the room's default backend; `canRun`, true once a
teammate could run on a provider or on a harness that is the default; and
the number of teammates. It is derived from the credentials, `backends.list`
and the roster every time it is asked, never stored, so there is no "seen"
flag to reset: the pane is on screen exactly while the room has no teammate,
and opens on the step that is still to do.

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
machine Hotline Agent caller does not. ACP mode and Computer access do not imply
Whole machine authority. `list_teammates` returns only each other teammate's
`personaId` and `name`.

`human.answer` is the same fact for a `request_human` card: `done` or
`declined`, with an optional `note` the agent receives word for word,
refused when the deadline passed, the session stopped, the room restarted,
or somebody else answered first. The tape still writes `dismissed` for a
decline, which is the previous edition's word for that afterlife.

`PersonaDraft` is `{name, goal?, team?, backendId?, cwd?, reach?,
modelId?, effortId?, computer?, backgroundWork?}`. Create fills what the draft leaves blank: a fresh
uuid, name `"Untitled"` if blank, empty goal, `backendId` from the room's
`defaultBackendId` or `"hotline"`, a workspace under the data directory,
`mcpPolicy` `{mode: "none", serverIds: []}`, and no `reach` unless the
draft asked for `"machine"`. Background work is off unless the draft turned
it on, and `allowedSenders` defaults to an empty list. The whole teammate is written as one room
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
`persona.computer.memory`, `pids` and `mounts` through `persona.update`. A
`computer` patch does reattach the teammate, and the reattach hands a running
computer the secrets `persona.computer.secrets` names now.

`computer.runtimes` is detection for the window: every CLI Hotline knows,
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

`defaultModelId` is the person's standing model for Hotline Agent, set in
Settings, a `provider/model` string. `lastModelId` is the model a Hotline
Agent teammate most recently ran on or was set to, written by the wire.
Both are absent until someone writes them. JSON `null` on
`defaultModelId` puts the last-used fallback back.

`session.set_model` writes the teammate's `modelId` first and switches a
live session second. It works on an idle teammate: the persona is the
truth, and the live switch is a courtesy to the turn already running. A
Hotline Agent id the desk's `models.list` does not name is refused; an ACP
teammate accepts any non-empty id, and the harness validates when live.

`session.set_config` is the same shape for a setting that is not the
model or the mode. For a Hotline Agent teammate with `configId` `"effort"`,
it writes `effortId` (or `null` when `value` is empty) first and
switches a live session second. A value the teammate's effective model
does not list is refused; when no model is known yet, the value is
accepted. An ACP teammate goes only to the live session — idle is "That
teammate is not running." `models.efforts` is the idle picker's list for
one catalogue id, each choice labelled (`low` → "Low", `xhigh` →
"Extra high"), with `defaultId` naming the level a teammate with no
stored effort runs at. That is `high` whenever the model lists it, so a
fresh Hotline Agent teammate thinks properly instead of at whatever the
provider picks when nothing is sent; a model with no such level runs with
nothing sent. The stored `effortId` always wins over the default, and a
model switch to one that does not list the stored level falls back to the
new model's default.

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

A teammate sends the person a file with its `send_file` tool, and the file
arrives as the teammate's message: an `agent` event whose `text` is the
caption, possibly empty, and whose `attachments` holds the one file,
`{kind, name, path, mimeType, size, width?, height?, origin}`. `kind` is
`image` for a picture, which the desk has already made into a JPEG of at
most 2000 px, and `file` for anything else, kept as it came, up to 25 MB.
`width` and `height` are a picture's pixels, so a client can hold its
place before it loads. `origin` says where it came from in words. `path` is
the desk's own copy, under `files/` in the data directory, and means
nothing off this machine: a client reads the file with `file.read`, which
names a message and never a path, and serves only the one file the desk
kept for that message. A message with no such file is `"That message has
no file."`; an offset past the end is refused in a sentence. A phone needs
no capability to ask: only a desk that serves `file.read` sends a
teammate's file.

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

`room.import`'s `from` is a path to an existing Hotline data directory. The
source is never written. `Report` is `{teammates, tapes, threads,
schedules, settings, keys, skipped: [{item, reason}], notes: [{item,
reason}]}`. `skipped` is left behind (a setting this Hotline does not have, a
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
when it has never started under a Hotline that keeps a ledger, sent as
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
not subscribe to that."` Both seat refusals carry `"code": "forbidden"`.

## Subscriptions

`sub` is a `Target`: `"room"`, `{"tape": "<personaId>"}`,
`{"thread": "<key>"}`, `{"view": "roster"}`, or
`{"schedules": "<personaId>"}`.

| target | snapshot | then |
| --- | --- | --- |
| `"room"` | the room stream's fold | each room event as it lands |
| `{"tape": id}` | that tape's fold | each tape event; `ephemeral` for streaming deltas |
| `{"thread": key}` | that thread's fold | each thread event |
| `{"view": "roster"}` | every living teammate's row | `event` for a changed row, `removed` for a tombstone |
| `{"schedules": id}` | that teammate's jobs and loops | the whole list again as a `snapshot` whenever it changes; `removed` when the teammate is deleted |

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

## The schedules view

A teammate's scheduled jobs and loops, for a client that may read them but
not the room stream they are kept on, which also carries every setting and
grant. It is how a paired phone reads schedules; the desk may open it too.

`{"id": n, "sub": {"schedules": "<personaId>"}}` is answered `{"id": n,
"ok": true}` and then `{"sub": n, "snapshot": [ScheduleEntry…]}`, soonest
first. Every later frame is the whole list again, as another `snapshot`,
sent only when the list changed. A client replaces what it holds with each
one; there are no deltas to apply, so a new job, a cancellation, a one-shot
that fired and a loop whose next run moved cannot leave a stale or doubled
entry behind. Another teammate's jobs never appear and never cause a frame.

A `ScheduleEntry`:

```json
{"id": "…", "personaId": "…", "kind": "loop", "prompt": "Sweep the inbox",
 "every": 3600000, "nextAt": 1790000000000, "quiet": true}
```

`kind` is `schedule` (once) or `loop`. `when` is a one-shot's original time
and `every` a loop's interval; `nextAt` is when the desk next means to wake
the teammate for the job. Times are milliseconds since the Unix epoch and
intervals are milliseconds. `nextAt` is a plan, not a promise: a desk that
is closed or asleep fires a missed job once when it can, a fire that could
not start tries again a minute later, and a loop counts its next interval
from when a run ends. `quiet` is present and true when the job was asked to
say nothing in the chat unless it finds something worth saying. There is no
paused or failed state: a job is listed until it has fired for the last
time or is cancelled. Who made a job is not part of the entry.

The view is read before the subscription is acknowledged, so a list that
cannot be told is a refusal and never `ok` followed by `[]`:

- a teammate the room does not hold: `{"id": n, "ok": false, "error":
  "There is no teammate <id>.", "code": "unknown_teammate"}`;
- a room stream that cannot be read: `"code": "unreadable"`;
- a seat that may not open it: `"code": "forbidden"`.

After it opens, a teammate deleted ends the view with `{"sub": n,
"removed": "<personaId>"}`, and a room that can no longer be read ends it
with `{"sub": n, "error": "…", "code": "unreadable"}`. Either way the id is
free again. A view that falls behind on the room stream reads the list
again. Reconnecting is the refresh: a new subscription's snapshot is the
list as it is now, including what changed while the client was away.

What a client can tell apart, and where each comes from:

| state | comes from |
| --- | --- |
| a list, possibly empty | the response: `ok`, then a `snapshot` (`[]` is "nothing scheduled") |
| loading, not yet known | the client: the subscription is sent and no snapshot has arrived |
| offline, or stale | the transport: the socket closed, or the view ended with `unreadable`; what the client holds is the last list it was sent, and a new subscription replaces it |
| an older desktop | the hello: `capabilities` does not list `schedules` (see [the phone seat](#the-phone-seat)) |
| not allowed | the response: `forbidden`; or the transport, for a phone whose pairing was revoked, whose socket is closed and whose handshake is refused |
| no such teammate | the response: `unknown_teammate`, or `removed` on an open view |
| the desk could not read it | the response: `unreadable` |

## The seat

A seat is what a socket may do: a set, not a routing table. The token that
opened the socket is what seated it. The desk seat may run every command
and subscribe to every target.

### The phone seat

A paired phone's socket is the phone seat: a smaller fixed set of commands
(`Seat::permits` in `crates/hotline-core/src/wire/mod.rs` lists them) and
three kinds of subscription — a tape, the roster, and a teammate's
schedules. It never opens the room stream, a thread or a run, and it can
neither make, cancel nor quiet a job. It reads a file a teammate sent with
`file.read`, because the file is part of the conversation it already
reads. Anything else is refused with `"code": "forbidden"`.

The phone's socket opens with a hello before any answer:

```json
{"type": "hello", "protocolVersion": 1, "desktopId": "…", "mode": "team",
 "capabilities": ["schedules"]}
```

`capabilities` names what this desk can do beyond protocol 1, so a phone
asks only for what the desk it reached understands. `schedules` is the
schedules view. A desk from before the list sends no `capabilities`, and a
phone reads that as "not on this desk", never as "nothing scheduled"; asked
anyway, such a desk refuses the target as one it cannot read.

### Pushes

A phone that registered a token with `mobile.push_register` hears each
turn's reply and every card that waits on the person as an Expo push
(`crates/hotline-core/src/push.rs`). A turn is one push however many
bubbles it took: its last reply, sent once the driver is done with the
line. A card goes the moment it is asked. Nothing goes while the desk's
window says the person is at it (`desk.looking`, below). Every push is `mutableContent`, so the
phone's notification service may rewrite it as the teammate's own message.
`data` always names `desktopId` and `personaId`. A card that waits on the
person also names its kind as `categoryId` and its request as
`data.requestId`, which is the id the answer names:

| `categoryId` | `data.requestId` is | Answered with |
|---|---|---|
| `permission` | the request's `requestId` | `session.answer_permission`, with an `optionId` from `data.options` (`[{optionId, kind}]`) |
| `human_action` | the card's `actionId` | `human.answer` |
| `passkey_ask` | the ask's `askId` | `secrets.passkey.answer` |

The answer goes over the phone seat like any other, so an answer from the
notification lands on the tape exactly as one from the conversation.

The window sends `desk.looking {looking: true}` while it has the focus,
is showing and has been used lately, again every thirty seconds at most
while that lasts, and `{looking: false}` on a blur or when it is hidden.
The room holds one `true` for two minutes, so a window left focused on an
empty desk lets the phone hear again. A desk seat command: a phone cannot
send it. An open phone already has every tape live, and its own handler
hides the banner for the conversation on its screen.

## The token gate

The handshake is `GET /ws?token=<token>`. Any other path is 404 (`"This
door serves the room's wire only."`). A token that does not match is 401
(`"unauthorized"`). Comparison is constant-time on the bytes; a
different-length token is refused without leaking how much matched.

The shell generates a 32-byte hex token per launch and injects it as
`window.__hotlineDesk.token`. The harness in `crates/hotline-core/tests/desk.rs`
uses a token of its own. There is no other way through the door.

The streams these subscriptions read are [log.md](log.md). What a session
does with a command is [sessions.md](sessions.md). How to run the room is
[development.md](development.md).
