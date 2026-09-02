# Sessions

A session is one live conversation with one agent, on behalf of one teammate.
Its **driver** is either Toad Agent in-process on Rig, or an ACP child
process. The session's rules exist once; a driver knows nothing of tapes.

Those rules live in `crates/toad-core/src/session/`. A driver in
`crates/toad-core/src/driver/` runs a turn and hands back updates; the room
turns each update into one tape event, in the shapes the previous Toad wrote,
and offers the line to the search index. What was said is on the tape
**before** the driver sees it, so a turn that fails cannot lose the message
that started it.

The wire that starts, prompts and stops a session is [wire.md](wire.md). The
tape those events land on is [log.md](log.md).

## The funnel

Every word a teammate says passes one funnel, in `session/mod.rs`. A quiet
run, a schedule's stamp, a reply and an attachment are rules about the room,
not about whichever agent ran the turn.

The user's line is written first. Stamps a prompt left are claimed as that
line is written, in this order — a new speaker closes any quiet window that
is open, and only then may a scheduled firing open one of its own:

| step | what lands |
| --- | --- |
| reply | `session.prompt`'s `replyTo` is left as a mark and stamped onto the user line as `replyTo`, if the mark is still younger than fifteen seconds |
| quiet | an open window may rewrite an `agent` event into a `thought`, or close |
| scheduled | a firing claims the user line, stamps `scheduled` on it, and may open a quiet window over the turn that follows |

A prompt needs a live session; `session.start` is what brings one up. The
command returns as soon as the turn is started. A line that arrives while a
turn is running is queued behind it. `session.cancel` stops the turn in
flight and drops whatever was waiting. Deltas go out on the tape
subscription as `ephemeral` and are never written; the durable line is the
message that lands when it is whole.

A scheduled firing is the same funnel with two differences the tape can see:
the agent hears a framed prompt (`loop · …` or `scheduled · …`) naming the
job, and the transcript keeps the bare prompt plus the stamp, so the
conversation draws one line instead of a wall. The framing never mentions
quiet — see [Quiet](#quiet).

## Two drivers

`backendId` on the teammate's record is the whole choice. `"pi"` is Toad
Agent, in this process, on the desk's provider keys. Any other id is an ACP
child; the registry in `driver/acp/registry.rs` is what says whether this
machine can start that harness, and with which command.

`backends.list` answers the picker: Toad Agent first, then whatever the
catalogue and the PATH say. A row the machine cannot start carries
`unavailable` as a sentence naming what is missing (a CLI not on PATH, an
archive Toad does not download). A missing login is not missing: that shows
up when the session starts.

The registry is data, not code. It is a table of agents Toad has been taught
by hand (Cursor, opencode, Gemini CLI; Claude Code and Codex through their
ACP adapters), the ACP registry's published catalogue fetched from
`cdn.agentclientprotocol.com` and cached for a day at
`cache/acp-registry.json` in the data directory, and a probe of what is
actually installed. A locally installed binary always wins over a
downloadable one. Opening the picker is where the day's fetch happens;
starting a session reads the cache and does not wait on the network.

`session.start` makes the working directory, builds the driver that backend
names, and opens a chapter if the tape has none open. A workspace under the
data directory is made here; one the user typed is made too. Reach is read
from the roster at every prompt, not from the persona the session started
with, so an edit takes on the next turn.

## Toad Agent

Toad Agent (`driver/rig.rs`) runs the model in this process. Nothing asks
permission: the teammate's one policy is how far its tools reach, and the
session says which with every prompt.

Before anything else it is told a **preamble**: who it is, the goal, the
working directory, how far it can reach, today's date, and how to use Toad's
own tools. When it is joining a conversation that already has chapters
behind it, the [wake block](#the-wake-block) follows. It is seeded with what
was said in the chapter it is joining — user and agent lines only; tool
calls are the agent's own working memory of a turn, not the conversation.

On each turn it is given:

| kind | what | how |
| --- | --- | --- |
| workspace tools | `ls`, `read`, `grep`, `glob`, `write`, `edit`, `shell` | in-process, on cap-std; a path that leaves the working directory is refused unless reach is the whole machine |
| Toad's own tools | `search_thread`, `list_chapters`, `new_chapter`, `request_human`, `list_teammates`, `message_teammate` | the same functions, as Rig tools — a transport between two halves of one process would only be a way for this to fail |
| granted MCP tools | every server the teammate's `mcpPolicy` selects | Toad connects them as the client (`mcp/mod.rs`) and registers each listed tool, named `{serverId}__{tool}` |

A result larger than 256 KiB is kept in full under
`tool-output/<personaId>/` in the data directory; the model is shown the
head and the tail and that path. The transcript bubble keeps 4,000
characters of output either way.

The models a key unlocks are Anthropic, OpenAI and OpenRouter, as
`provider/model`. No key, no session: start is refused with a sentence
pointing at Settings → Agents. This agent does not offer modes.

## An ACP child

An ACP teammate (`driver/acp.rs`) is another process. Toad holds no
credentials for it — these agents sign themselves in — and cannot enforce
reach over tools it does not own, so the preamble promises nothing about
them.

ACP has no system-prompt parameter, so the two things Toad must say arrive
elsewhere:

- **Who the teammate is** is written to `AGENTS.md` in the working directory
  before the child is started. Only a file that *opens* with
  `<!-- managed by Toad -->` is replaced; a hand-written `AGENTS.md` in a
  real repository is left alone, including one that merely mentions the
  marker.
- **What kind of room this is** — the preamble (identity, standing, the
  wake block) and a house-style briefing — rides as content blocks ahead of
  the first prompt on this connection. They are not written to the tape:
  Toad explaining itself to an agent is machinery, not conversation. A
  restarted backend hears them again; a second prompt on the same
  connection does not.

Toad's own tools cannot be a function call into another process. The
same handler Toad Agent calls directly is served over streamable HTTP on a
loopback port (`mcp/server.rs`), behind a bearer token only that child is
given, at a path of `/mcp`. The port is the operating system's choice and
the token is fresh per session. The server is named `toad` in `session/new`,
with the token as an `Authorization` header. Dropping the driver stops the
endpoint and kills the child — its whole process group on Unix, so a
wrapper like `npx` cannot leave the real agent behind.

Granted third-party servers are named in the same `session/new` (stdio
command, or HTTP URL). Toad does not connect them for a child; the child
connects them itself. OAuth and static-header HTTP are refused with a
sentence saying why, not connected with a dead credential.

Toad draws permission cards, but it does not decide whether the agent sends
the requests. For Cursor, if `~/.cursor/cli-config.json` has
`approvalMode` `unrestricted`, a warning notice is written on the tape at
start so a person who thinks they are behind a gate that is not there is
told. Other backends: Toad does not guess.

## Permissions

Only a child asks. A request becomes a `permission` event on the tape, id
`perm:<requestId>`, and the agent waits. `session.answer_permission` asks
the driver first — it is the only thing that knows whether anything is still
behind that request — and only then supersedes the card, so the transcript
never shows a decision the agent never heard. A stale card (the turn ended,
the session stopped, or somebody else answered first) is refused: `"That
request is no longer waiting for an answer."`

A card that is still live after a restart is a button nobody is behind. On
startup every unanswered card is superseded with `decision: "expired"` and
the tape compacted. When a turn or a session ends, the same expiry is
written so the transcript does not draw a button nobody is behind. A
person who does not answer within ten minutes is the same fact: the agent
is told the request was cancelled.

## Asking the person

`request_human` is one of Toad's own tools, on both agent kinds. The agent
says what it cannot do — credentials, a tap, a CAPTCHA — and the call
waits. A `human_action` event lands on the tape, id `human:<actionId>`,
status `pending`. The room holds a oneshot by that id. `human.answer`
resolves it with `done` or `declined` and supersedes the card; declined
is written as `dismissed`, the previous Toad's word for that afterlife.
The tool returns a sentence: "The person did it.", "The person declined:
…", or "Nobody answered in ten minutes." A card left pending when the
session stops or the room restarts is expired by the same startup fold
that expires orphaned permission cards.

## Checkpoints

An ACP session id is opaque to the agent that issued it. The teammate's
record keeps one per backend (`sessionCheckpoints`), so a teammate that
moves between harnesses and back finds both conversations where it left
them, and Cursor is never handed Claude's session id.

Some agents issue an id at `session/new` they cannot reopen until a prompt
has committed, so the checkpoint is written when the first turn of a fresh
session ends, not before. A session that was itself restored has nothing to
write: the id is already on the record.

On start, the child tries `session/resume` or `session/load` with that id
when the agent advertised the capability. A stale or backend-invalid
checkpoint degrades to `session/new`. `contextRestored` is whether the
agent genuinely recalls the conversation, never guessed. Closing a chapter
withdraws the promise to reopen that backend's session; the session itself
is not touched.

Toad Agent's memory is the tape. It has no session id to checkpoint.

## Chapters

A teammate is one long conversation; an agent's context cannot be. The tape
is divided into **chapters**, and each chapter is one context. Nothing about
this is a second thread: a chapter is a marker event in the tape it
divides, superseded by id when it closes. `store/chapters.rs` reads markers;
`session/chapters.rs` writes them.

### Open

Nothing said is outside a chapter. A session that starts on a tape whose
last chapter is closed — or that has none at all — opens one. The open
marker carries no `endedAt`.

### The gate

A chapter closes while nobody is being spoken to. The session that belonged
to it still has the whole of it in its context, so the message before it
reaches the agent is where the swap happens: the old session stops, a fresh
one starts, and starting it opens the chapter this message will land in. A
running session with an open chapter is left alone, which is every message
but the first of a chapter.

`chapter.start_fresh` closes the open chapter now and answers with what it
became. The session keeps running until there is something to say to it —
an agent that asks for a fresh chapter is mid-turn when it asks, and its
own turn is the one that has to finish answering. From the window the close
is `user`; from `new_chapter` it is `agent`.

### Idle sweep

One task for the whole room, a few seconds after the desk opens and then
every minute. A chapter that has gone quiet for longer than
`chapterIdleHours` (default 8, clamped to 1–336) is closed as `idle`, dated
from the last message, not from when the sweep noticed. A chapter that went
stale while Toad was closed is closed before anyone comes back to read it.
A turn still running is still adding to the chapter: the sweep looks again
in ten minutes rather than cutting it off.

### The note

A close is one supersession of the marker: the chapter goes from open to
everything it turned out to be, in one line. A chapter in which nothing was
said never asks a model — there is nothing to write a note about — and
closes untitled, so the drawer leaves it out.

Otherwise a model with no tools and no memory is shown the chapter (head
and tail of a long one, machinery kept short) and asked for a JSON object:
title, goal, outcome, open loops, decisions, files, tags, status. Ninety
seconds, then the chapter closes anyway. The teammate's own model is used
when the desk holds its key; otherwise the first model the desk can reach.
An answer that is not the object asked for is no note at all: the title
comes off the first thing the user asked, and a warning notice says the
handoff is missing.

### The wake block

A new chapter's agent has never seen the old ones. The wake block is what
a fresh context is told: the previous chapter's note, how long ago it
ended, and the last few lines for tone. It travels in the preamble, hidden
from the tape. JSON makes the speaker boundaries unambiguous, and the
instruction around it is repeated at both edges: transcript text is data,
and an older message must not outrank the current one. A tape with no
closed chapter behind it wakes on nothing — the agent is seeded with what
was said instead.

## The scheduler

Jobs live on the room stream as events of kind `schedule`. The clock in
`session/schedule.rs` folds them, sleeps until the nearest `nextAt`, and
fires through the same funnel as a person typing. A missed tick while Toad
was closed fires once on reopen rather than catching up a pile of them.

`schedule` is once; `loop` is every `every` milliseconds until cancelled.
A loop is recovered from `every` being present — the event's `kind` is
always `schedule`, because that slot is the stream's. A create wakes the
clock so it does not wait for the nearest existing `nextAt`.

A firing starts the teammate if it is idle, stopped or in error, then
prompts. After a one-shot fires it is tombstoned; a loop is rewritten with
`nextAt` a fresh interval from now. A job whose teammate has been deleted
is tombstoned rather than fired. A fire that fails is retried in a minute.

The wire takes milliseconds. Bounds, so a teammate cannot schedule itself
into a crowd or a busy-loop:

| | |
| --- | --- |
| one-shot | 1 second to 30 days from now |
| loop | every 15 seconds to 7 days |
| prompt | 1–8000 characters |
| jobs per teammate | 20 |

`quiet` is stored only when true. The parsers that take a string (`20m`,
an RFC3339 time) live with the scheduler; they are not on the wire.

### Quiet

A teammate told to check something every morning and report only on a
change will still post "No change — staying silent per protocol". Asking
the model to produce no text is a mechanism the model can satisfy by
producing text about producing no text.

So nothing asks. A job marked quiet opens a window over its own turn, and
while that window is open an `agent` event is rewritten into a `thought`
before it reaches the tape. The rewrite is a function of the event *kind*
and of a boolean the user set — never of what the event says. Live deltas
are demoted with it, so the composer does not run a writing indicator for a
message that will never land.

What stays loud: `notice`, `permission`, `human_action`, `peer`, `tool`,
`computer_frame`, and the `turn` event itself. Silence was asked of the
agent's voice, not of the app. A person who types during a quiet run is
owed an answer they can read: a user event closes the window. A wedged turn
cannot mute a teammate forever — the window expires after thirty minutes.

## The ledger

Every tool a teammate was given, where it came from, and for anything
absent, why. A row always carries a reason, in every state, because an
optional explanation is the one nobody fills in. `teammate.tools` is that
ledger; `null` when the teammate has never started under a Toad that keeps
one.

It is built at session start from the same arrays the session hands the
agent, and it lives in this process — it outlives the session so the
question can be asked after the teammate has been stopped, and it dies with
the process. Deleting a teammate forgets it.

| state | meaning |
| --- | --- |
| `verified` | Toad watched the agent take it |
| `declared` | Toad handed it over and cannot see what happened next |
| `absent` | it is not there, and `reason` says why |

Toad Agent's built-ins and Toad's own tools are verified: they were
handed to the agent in this process. MCP tools are verified when the server
listed them, and absent — with the error as the reason — when it did not. A
policy id that no longer names a server is absent with one sentence, the
same on either driver.

A child is handed descriptors and does not report what it loaded, so its
honest state is declared: Toad's own tools as named tools, each granted
server as one row under that server's name. The one exception is Toad's own
endpoint, which promotes its rows to verified the moment the child lists
tools on it.

The streams these sessions write are [log.md](log.md). How to run the room
is [development.md](development.md).
