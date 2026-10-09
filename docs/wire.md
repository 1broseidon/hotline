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
then subscribes must see it. A command that names one teammate — by
`personaId`, by a `thread` (a DM is its teammate's), by a `sideId`, or as
the `id` of a `persona.*` command — is answered on that teammate's own lane
instead: in order with that teammate's other commands, and waiting on
nobody else's. Starting an agent can take a while, or never finish, and it
holds up only the commands about that teammate. Its answer may therefore
arrive after the answers to commands sent after it.

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
`schedule.set_quiet`, `computer.stop`, `computer.remove`, `thread.prompt`,
`thread.cancel`, `thread.park`, `thread.close`, `thread.answer`, and a
successful unsubscribe — is answered
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
| `mobile.persona_create` | `{requestId, name, goal?, backendId?, modelId?, effortId?}` | the created `Persona`, with workspace reach, this desk's default workspace, no computer and no background work — the fields the phone cannot name; a repeated `requestId` answers the same teammate unchanged |
| `mobile.persona_update` | `{id, name?, goal?}` | the teammate with its new name, goal or both — nothing else a patch could carry; an empty goal clears it, a blank name or an edit naming neither is refused |
| `persona.update` | `{id, patch}` | the teammate after the patch |
| `persona.delete` | `{id}` | none — the agent is stopped, its peer sessions dropped, its tape kept |
| `persona.pin` | `{id, slot?}` | the desk's pinned ids, in order — pins the teammate at `slot` (0-based, shifting the rest along, past the end appends) or unpins it when `slot` is absent; a pinned teammate given a slot moves; a fourth pin is refused; owner or local desk only |
| `settings.update` | `{patch}` | every setting, defaults included |
| `images.status` | `{}` | `ImagesStatus` `{available, unavailable?, provider?, model?, spending?, spendingUnavailable?}`; owner or local desk only |
| `capabilities.options` | `{}` | `CapabilityOptions` `{images, stt, tts, dispatcher, spending}`; owner or local desk only. Each job is `{selected?, automatic?, unavailable?, options}`: `selected` is the owner's pick (absent means automatic), `automatic` is what automatic resolves to now, `unavailable` is a sentence when no connected provider can do the job, and `options` is `[{providerId, providerName, models: [{id, label?, voices?}]}]` from connected providers only. `spending` is `{dayUsd, monthUsd, spentDayUsd, spentMonthUsd, unavailable?}` |
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
| `session.retry` | `{personaId}` | none; runs the last turn again when it failed and nothing was said after it, without writing the person's message again; otherwise refused |
| `session.prompt` | `{personaId, text, replyTo?, attachments?}` | none |
| `session.cancel` | `{personaId}` | none |
| `session.set_model` | `{personaId, modelId}` | `SessionInfo` |
| `session.set_mode` | `{personaId, modeId}` | `SessionInfo` |
| `session.set_config` | `{personaId, configId, value}` | `SessionInfo` |
| `session.answer_permission` | `{personaId, requestId, optionId}` | none; `collab:` request ids are room-owned collaboration cards |
| `human.answer` | `{personaId, actionId, status: "done"|"declined", note?}` | none |
| `file.read` | `{personaId, eventId, offset}` | `FileChunk` `{name, mimeType, size, offset, data, next?}` — at most 512 KiB of the file that message carries, as base64; `next` is where the next part starts, absent at the end |
| `avatar.read` | `{personaId, hash, offset?}` | `FileChunk`, as `file.read` answers — a teammate's kept picture, a square PNG. `hash` is the 64 lowercase hex digits from the teammate's `avatar.hash`; anything else, or a picture the desk does not keep, is refused in a sentence. Every seat may read one |
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
| `peers.answer_permission` | `{key, requestId, optionId}` | none; answers a card raised in a peer turn while that turn waits on it. Owner seat only |
| `side.start` | `{personaId, text}` | the new `SideThreadSummary`, returned at once with the task already written to the thread: its agent starts on its own task and the first turn begins when it is up, so the summary is `working` until then. A line said meanwhile waits behind it, and an agent that cannot start says so in the thread, which is then archived as failed; past three running agents the idlest thread is parked, and it is refused only while all are mid-turn |
| `side.prompt` | `{sideId, text, attachments?}` | none; returns at once, the answer is on the `{"side": id}` subscription; a parked thread is brought back first, an archived one is refused until it is continued |
| `side.cancel` | `{sideId}` | none; stops the turn in flight, the thread stays live |
| `side.archive` | `{sideId}` | none; ends a live or parked thread, and archiving an archived thread is also none |
| `side.continue` | `{sideId}` | the `SideThreadSummary`, live again; brings back a parked or archived thread, resuming its saved session when the harness can |
| `side.list` | `{personaId}` | `SideThreadSummary[]`: live first, then parked, then archived, each newest first; each carries a one-line `preview` of the newest thing said |
| `side.answer_permission` | `{sideId, requestId, optionId}` | none |
| `client.hello` | `{capabilities}` | `{capabilities}`: what this core can do for the seat. Names this socket as reading `threads2`, see [Threads](#threads); any seat; an unknown name is ignored |
| `thread.list` | `{personaId?}` | `ThreadSummary[]` for that teammate, or for the whole room without `personaId`; live first, then parked, then closed, each newest first. A phone is not shown pairs or calls |
| `thread.open` | `{personaId, text}` | the new work thread's `ThreadSummary`, returned at once and starting as `side.start` describes |
| `thread.prompt` | `{thread, text, replyTo?, attachments?}` | none; returns at once, the answer is on the thread's subscription. The main conversation and a work thread only (`session.prompt`, `side.prompt`) |
| `thread.cancel` | `{thread}` | none; stops the turn in flight, the thread stays as it was. On a pair it stops the automatic exchange |
| `thread.park` | `{thread}` | none; lets go of a work thread's agent and keeps it open: saying something in it brings one back. Parking a parked thread is none |
| `thread.close` | `{thread}` | none; ends a work thread, transcript kept (`side.archive`) |
| `thread.continue` | `{thread}` | the thread's `ThreadSummary`, live again. A parked or closed work thread, or, on a pair, a paused exchange |
| `thread.answer` | `{thread, answer}` | none; `answer` is `{kind: "permission", requestId, optionId}` or `{kind: "human", actionId, status: "done"\|"declined", note?}`, routed by the thread's kind |
| `thread.page` | `{thread, before, limit?, through?}` | `{events, more}`: older lines of any thread than its subscription opened with (`tape.page` on a DM) |
| `computer.capacity` | `{}` | `{runtime: "docker"\|"podman"\|"container"\|null, cpus, memoryBytes, source: "runtime"\|"host"\|"default"}`; read-only for every seat |
| `mobile.persona_computer` | `{id, enabled?, memory?, cpus?: number\|null}` | updated `Persona`; owner phone or desk only |
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
import and its record: reading a browser on the desk host, handing
the chosen sites' cookies to a teammate's computer, and taking them back. They
require an owner or local desk seat — the companion allowlist does not name
them and no agent tool reaches them, so the agent can never pull cookies itself. A preview carries
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
`answer` require an owner or local desk seat; `answer` is one answer to one request the
person armed for at the desk, so the phone may give it, as it may answer a
permission card. Which teammate may use which secret is
`persona.computer.secrets` through `persona.update`; a change to a stored
secret also hands every running computer the set its teammate is granted
now, so a rotation or a revocation lands without a restart (see
[security.md](security.md)).

`backends.list` is every harness this machine can start, and the ones it
knows of but cannot, with the reason. Hotline Agent (`id` `"hotline"`) is always
first. `unavailable` is absent when the row can be started here and a
sentence naming what is missing when it cannot. It carries only `id`, `name`,
`description` and `unavailable`, nothing that needs a credential to read, so
the phone seat may ask it too.

`mobile.persona_create` is the phone's own narrow `persona.create`: it names
a teammate and, optionally, a goal, a harness, a model and an effort, and
core fills in the rest with what the phone posture must never widen — the
workspace as reach, a fresh workspace under the data directory as `cwd`, no
computer, no background work. `requestId` is a uuid the phone mints; it
becomes the teammate's id, so a retry after a lost acknowledgement answers
the teammate already made rather than making a second one. `backendId`
defaults the way `persona.create`'s does and must otherwise name a row
`backends.list` reports with no `unavailable`; naming anything else, an
unparseable `requestId`, or a blank name are each refused with a sentence.
The full `persona.create` — reach, path and computer included — stays
desk-seat only.

`welcome` is what the window's welcome pane reads in place of an empty
room: the providers with a live credential, by name; the ACP harnesses this
machine can start; the room's default backend; `canRun`, true once a
teammate could run on a provider or on a harness that is the default; the
number of teammates; and `setUp`, true once a teammate has been made in the
room, whether or not one is left. It is derived from the credentials,
`backends.list` and the room stream every time it is asked, never stored, so
there is no "seen" flag to reset. `setUp` reads the stream's `persona`
events with their tombstones: a delete leaves one in the teammate's place, so
a room keeps having been set up through deletes and compactions, and a room
imported with teammates is set up from its first open. The pane is on screen
exactly while the room is not set up, and opens on the step that is still to
do; a room that is set up and empty keeps the window and offers New teammate
instead.

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
Whole machine authority. `list_teammates` returns each other teammate's
`personaId` and `name`, plus what the roster already knows about it: `state`
(`idle`, `working`, `waiting` or `stopped`), the running tool's title as
`activity`, and `workingOn` (`title` — the open chapter's or the goal's,
whichever there is — and `lastTurnAt`). Never a message's text, a working
path, or a tool inventory.

`teammates.exchange_resume {a,b}` and `teammates.exchange_stop {a,b}`
answer a pair's `exchange_paused` card from either seat. Keep going resets
the twelve-message count and releases queued asks, handoffs and replies;
Stop exchange settles outstanding work without revoking collaboration.
The card's status becomes `resumed` or `stopped`. These are operator
commands, not teammate tools. There are no link commands or roster links.

`human.answer` is the same fact for a `request_human` card: `done` or
`declined`, with an optional `note` the agent receives word for word. A card
that `delivers` is answered off the tape at any time until it is settled,
across restarts, and the answer reaches the teammate as a `delivery`. A card
a tool is waiting on (a colleague's side session asked it) is refused once
the deadline passed, the session stopped, the room restarted, or somebody
else answered first. The tape still writes `dismissed` for a
decline, which is the previous edition's word for that afterlife.

`PersonaDraft` is `{name, goal?, team?, backendId?, cwd?, reach?, folders?,
modelId?, effortId?, computer?, backgroundWork?}`. Create fills what the draft leaves blank: a fresh
uuid, name `"Untitled"` if blank, empty goal, `backendId` from the room's
`defaultBackendId` or `"hotline"`, a workspace under the data directory,
`mcpPolicy` `{mode: "none", serverIds: []}`, and no `reach` unless the
draft asked for `"machine"`. Background work is off unless the draft turned
it on, and `allowedSenders` defaults to an empty list. The whole teammate is written as one room
event; a patch is folded over the record and the whole record is written
again, because a stream folds by id and a partial line would leave half a
teammate. A patch that names `cwd`, `reach`, `folders`, `goal`, `mcpPolicy`,
`backgroundWork`, `allowedSenders`, `backendId` or `harnessOverride` invalidates
current main and peer execution before writing the record, clears queued
turns, and then reattaches a live main session. The old driver cannot keep
using the previous grant while the new one is being installed.

`folders` is the teammate's extra folders, `[{path, writable?}]`, on the
persona and the draft. Absent means none, and so does an empty list, which
is how a patch clears them. `writable` absent is `false`: read-only. Create
and a patch naming `folders` check every entry before anything is revoked
or written, and refuse the whole list with a sentence when one is not an
absolute path (`~` expands) to an existing directory, is `/`, the home
directory or a folder holding it, Hotline's data directory, a folder
holding it or a folder inside it (another teammate's workspace there too),
is this teammate's workspace or inside it, overlaps another entry, or when
there are more than 16. What is stored, and answered, is each path as the
directory it resolves to (`/tmp/x` is `/private/tmp/x` on macOS); the same
folder twice is one entry, writable if either said so. A teammate's
`SessionCapabilities.additionalDirectories` says whether its harness took
them when its session opened; Hotline Agent's is `false` and means nothing.

`pinnedTeammates` is the desk's pinned teammates: a list of up to three
persona ids, in the order they sit at the top of the team. `persona.pin` is
how it is written; a patch that names it is held to the same rules (a list of
ids, repeats dropped, more than three refused). Deleting a teammate takes its
pin with it, and a stored list that names someone no longer on the team reads
as if they were not in it. Companion phones cannot read settings, so the
roster carries each teammate's slot as `pin`.

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
`persona.computer.memory`, `cpus`, `pids` and `mounts` through `persona.update`.
Only changing whether a computer is enabled reattaches the teammate; limit,
image and mount edits apply on the next container creation, not a restart of
an existing container. Secret-grant edits are handed to a running computer.

`capabilities.options` feeds Settings > Providers > Use for and Settings >
Budgets, and reads the same connections `images.status` and `voice.status`
resolve from. Its `spending` is `CapabilitySpending`
`{budgets: SpendingBudget[], unavailable?}`: always the `chat`, `voice` and
`images` budgets in that order, each
`{kind, dayUsd: number|null, monthUsd: number|null, spentDayUsd, spentMonthUsd, lines}`.
A `null` limit is no limit. `lines` say what the spend went on:
`teammates` (Hotline Agent turns on per-token keys; ACP teammates bill their
own accounts and are not counted) and `callAssistant` for chat,
`transcription` and `speech` for voice, none for images. Teammates come from
`chat-ledger.json`, the call assistant and voice from `voice-ledger.json`,
images from the image tally (`spending.json`);
`unavailable` is set when either cannot be read. The `stt` and `tts` options are every speech model each
connected provider offers, as the provider lists them (cached for a day; the
provider's default when it cannot be asked), with voices on each speaking
model and the default model first; see `docs/voice.md`. The command may wait on
those lists for up to five seconds; starting a call does not. Voice reads
`settings.spending` when the owner has set it. A room that never set it but
kept an early version's `settings.voice.dayUsd` and `monthUsd` has those as
its Voice limits, and no others.

`images` is an `ImageSettings` object `{provider?: string, model?: string}`,
defaulting to `{}`. An omitted provider selects the first connected provider
that can make images, on its default model; a model is used only with an
explicit provider. An unavailable explicit provider is reported rather than
silently switched. `spending` is a `SpendingSettings` object with a budget
each for `chat` (teammates and the call assistant), `voice` (transcription
and speech) and `images`, each `{dayUsd?: number, monthUsd?: number}`:

```json
{"chat": {"dayUsd": 5, "monthUsd": 50}, "voice": {"dayUsd": 10}, "images": {}}
```

An absent or `null` limit is no limit, and the default is no limits at all.
Each limit must be finite and non-negative; zero turns that budget's paid use
off. The shared `{dayUsd, monthUsd}` earlier versions wrote is still
accepted, as the voice and images limits with chat unlimited, and is stored
in the new shape; mixing it with budget keys is refused. Each object replaces
the whole setting rather than merging its fields. Top-level `null` restores
the whole object's defaults. Image selections must be non-blank strings when
present. Both objects are typed and validated before any key in an update is
written; unknown settings keys retain their existing map semantics. Malformed
stored image or spending overrides remain visible so consumers refuse them
rather than automatically selecting a provider or restoring paid budgets.

`images.status` describes the current selection using the desk's existing
vault connections. It makes no image or provider request and changes no
settings. `available: true` carries the resolved provider and model ids;
`available: false` carries an `unavailable` reason and omits those ids.
`spending`, when readable, is `SpendingSummary` `{dayUsd, monthUsd}`: recorded
usage including reservations for the current UTC day and month, not the caps.
It comes from the same room ledger image generation uses. Invalid saved
spending settings or an unreadable ledger omit that summary and carry a
`spendingUnavailable` reason; image-provider availability is independent of
this spending readiness. A status read never creates a spending file.
Status and image generation read settings strictly: a malformed room JSONL
line or an `images`/`spending` event whose kind is not `setting` refuses the
operation rather than restoring defaults. Status reports `available: false`
with safe `unavailable` and `spendingUnavailable` reasons and omits the
provider, model and usage summary; no raw room content is included.
Credentials and keys are never returned. Owners and the local desk may ask;
companions receive `forbidden`. Test room handles default to unavailable.

Codex subscription images use an existing Hotline ChatGPT sign-in
(`openai-codex`); the only supported model is `gpt-image-2` (also the
default). Grok images use an xAI key or a Grok sign-in (`xai`, offered as
`Grok` or `Grok (subscription)`), with `grok-imagine-image-2.0` by default.
Automatic selection takes a subscription before a paid key, and a
subscription falls back only to another subscription, never to a paid
provider. `available` means configured, not verified image entitlement:
status neither refreshes a login nor calls the provider.

The existing `generate_image` tool uses the Codex backend's internal image
endpoints, with automatic size and quality; aspect is a prompt instruction,
not a guaranteed pixel ratio. It supports up to five PNG, JPEG or WebP
references. It refreshes only Hotline's saved login, never starts a login or
reads Codex CLI credentials. Subscription results return `costUsd: null` and
`billing: "subscription"`; paid-provider results retain their existing numeric
cost. Subscription calls do not reserve or charge the dollar ledger, including
when dollar spending is disabled or exhausted. Malformed settings still refuse.
Upstream subscription limits still apply, and a refusal is retried only
through another subscription. This internal endpoint may change; live compatibility and
account entitlement must be verified separately before relying on it.
The capability lease is checked after login refresh and immediately before
dispatch. A refresh that exceeds 30 seconds asks the caller to try again;
it does not claim the login is invalid or send an image request.

`computer.capacity` reports totals, not free resources, cached for about a
minute per runtime preference. Docker/Podman totals take precedence, then
host totals, then four CPUs and 8 GiB. A stopped installed runtime keeps its
name; `runtime: null` means none installed. `source` identifies the fallback.
`mobile.persona_computer` is advertised by the owner-only `personaComputer`
hello capability. It preserves omitted fields and all image, process-limit,
mount and secret choices; `cpus: null` removes the cap. CPU caps must be
positive multiples of 0.5 no greater than capacity; memory must be whole
MiB/GiB (`4608m` or `4g`), at least 512 MiB, in 512 MiB increments, within
capacity. An empty change is refused. Apple rounds CPU caps upward to whole
CPUs; Docker and Podman retain fractions.

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
`mcp.auth_callback` is for a browser that is not on the desk's machine. The
window on a laptop uses it for a desk on a server: the redirect lands on the
laptop's own `127.0.0.1` and fails to load, so the person pastes the address
of the page it ends on. Only that login's own callback finishes it: the
redirect's scheme, host, port and path, no userinfo or fragment, and the
login's `state`. A wrong address leaves the sign-in waiting for the right one,
and no answer repeats the address, which carries the code. `mcp.auth_status` reports `signed_out`, `pending`, `signed_in` or
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

A teammate's `avatar` is `{hash, by: "self"|"person", updatedAt}`, and is
absent when the teammate shows its initial. The picture is kept under
`avatars/<teammate>/<hash>.png` and never rewritten, so a client may cache it
by hash for good. A teammate sets its own with the `set_avatar` tool;
`persona.update` accepts `avatar: null` to clear it and nothing else for that
field. Neither restarts the session.

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
harmless. `peers.answer_permission` answers a permission card in a peer
thread (`perm:<requestId>` on the pair stream) and is refused once its turn is
over. A message's `receipt` is `sent` when it enters the thread and
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
`{"thread": "<key>"}`, `{"threadId": {"kind", "key"}}`, `{"run": "<runId>"}`,
`{"side": "<sideId>"}`, `{"view": "roster"}`, or `{"schedules": "<personaId>"}`.

| target | snapshot | then |
| --- | --- | --- |
| `"room"` | the room stream's fold | each room event as it lands |
| `{"tape": id}` | that tape's fold | each tape event; `ephemeral` for streaming deltas |
| `{"thread": key}` | that pair's fold | each thread event |
| `{"threadId": {kind, key}}` | that thread's fold, whatever its kind: the teammate's tape for a `dm`, and the stream of that name for a `side`, `pair`, `run` or `call` | each event; `ephemeral` for streaming deltas |
| `{"side": id}` | that side thread's fold, headed by its `side` marker | each event; `ephemeral` for its streaming deltas |
| `{"view": "roster"}` | every living teammate's row | `event` for a changed row, `removed` for a tombstone |
| `{"schedules": id}` | that teammate's jobs and loops | the whole list again as a `snapshot` whenever it changes; `removed` when the teammate is deleted |

An `agent` event on a tape may carry `spoken`, what was said on a call for
it ([Voice calls](#voice-calls)).

A tape subscription also forwards `StreamDelta`s for that teammate, never
written down:

```json
{"sub": 1, "ephemeral": {"type": "agent_delta", "personaId": "ada", "messageId": "m2", "text": "hel"}}
```

`type` is `agent_delta` or `thought_delta`. A delta for another teammate
is ignored. A side subscription forwards `side_agent_delta` /
`side_thought_delta` carrying `sideId` instead of `personaId`, for that side
alone; a tape never receives them. A closed delta channel is not recovered: the durable line
carries the characters anyway. A socket that declared `threads2` is sent
`thread_delta` instead of all four ([Threads](#threads)).

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

## Threads

A thread is any conversation the room keeps: the main conversation (`dm`), a
work thread (`side`, the name it is stored and sent under), a thread between
two teammates (`pair`), a voice call (`call`) and a subagent's run (`run`). One
`ThreadId` names any of them, `{kind, key}`: a teammate's id for a `dm`, the
side, run or call id, and the pair key. [threads.md](threads.md) is the design.

**The subscription's name.** `{"thread": "<key>"}` already meant a pair, and a
phone on an older build still sends it, so it keeps meaning that. A thread of
any kind is a new target, `{"threadId": {"kind": "side", "key": "<id>"}}`: the
contract gains a variant and no existing one changes. `thread` in a command's
params is always a `ThreadId` object, since no old command used the name.

**Negotiation, per connection.** A desk lists `threads2` in its hello
([the phone seat](#the-phone-seat)), and a client that reads the newer shapes
says so with `client.hello {capabilities: ["threads2"]}`. The window's socket
has no hello to read, so `client.hello` answers `{capabilities}` for its seat as
well. The declaration is the socket's, and it is read as each frame is made, so
it reaches subscriptions already open: declare it before subscribing. A socket
that never says it is sent exactly what it was before, and the contract's older
shapes are all still there. The window needs it: a core that rejects the hello or
leaves `threads2` out of its answer is not subscribed to, and the window says it
needs a newer core.

| | without `threads2` | with `threads2` |
| --- | --- | --- |
| a thread's link on its parent's stream and its own | the marker its kind always had: `side`, `subagent`, and `call` | `link`, for every kind, an old marker on disk included (it keeps its id, so it replaces itself) |
| live words | `agent_delta` / `thought_delta` on a tape, `side_agent_delta` / `side_thought_delta` on a side thread, nothing for a run | `thread_delta` on every subscription that has live words: a tape, a side thread, a run |
| pages and snapshots | links as markers | links as `link` |

A `link` is `{kind: "link", id, ts, thread, threadKind, personaId?, title,
state: "live"|"parked"|"closed", end?, outcome?, at?, note?, sessionId?,
backendId?, elapsedMs?, openerId?, openerName?}`; `end` is how a closed thread
ended (`person`, `agent`, `idle`, `stopped`, `done`, `failed`, `cancelled`).
It is rewritten under the same id as the thread goes. A `thread_delta` is
`{type: "thread_delta", thread, messageId, kind: "text"|"thought", text}`, never
written down. The room broadcasts only that; the older four are what the door
turns it into for a socket that did not declare.

**`ThreadSummary`**, one shape for every kind: `{thread, personaId,
withPersonaId?, title?, state, end?, opener?, startedAt, updatedAt, working,
waiting, preview?, outcome?}`. `personaId` is the teammate whose thread it is
(a pair is listed with the first of its two, and `withPersonaId` is the other);
`updatedAt` is the newest line, which is what a client counts unread against;
`working` is a turn running now; `waiting` is a card in it unanswered;
`preview` is the teammate's last words, else the person's last line. A work
thread whose record says live and that the room holds no agent for is listed
`parked`, as the next start would make it.

**Which verb applies to which kind.** A verb a kind has no meaning for is
refused in a sentence, never silently:

| | `dm` | `side` | `pair` | `call` | `run` |
| --- | --- | --- | --- | --- | --- |
| `prompt` | yes | yes | | | |
| `cancel` | the turn | the turn | stops the exchange | | |
| `park`, `close` | | yes | | | |
| `continue` | | yes | resumes the exchange | | |
| `answer` | yes | yes (a permission, or a request) | a permission | | |
| `page`, subscription | yes | yes | yes | yes | yes |

**Seats.** The local desk and an owner may run all of it. A companion phone
may run each verb exactly as it could under the old name, so the rule is by
kind: `thread.prompt` only in a work thread (it speaks to a teammate with
`mobile.prompt`, as before), `thread.answer` in anything but a pair
(`peers.answer_permission` is the owner's), `thread.page` and `{"threadId": …}`
on anything but a call or the voice dispatcher's tape, and `thread.list` without
pairs or calls (`peers.list` is refused it, and a call's preview is what was said). `client.hello` is every seat's. A refusal is
`forbidden`, as ever. [security.md](security.md) has the table and its tests.

A call is `voice.*`'s, a run's lifetime is its teammate's, and nobody answers a
card in either (the kind's `answer` policy is `Nobody`; the card expires). The
main conversation is never parked or closed: its chapters are its lifecycle.

**The older commands are these handlers under their old names.**
`session.prompt`, `session.cancel`, `session.answer_permission`,
`human.answer` (a DM's `thread.answer`, which also finds a card raised in one of
its work threads), `side.prompt`, `side.cancel`, `side.archive`,
`side.answer_permission`, `peers.answer_permission` and `tape.page` run
`thread.prompt`, `cancel`, `answer`, `close` and `page` on the matching
`ThreadId`, and answer what they always did. `side.start`, `side.continue`,
`side.list` and `peers.list` answer their own summaries (`SideThreadSummary`,
`PeerThreadSummary`) and are left as they were; `peers.mark_read` is a pair's
read receipts and stays one too. `voice.*` is still the call's control surface.

## The roster view

Nothing logs this. It is a join of the room stream, each teammate's tape,
and the live sessions. Each row is a `RosterEntry`:

```json
{
  "persona": { … },
  "preview": {"from": "me", "text": "morning", "at": 5},
  "latest": 5,
  "activity": "read note.txt",
  "pin": 0,
  "session": { "personaId": "…", "state": "thinking", … }
}
```

`preview` is absent for a teammate that has never spoken. `from` is `"me"`
for a user line and `"them"` for an agent line; only those two kinds
count. `latest` is that line's `at`, kept beside it so the window can count
unread without opening every tape. `activity` is the title of the tool
still running, only while the session is thinking — absent, not null, when
there is none. `pin` is the teammate's 0-based slot among the desk's pinned
teammates, absent when it is not pinned; the affected rows are sent again when
the `pinnedTeammates` setting changes. `session` is a `SessionInfo` (`state` is `idle`, `starting`,
`ready`, `thinking`, `error`, or `stopped`). While a turn is `thinking`,
`awaitingSubagents: true` says it is open only for subagents it started and
reads as done: the reply is in the conversation and the composer is as it is
between turns (see [sessions.md](sessions.md#a-turn-left-open-for-its-subagents)).
`subagents` lists the runs still going, oldest first. A persona tombstone on the
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

A seat is what a socket may do: a set, not a routing table. A remote socket's
seat comes from the device grant authenticated by Noise. The local desk and paired owner seats
may run every command and subscribe to every target. This includes existing
phone owners; companion grants retain the smaller set below.

### The phone seat

A paired companion's socket is the phone seat: a smaller fixed set of commands
(`Seat::permits` in `crates/hotline-core/src/wire/mod.rs` lists them) and
four kinds of subscription — a tape, a thread (a pair's `{"thread": key}`, or
any thread but a call by `{"threadId": …}`), the roster, and a teammate's
schedules. It never opens the room stream or a run, and it can
neither make, cancel nor quiet a job. It reads a file a teammate sent with
`file.read` and a teammate's picture with `avatar.read`, because the file is part of the conversation it already
reads. It may add a teammate through `mobile.persona_create`, a narrow
create confined in core rather than by what the phone's form leaves out,
and read `backends.list` to know which harness to offer; the full
`persona.create` — reach, path and computer included — is still refused.
It may rename a teammate or change its goal through `mobile.persona_update`,
and remove one with `persona.delete`; `persona.update`, which can reach a
teammate's grants, is refused.
Anything else is refused with `"code": "forbidden"`.

The phone's socket opens with a hello before any answer:

```json
{"type": "hello", "protocolVersion": 2, "desktopId": "…", "mode": "team",
 "capabilities": ["personaCreate", "personaEdit", "schedules", "threads", "runs", "threads2"]}
```

`capabilities` names the optional features this desk supports, so a phone
asks only for what the desk it reached understands. `personaCreate` is
`mobile.persona_create`; `personaEdit` is `mobile.persona_update` and
`persona.delete`; `turnRetry` is `session.retry`, so a phone offers Try
again on a failed turn only where the desk can run it; `schedules` is the schedules view; `threads` is
reading a thread between two teammates the way a tape is read; `threads2` is
the `thread.*` commands, `{"threadId": …}`, and, once the phone says it reads
them with `client.hello`, `link` events and `thread_delta` ([Threads](#threads)). A desk from
before one of these sends it absent, and a phone reads that as "not on this
desk", never as "nothing there"; asked anyway, such a desk refuses the
command as forbidden or the subscription as one it cannot read.

### Pushes

A phone that registered a token with `mobile.push_register` hears each
turn's reply and every card that waits on the person as an Expo push
(`crates/hotline-core/src/push.rs`). A turn is one push however many
bubbles it took: its last reply, sent once the driver is done with the
line. A card goes the moment it is asked. Nothing goes while the desk's
window says the person is at it (`desk.looking`, below). Every push is `mutableContent`, so the
phone's notification service may rewrite it as the teammate's own message.
`data` always names `desktopId` and `personaId`. A card raised in one of the
teammate's side threads also names it as `data.sideId`, and is answered with
`side.answer_permission`, not the teammate's. A card that waits on the
person also names its kind as `categoryId` and its request as
`data.requestId`, which is the id the answer names:

| `categoryId` | `data.requestId` is | Answered with |
|---|---|---|
| `permission` | the request's `requestId` | `session.answer_permission`, or `side.answer_permission` when `data.sideId` is present, with an `optionId` from `data.options` (`[{optionId, kind}]`) |
| `human_action` | the card's `actionId` | `human.answer` |
| `passkey_ask` | the ask's `askId` | `secrets.passkey.answer` |

The answer goes over the phone seat like any other, so an answer from the
notification lands on the tape exactly as one from the conversation.

The window sends `desk.looking {looking: true}` while it has the focus,
is showing and has been used lately, again every thirty seconds at most
while that lasts, and `{looking: false}` on a blur or when it is hidden.
The room holds one `true` for two minutes, so a window left focused on an
empty desk lets the phone hear again. An owner or local desk may send it;
a companion cannot. An open phone already has every tape live, and its own handler
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

## Sealed channel v2

BRO-114's desk/phone interoperability contract. The Rust responder and
fixed-key vectors implement this protocol; Swift uses the same vectors.

- Pattern: `Noise_IK_25519_ChaChaPoly_SHA256`; prologue is the ASCII bytes
  `hotline/2`. The desk static X25519 private key lives in its SecretStore,
  independently of TLS certificates. Each phone holds its own device key.
- Session path: `<public-url>/v2`. Pairing path: `<public-url>/v2/pair`.
  Both are WebSockets over TLS. The pairing path returns 404 unless an
  explicit two-minute pairing window is open. Neither desktop nor served
  listeners expose a PAKE or legacy bearer route.
- Each handshake message is one binary WebSocket message. Message 1 is the
  initiator's `(e, es, s, ss)`; message 2 the responder's `(e, ee, se)`.
  Successful room session payloads are empty. Viewer payloads bind the target as
  described below. For pairing, message 1 carries UTF-8 JSON
  `{"secret":"<b64url>","name":"<device name>"}` and message 2 carries
  `{"role":"owner"|"companion","deskName":"…","deskId":"…"}`. The desk closes after
  message 2. Grant creation and single-use secret consumption are atomic;
  scanning one QR with two devices yields only one grant.
- No wire hello or banner precedes authentication. After successfully reading
  Noise message 1 and validating the room/viewer binding, an unknown or revoked
  session key receives message 2 with `{"error":"device_not_authorized"}` and
  the socket closes. The client may report revoked only after verifying that
  message under the pinned desk identity. Invalid first messages close silently.
  Pending connection limits apply per TCP peer IP, with a separate global
  pending cap, absolute authentication deadline and byte cap. Authenticated
  sessions have independent limits keyed by the proven device identity.
- Each transport message is one binary WebSocket message, at most 65535
  bytes including the Noise authentication tag. Plaintext is `[flag u8]
  [chunk]`; flag 0 continues the current frame and flag 1 ends it. Chunks
  concatenate to exactly one existing UTF-8 JSON wire frame. Reassembly is
  bounded independently of the per-message cap. Text WebSocket messages
  are forbidden. The authenticated hello has `protocolVersion: 2`.
- QR: `hotline://pair?v=2&k=<desk static public key>&u=<public URL>&s=<secret>`.
  `k` and `s` are 32-byte values encoded base64url without padding; `u` is
  URL-encoded. The QR also carries `r` (role), `e` (expiry in Unix milliseconds)
  and `n` (desk name). Desktop links use `hotline://pair?p=<base64url JSON>`;
  [serve.md](serve.md) defines the versioned JSON fields and CLI outputs.
  No bearer token travels in v2 requests or transport.
- The phone validates publicly trusted certificates normally and allows
  self-signed TLS for this pinned Noise desk identity. Identity trust is
  the Noise key, not the certificate. A wrong key fails even with a valid
  certificate; rotating certificates does not revoke v2 grants. Old
  pinned-certificate grants cannot authenticate; the phone must re-pair
  through the sealed QR or link.
- Grants record the device public key and explicit `owner` or `companion`
  role. Missing legacy roles migrate to owner; an empty grant set never
  bootstraps an owner. Companion retains the existing phone allowlist;
  owner has the same command and subscription set as the local desk.
- Fixed-key interoperability vectors are at
  `crates/hotline-core/tests/fixtures/noise_v2.json`: desk/device static keys,
  both ephemerals, and expected handshake and first transport bytes.

Computer-viewer transport uses `<public-url>/v2/computer/{personaId}/ws`.
Noise message 1 carries UTF-8 JSON
`{"purpose":"computer","personaId":"<id>"}`; the desk requires an exact match
with the HTTP target before opening the upstream viewer. Missing, extra or
mismatched fields are refused. A TLS proxy cannot redirect authenticated
controls to a different computer by changing the path. A successful message 2
is empty; rejected device keys receive the authenticated refusal above.
There is no viewer hello. Transport plaintexts are JSON envelopes:
`{"type":"text","data":"…"}` or
`{"type":"binary","data":"<standard padded base64>"}`. Inputs accept text
only. The desk resolves the upstream from its own running computer status;
the viewer address and bearer never leave the desk.

Remote controls require an owner or local desk seat, used by the window and CLI:

| cmd | params | result |
| --- | --- | --- |
| `remote.status` | `{}` | `RemoteStatus` |
| `remote.configure` | `{enabled, host}` | `RemoteStatus`; a served address cannot change |
| `remote.devices` | `{}` | `RemoteDevice[]` with roles and public keys, never secrets |
| `remote.revoke` | `{deviceId}` | `RemoteStatus` after immediate revocation |
| `remote.public_url` | `{url?}` | `RemoteStatus`; keeps an https origin for a tunnel or proxy, or clears it with none or blank; a served desk refuses |
| `remote.pairing` | `{role?}` | `SealedPairing` with id, QR, URI and expiry; owner by default |
| `remote.pairing` | `{id}` | paired device, or explicit JSON `null` while waiting |
| `remote.pairing` | `{id, cancel: true}` | none; ends only the matching invitation |

The socket creating a sealed invitation owns its disconnect cleanup. A new
invitation replaces the previous one; disconnecting an old socket cannot
cancel the replacement. The CLI keeps its socket open until success,
cancellation or expiry. An owner may invoke these controls; a companion may not.
Both listeners mount only v2 pairing, room and computer routes. A stored
device without a public key stays listed and revocable, but cannot
authenticate or receive push notifications. The window labels it as needing
a re-pair. These inactive records do not consume the 16 sealed-device slots.
There is no typed short-code flow.

Pending connections have their own budget: 16 total and 4 per TCP peer IP.
When that budget fills, the oldest pending connection in the exhausted budget
is cancelled and fully released before its replacement starts TLS. Each new
pending connection gets a one-second grace period before it can be evicted,
so incoming bursts cannot immediately cancel every handshake. A single
five-second deadline runs from TCP acceptance through TLS, HTTP, the WebSocket
upgrade and Noise authentication; HTTP keepalive requests cannot reset it.
IPv4-mapped IPv6 addresses share the IPv4 budget. Forwarding headers do not
change admission: the listener does not trust `X-Forwarded-For` or
`CF-Connecting-IP`.

After authentication, room and viewer connections share a separate budget of
16 total and 4 per device, independent of its IP address. An authenticated
connection is never evicted to admit anonymous traffic. The listener allocates
at most 32 connection states plus one accepted socket waiting for an evicted
connection to finish; the OS listen backlog is separate. A device at its limit,
or a full authenticated budget, closes a new upgrade without an application
hello. Clients use their normal reconnect backoff.

The first Noise message is limited to 4096 bytes and WebSocket records to
65535. Reassembly is capped at 32 MiB; the owner wire permits 32 MiB frames,
while companions retain the 1 MiB output frame limit. Both use a bounded
64-frame outbox. These limits never widen a seat or fall back to plaintext.
Rotation prevents a fixed set of anonymous connections from holding all
reconnect slots, but cannot guarantee availability under sustained connection
floods or a proxy that refuses traffic. Public deployments still need edge
traffic controls.

## Voice calls

Voice commands and `{"call":"<callId>"}` subscriptions are available to the
local desk and paired owners. Companions receive `code: "forbidden"`.
An owner hello advertises `voice`, `voiceDirectCalls` and
`voiceListenWhileThinking` when speech and the budget permit an audio or text
direct call, and `voiceTextInput` when text is ready. `VoiceStatus.available` reports desk readiness for the requested mode,
including the dispatcher; additive `directAvailable` reports readiness without
that dispatcher. A direct call can work while desk routing is misconfigured:
it has no dispatcher, since the teammate's own session answers it.
Speech comes from connected providers. By default the dispatcher uses the room's
default provider and prefers its lightweight chat models, excluding speech,
embedding, image and audio model IDs. `settings.voice.dispatcher` can select a
provider and model explicitly; catalogues supply no measured latency ranking.
`settings.voice` also selects speech models and voices; what voice may spend
is the Voice and Chat budgets of `settings.spending`. See
[Voice providers and settings](voice.md).
No extra speech credential is created.

| Command | Params | Result |
| --- | --- | --- |
| `voice.status` | `{inputMode?:"audio"\|"text"}` | `VoiceStatus`: desk/direct availability for the mode, provider/model selections and `budget`, the Voice budget's limits (`dayUsd`, `monthUsd`, absent when none) and what transcription and speech spent; optional `replies`, how teammates' replies on calls were said, by model |
| `voice.call_start` | `{callId,personaId?,streamAudio?,inputMode?:"audio"\|"text"}` | `VoiceCall`: call id, accepted input formats, primary output format, echoed `inputMode`, and optional echoed `personaId` |
| `voice.text` | `{callId,seq,text}` | void; one finalized device transcript on a negotiated text call |
| `voice.audio` | `{callId,seq,index,data,final}` | void; negotiated mono PCM16 at 16 kHz |
| `voice.utterance` | `{callId,seq,mimeType,data,durationMs}` | void |
| `voice.interrupt` | `{callId}` | void |
| `voice.hold` | `{callId,hold}` | void |
| `voice.call_end` | `{callId}` | void |
| `voice.models` | `{}` | `SpeechModel[]`: each of the desk's own speech models, its sizes, credit and licence, and `state`: `available`, `downloading` (with `receivedBytes`), `unpacking` or `installed`, with `error` after a failed download |
| `voice.model_install` | `{modelId}` | `SpeechModel[]`; starts the download in the background |
| `voice.model_cancel` | `{modelId}` | `SpeechModel[]` |
| `voice.model_remove` | `{modelId}` | `SpeechModel[]`; also stops a download under way |
| `voice.transcribe` | `{mimeType,data}` | `VoiceTranscript`: `{text}` |

The `voice.model_*` commands and `voice.transcribe` are the desk's own
hearing ([Hearing on the desk](voice.md#hearing-on-the-desk)). Nothing is
installed until an owner or the desk asks; a download is checked against the
size and SHA-256 Hotline pins for it before anything is unpacked, and a client
follows it by asking `voice.models` again (the window does every half second).
`voice.transcribe` hears one clip outside any call with an installed model:
standard base64 of a mono PCM16 WAV or AAC in MP4, at most a minute. It needs
no provider and no budget, and is refused with a sentence when nothing is
installed. Like every `voice.*` command these are refused to a companion.

`callId` is a client-generated UUID. Omitting `personaId` calls the desk;
including it calls that teammate's existing session, chapter and harness:
each utterance is a turn of its conversation, said into a turn still running
as a steer, with the reply's spoken version said on the call and its written
version shown in the chat ([One brain, two outputs](voice.md#one-brain-two-outputs)).
Clients require `voiceDirectCalls` before sending a target and check its echo
in the descriptor. The core validates the target before replacing an active
call. A direct turn has the same operator origin and standing grants as typed
input, and its replies must carry that call and turn's internal origin.

A reply to a turn said on a direct call is an `agent` event like any other,
whose `text` is the written version, and whose first bubble also carries
`spoken`: the version written to be heard, as the agent wrote it (a line it
wrote before the version included), before it was cleaned for speech. It is
on the tape and on the wire as an optional field, absent on every other
message, so a client that does not know it shows `text`, which stands alone.
The window draws it as a transcript line above the reply. What was actually
said, cut short or not, is the call's own thread.

```json
{"kind": "agent", "id": "m2", "ts": 1760000000000, "text": "The build fails for two reasons: ...", "spoken": "Two things are wrong. I've put both fixes in the chat."}
```

`voice.status` may carry `replies`, read-only counts of how teammates' replies
on calls to them were said, one `VoiceReplies` per model that has written
any, sorted by `model`: `hotline/<provider>/<model>` for Hotline Agent, or
`acp/<adapter>` for an ACP agent, followed by `/<model>` when its session
reports one. Each reply the call said is counted once under `both` (a closed
spoken version and a written one), `spokenOnly`, `unclosed` (a `<spoken>`
never closed) or `untagged` (no spoken version). The field is additive and
absent until a reply has been counted; the desk keeps the counts across
restarts ([voice.md](voice.md#one-brain-two-outputs)).

```json
{"replies": [{"model": "acp/claude-code", "both": 47, "spokenOnly": 1, "unclosed": 0, "untagged": 2}]}
```

Repeating a retained id with the same target and input mode returns the same
call descriptor, including an ended call; changing either is refused. The desk retains the latest 32 call
ids for the life of this process. A different id ends the previous call with
`replaced`. A disconnected or revoked opening connection ends its call. An
ended call needs a new UUID to start again.

`inputMode` defaults to `"audio"`. A client requires `voiceTextInput` before
requesting `"text"`, verifies the echoed mode and `text/plain` input format,
and submits only finalized recognition through `voice.text`. Text is nonblank,
at most 8,000 characters and 32,000 UTF-8 bytes. It uses the same increasing
sequence, pending-turn gate, opening connection and cancellation as audio;
duplicate commits and input from the wrong mode are refused. Text resolves
output only, makes no remote STT request or reservation, and retains TTS,
dispatcher (for desk calls), budget and owner-seat enforcement.

`streamAudio: true` opts into progressive speech output and, on audio calls
when the selected STT adapter supports it, adds `audio/pcm` to the existing
WAV/MP4 input list.
Without that input format, a client retains its whole-clip microphone path.
`voice.audio` chunks contain base64 PCM16 little endian, mono at 16 kHz, at
most 32 KiB decoded per chunk and 20 seconds per turn. Sequence and chunk
indices increase within the call; an empty final chunk commits the turn.
Partial recognition never dispatches work. Hold, interrupt, disconnect and
revocation cancel unfinished microphone input. See [the call contract ledger](voice-calls.md).

Utterances carry standard base64, at most 2 MiB decoded audio and 20 seconds.
WAV must be 16 kHz mono PCM16. MP4 must carry a duration in its media header.
The server checks the WAV sample duration or MP4 header against `durationMs`
(250 ms tolerance). An MP4 STT reservation also uses its byte count at 32 kbit/s,
capped at 20 seconds, when that exceeds the header duration. This is a
conservative estimate, not verification of the encoded audio's duration.
`seq` increases per call. A held call refuses microphone audio. A call still
processing its previous utterance refuses another until it can accept work;
the caller may retry a refused sequence. Calls end after ten minutes without
operator activity. An interrupt discards speech without cancelling a teammate's
turn or the dispatcher's pending text answer; on a direct call it also stops
the reply being said and puts the call back to `listening` while the turn goes
on. A hold also suppresses clips;
teammate replies received while held use their ordinary push. Resuming or
interrupting an unfinished utterance leaves the state `thinking` and refuses
new utterances until that work finishes. The answer is still recorded and sent
as `said`, even when its speech was interrupted.

The call subscription starts with a one-element `snapshot` containing its
`state` event. Updates use the normal `event` envelope:

| Event | Fields |
| --- | --- |
| `state` | `state`: `listening`, `thinking`, `speaking`, `held`, `ended`; optional `reason`; optional `listening: true` on `thinking` |
| `heard` | `seq`, `text` |
| `said` | `id`, `text` |
| `clip` | matching `id`, `index`, `final`, `mimeType`, base64 `data` |
| `delivery` | `personaId`, `eventId`, narrated `text` |
| `card` | `personaId`, `requestId`, `kind` |

An end reason is `client`, `goodbye`, `budget`, `replaced`, `error` or `idle`.
Each clip is independently playable, limited to 2 MiB decoded audio. Progressive
chunks retain one `said.id`, increasing indices, and `final: true` on the last
chunk. A subscriber that misses events
must reconnect; the desk closes that socket rather than silently dropping audio.
Provider selections in `VoiceStatus` are optional when unavailable; `unavailable`
is a sentence explaining what the owner needs to change.


After a nonempty, non-goodbye `heard`, the desk says nothing until there is
an answer: the call is `thinking`, and a client covers the wait with its own
sound (the desktop plays a short blip-blip, repeated while it lasts) rather
than speech. On a desk call the answer is the dispatcher's; on a direct call it
is the teammate's own reply, and the call stays `thinking` between what it says
until the teammate's turn is over, sending `thinking` again every 15 seconds
while it waits. Once the person's words are with the teammate, that
`thinking` carries `listening: true`: the call takes an utterance now, which
steers into the open turn and stops what the call is saying, so a client keeps
its microphone open as on `listening` while nothing plays, and holds what
begins to play while the person is talking. Absent (and from an older desk),
`thinking` has the floor: an utterance is refused until it changes. A
`thinking` with `listening` that a client receives with an utterance of its
own still on the way is from before the desk took it, so it waits for the
desk's next state. An answer streams at sentence boundaries, and one answer keeps
one `said.id`: each sentence sends `said` again under that id with the answer
so far, which a client shows in place of the line it had. Its clips carry on
that id's indices, one whole clip per sentence or, with negotiated progressive
output, several, and the answer ends with an empty `final: true` clip
(`data: ""`) once it is done. A whole line said at once (a narrated reply, a
goodbye) marks its own last clip `final` instead. Clients must queue clips
across successive `said` IDs instead of replacing playback. A direct call's
`said` and `clip` carry only the reply's spoken part; the marker never reaches
them.

A completed teammate reply on a desk call, active and unheld, is narrated and
sent as `delivery`, then `said`, then sentence `clip` events. A direct call
sends no `delivery`: the teammate is the voice. Failed or empty narration
falls back to the reply's first sentence, except a budget refusal, which ends
the call without another paid request. Outside a call, only a reply to a
handoff made by the voice dispatcher may have a summarized push. Every other
push retains the teammate's own text without a narration request. If voice is
unavailable or narration is busy, the original reply remains the push body.
Each `clip.mimeType` describes that clip, including bundled and fallback clips;
`call_start.output` describes only the primary speech adapter.

One failed utterance speaks a bundled “Sorry, say that again.” and returns to
`listening` (or stays held). Three consecutive failed work items end with a
bundled explanation and reason `error`; success resets that count. Budget
failure ends immediately with its bundled line. Only a budget the call pays
into can end it: transcription and speech on a provider that charges spend
the Voice budget, and a call assistant billed per token the Chat budget. A
call heard, spoken and answered for free runs whatever the budgets say. At
`voice.call_start` a call whose paid part's budget is spent or zero is
refused with a sentence naming it ("The Voice budget for today is spent.
Raise it in Settings › Budgets."); during a call it ends with reason `budget`
when a reservation is refused. Failed goodbye synthesis uses
a bundled “Goodbye.” and still ends with reason `goodbye`. The whole-utterance
farewell bypass requires at least 400 ms of audio and one byte per millisecond;
shorter or sparser clips follow the dispatcher path. This size/duration check
does not classify noise in a sufficiently long clip. Clients must drain
final queued audio for `goodbye`, `budget`, and `error`; `ended` means the desk
will produce no further clips.

The dispatcher can read roster/state and a conversation tail, send text through
`session.start`/`session.prompt`, and list/create/cancel schedules. A separate
command allowlist refuses approval, credential, file and administration commands.
Action tools are offered only on turns driven by a new utterance, never on
narration. Conversation tails and narrated text are bounded, escaped untrusted
blocks. Each utterance can queue at most three text handoffs. A queued result
means startup is pending, not that the task has landed; startup failures produce
a spoken notice or normal push. Handoffs queue behind ongoing teammate work.
Their user event IDs carry the opaque `voice:` prefix for reply provenance.

Voice schedules require the target teammate's live background-work grant,
including at firing time. They must be one-shot `schedule` jobs, with no `every`
or `quiet` field, and are recorded with `operatorCreated: false`. A call retains at most 32 created job IDs and can cancel only jobs it created.
Ordinary operator-created schedules retain their existing behavior.
Approval requests arrive as `card` and must be answered in the existing UI.
The dispatcher has no persona: its `voice-dispatcher` tape is hidden from the
roster and rail, but indexed by `search.all` and `search.thread`.

A direct call has no dispatcher and pays for none: its turns are the
teammate's, metered by the session against Chat on a per-token key, as typed
turns are.

Speech attempts and conservative dispatcher estimates are reserved before a
request so cancellation or a lost response cannot erase their cost. Reservations
serialize across calls and push narration, and cannot exceed the remaining cap.
Dispatcher estimates count a third of the request's bytes (prompt, history,
preamble and tools) as input tokens plus the request's output limit; reported
usage then settles the reservation to the actual cost, up or down, and a call
without usage keeps it (see [voice.md](voice.md#the-ledger)).
These are spending guards, not provider invoices. Each fallback attempt is charged separately.
An unavailable or exhausted ledger stops work; a bundled spoken system
line can be played without a further paid request. A whole-utterance
farewell such as “goodbye” bypasses the dispatcher model.
