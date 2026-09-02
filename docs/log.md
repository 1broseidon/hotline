# The log

Everything the room remembers is an event on a stream. A stream is an
append-only JSONL file — one event per line — folded by `id` when it is
read. `Log` in `crates/toad-core/src/log/` is the only door to every
stream, and the only writer. Opening a log touches nothing: a stream's
file is made when something is appended to it.

Three streams, one rule for all of them:

| Stream | File | One per |
| --- | --- | --- |
| `Room` | `room.jsonl` | room |
| `Tape(id)` | `transcripts/<id>/<epoch>.jsonl` | teammate |
| `Thread(key)` | `threads/<key>.jsonl` | pair |

A subscriber is handed every event appended after it asked. History is
`Log::load`, not replayed on subscribe. Publish happens after the bytes
are on disk, never before.

## Fold by id

Later lines win by `id`, and each id keeps the place it first appeared. A
tool call that moves from pending to completed is one line superseding
another, not a second entry. A line with no `id` is skipped. A torn final
line from an unclean exit is skipped.

`Log::compact` rewrites the file the stream is currently written to with
that fold. A fold that changes nothing skips the write. A tape's older
segments are closed history and are left alone, duplicates and all.

## Tombstones

A delete is not a new kind. It is the same kind and id again with
`"deleted": true`. The fold finds the tombstone instead of what it
replaced, which is also what makes a delete something a mirror can ship
rather than an absence it has to notice.

The roster is every `persona` event that is not deleted, in the order the
teammates first appeared. A setting tombstone is not an override: that
key's default stands again. A credential tombstone is not a credential.
A schedule tombstone is not a job.

## The tape

One segment per owner epoch, `transcripts/<id>/<epoch>.jsonl`. The open
segment — the one an append lands in — is the highest epoch on disk, or 1
for a tape that has none yet. The legacy flat file `transcripts/<id>.jsonl`
stands in for epoch 1 only when `1.jsonl` is not already there.

A reader never moves a flat file. The first write relocates it into
`1.jsonl` by rename and keeps its bytes. A writer that finds both the
flat file and `1.jsonl` refuses rather than guessing which is the tape.

This layout is the previous Toad's, spelled the same way on purpose. The
workspace enables `serde_json`'s `preserve_order` so a line read and
written back keeps the key order it was written with. Tests pin a tape
this writes as byte-for-byte the file that Toad's `transcript.ts` writer
produced for the same events, including after compact. Importing a data
directory copies tapes unchanged.

`Log::append` on a tape answers which epoch, byte offset and bytes
landed, newline included. Offsets add up: the next write starts where
the last one ended.

## Threads

A thread is a stream like any other, minus the epochs: one file per pair,
`threads/<key>.jsonl`, and no segments. The key names the two
participants sorted by UTF-16 code unit, `ada~bob`. A key that does not
spell itself the same way again — unsorted, three-sided, or an id holding
`~`, `/` or `.` — is refused rather than written somewhere surprising.

Beside the stream sits a JSON sidecar, `threads/<key>.json`. It is not
events: it is a record of who is in the room, rewritten in place through
a temporary file. The bytes the tests pin against the previous Toad are
`version` 1, the two ids (`a`, `b`), `sides`, `sessions`, `createdAt`,
`updatedAt`, and optional `labels`. The file is read as free-form JSON so
a field a newer build added is not dropped on the next label write. A
sidecar whose `version` is not 1 is ignored as a sidecar. A conversation
exists from the moment its sidecar is opened, whether or not anybody has
said anything yet; listing threads reads the sidecars, not the streams.

A label for a side the roster cannot resolve is written onto an existing
sidecar. No sidecar, no invented one.

## The room stream

`room.jsonl` holds the roster, the settings, the jobs that will wake a
teammate later, and the fact of each credential. One file, no epochs. The
folds are `room::roster`, `room::settings` and `room::schedules` in
`crates/toad-core/src/room.rs`; the vault writes the credential events.

A setting nobody has set is the default: `defaultBackendId` is `"pi"`
(Toad Agent), `chapterIdleHours` is `8`, and `mcpServers` is an empty
list.

### `persona`

The teammate's record as the contract serializes it, with `"kind":
"persona"` beside it. `id` is the teammate's. Always present:

| field | |
| --- | --- |
| `id` | the teammate |
| `name` | |
| `goal` | |
| `backendId` | |
| `cwd` | |
| `mcpPolicy` | `{mode, serverIds}` |
| `sessionCheckpoints` | `{backendId, sessionId}[]` |
| `createdAt`, `updatedAt` | milliseconds |

Present when they were set: `node`, `face`, `team`, `reach`
(`"workspace"` or `"machine"`), `modelId`, `modeId`, `harnessOverride`,
`hopNotice`, `webSearchPolicy`, `computer`, `subagents`, `lastSessionId`.
A tombstone is `{"kind": "persona", "id": "…", "deleted": true}`. An
event that does not read as a `Persona` is skipped rather than fatal.

### `setting`

One preference. `id` names it, `value` holds it:

```json
{"kind": "setting", "id": "chapterIdleHours", "value": 2}
```

A tombstone is `{"kind": "setting", "id": "chapterIdleHours", "deleted": true}`.

### `credential`

The contract's `Credential` under that kind. The secret is never here.

```json
{
  "kind": "credential",
  "id": "…",
  "providerId": "openai",
  "credentialKind": "api_key",
  "label": "personal",
  "revoked": false,
  "createdAt": 1,
  "updatedAt": 1
}
```

A tombstone is `{"kind": "credential", "id": "…", "deleted": true}`.
Revoking writes the same fields with `revoked: true`; the secret stays
until delete takes it.

### `schedule`

A job, as the contract serializes it, with `"kind": "schedule"` beside it
— that slot is the stream's. The job's own kind (`schedule` once, `loop`
every interval) is recovered from `every` being present. Jobs still
waiting to fire, soonest first:

```json
{
  "kind": "schedule",
  "id": "…",
  "personaId": "ada",
  "when": 1700000001000,
  "prompt": "Check the order",
  "quiet": true,
  "nextAt": 1700000001000,
  "createdAt": 1
}
```

A loop carries `every` (milliseconds) instead of `when`. `quiet` is stored
only when true. A tombstone is `{"kind": "schedule", "id": "…", "deleted": true}`.
The clock that fires these is [sessions.md](sessions.md).

## Startup settle

Opening the room folds every living teammate's tape, and every thread
beside it, before anything is served from either. A permission card left
open by the last process is a button nobody is behind, so it is
superseded with `decision: "expired"`;
a `human_action` card still `pending` is the same fact and is superseded
with `status: "expired"`. Then the tape is compacted and the search index
is synced, because the fold just rewrote files and a tape written by the
importer or the previous Toad has never been indexed here at all. The idle
chapter sweep and the scheduler's clock start on the same open; those are
[sessions.md](sessions.md).

## The vault

`<root>/vault/secrets.json` is a JSON map from credential id to secret.
On Unix the directory is `0700` and the file is `0600`, written through a
temporary file and a rename so no reader ever sees half of one. Opening
the vault creates nothing; the layout is a write-time obligation. A
symlink where the directory or the file should be is refused rather than
followed. On Windows, making the directory private is not built, and a
write is refused rather than pretending.

Create writes the secret first, then the room event. Delete takes the
secret first, then the tombstone. `list` is the room's metadata in
creation order; `provider_keys` is one usable API key per provider — the
first created wins — skipping revoked rows and rows whose secret is
missing.

## The search index

`index.sqlite` is FTS5 over every teammate's messages and chapters. The
JSONL is the record; the index is a derivative, rebuilt from the tape
whenever they disagree, and safe to delete. A missing or damaged index
answers no hits and never costs a record.

The one connection that creates the file is `store::search::Indexer`, in
the process that owns the tapes. A question is asked over a read-only
connection that creates nothing. Live appends are offered to the indexer
as they land; import calls `sync`, which re-reads a tape whose size or
mtime differs from the stamp last written. A schema change is a new
`CREATE` and a rebuild — nothing migrates this file.

Two things are indexed. Messages answer "where did we say X". Chapters —
title, note, tags — answer "what was that thing we did in June". Chapter
hits come first. Each word becomes a quoted prefix term; the words are
ANDed, then ORed if nothing matched.

The wire that subscribes to these streams is [wire.md](wire.md). What a
session writes onto a tape is [sessions.md](sessions.md).
