# Sessions

A session is one live conversation with one agent, on behalf of one teammate.
Its **driver** is either Toad Agent in-process on Rig, or an ACP child
process. The session's rules exist once; a driver knows nothing of tapes.

Those rules live in `crates/toad-core/src/session/`. A driver in
`crates/toad-core/src/driver/` runs a turn and hands back updates; the room
turns each update into the tape events it is, in the shapes the previous Toad
wrote, and offers the line to the search index. What was said is on the tape
**before** the driver sees it, so a turn that fails cannot lose the message
that started it. Toad Agent's live history keeps the user's line on failure
too, the same as on cancel, so a retry still has the question.

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
turn is running is queued behind it. Joining that queue and claiming an idle
driver are one decision under one lock, so a line cannot be filed behind a
turn that has already stopped coming back for it. `session.cancel` stops the
turn in flight and drops whatever was waiting. Deltas go out on the tape
subscription as `ephemeral` and are never written; the durable line is the
message that lands when it is whole.

### Pacing

A reply is chat or it is a note, decided by one function on the text
(`session/pacing.rs::paced`), never by the model's mood. Chat is one to four
bubbles. A reply is a note when any of these holds: the first non-empty line
is a markdown heading (`#` to `######`, then a space); after merging there
are more than four units; the text is longer than 1,200 characters
(`CHAT_CHARS`); any line outside a fence starts with `|` (a table). A stub
shorter than 60 characters joins its neighbour, and a unit that ends with
`:` joins the next, so "Here's the fix:" and a fence stay one bubble.

A note is one `agent` event with a `title`; chat is one `agent` event per
bubble, ids `{id}` then `{id}-2`…, same timestamp. The model said one thing:
history rejoins consecutive agent events with a blank line, and a note as
`# {title}\n\n{body}`. Both agent kinds and peer threads pass the same
funnel, so they get the same pacing. The house style in the preamble tells
the agent this rule, in words it can act on.

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
starting a session reads the cache and does not wait on the network. A
cached stamp from the future is not the day's. An adapter row is
unavailable until both the client CLI and its launcher (`npx`, say) are on
PATH.

`session.start` makes the working directory, builds the driver that backend
names, and opens a chapter if the tape has none open. A workspace under the
data directory is made here; one the user typed is made too. Starting a
teammate, and the chapter gate in front of a message, share one lock per
teammate: two callers cannot spawn two agents and open two chapter markers
on one tape. Reach is read from the roster at every prompt, not from the
persona the session started with, so a turn already running sees a new
wall. A change to reach, or to anything else the driver is built from,
also restarts the session; see [Reattaching](#reattaching).

## Toad Agent

Toad Agent (`driver/rig.rs`) runs the model in this process. Nothing asks
permission: the teammate's one policy is how far its tools reach, and the
session says which with every prompt.

Before anything else it is told a **preamble**: who it is, the goal, the
working directory, how far it can reach, today's date, how to use Toad's
own tools, and the house style — chat or a note, [Pacing](#pacing). When it
is joining a conversation that already has chapters behind it, the
[wake block](#the-wake-block) follows. It is seeded with what
was said in the chapter it is joining — user and agent lines only; tool
calls are the agent's own working memory of a turn, not the conversation.

On each turn it is given:

| kind | what | how |
| --- | --- | --- |
| workspace tools | `ls`, `read`, `grep`, `glob`, `write`, `edit` | in-process, on cap-std; a path that leaves the working directory is refused unless reach is the whole machine, except a read under the teammate's own `tool-output` directory |
| shell | `shell` | in-process. Machine reach is a command in the working directory with no wall. Workspace reach may read the machine but may only write the working directory and a private `/tmp`. Network stays on: agents install things. The wall is kept per OS, or the tool is not offered. |
| Toad's own tools | `search_thread`, `list_chapters`, `resume_chapter`, `new_chapter`, `request_human`, `list_teammates`, `message_teammate`, `schedule`, `loop`, `list_schedules`, `cancel_schedule` | the same functions, as Rig tools — a transport between two halves of one process would only be a way for this to fail |
| granted MCP tools | every server the teammate's `mcpPolicy` selects | Toad connects them as the client (`mcp/mod.rs`) and registers each listed tool, named `{server name as a slug}__{tool}` |

A shell that cannot see `/usr`, the toolchains under the home directory, or
the package caches cannot build anything, which is why those reads are
allowed when the file tools refuse them. Workspace reach for `shell`:

| OS | workspace reach |
| --- | --- |
| Linux | `bwrap` is the parent of `sh`: the root is read-only, the working directory is bound on top, `/tmp` is a private tmpfs. Missing `bwrap`, or a `bwrap` that cannot create a sandbox, omits the tool and the ledger says why. Ubuntu 24.04's AppArmor restriction on unprivileged user namespaces is the usual reason a present `bwrap` still cannot sandbox; the profile below lifts it for `bwrap` alone. |
| macOS | `sandbox-exec` with a Seatbelt profile that allows everything and denies `file-write*` except under the working directory, `/tmp`, `/private/tmp`, `/dev`, and `$TMPDIR`. Built, unproven on a Mac until George runs it. `sandbox-exec` is deprecated by Apple and still ships. |
| Windows | no confinement Toad can ship, so the tool is not offered; the ledger reason says to give the teammate machine reach. Machine reach keeps `cmd /C`. |

On Ubuntu 24.04 and later, `bwrap: setting up uid map: Permission denied`
means the kernel's `apparmor_restrict_unprivileged_userns` is on. Ubuntu's
own answer is a profile that names the program allowed to make a user
namespace, the shape it ships for 1Password and the browsers. Put this at
`/etc/apparmor.d/bwrap` and load it with `sudo apparmor_parser -r
/etc/apparmor.d/bwrap`; the next teammate start offers the shell again.

```
abi <abi/4.0>,
include <tunables/global>

profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
}
```

A granted stdio server is spawned in its own process group on Unix, so a
launcher like `npx` does not leave the real server behind when the session
stops. A result larger than 256 KiB is kept in full under
`tool-output/<personaId>/` in the data directory; the model is shown the
head and the tail and that path. Under workspace reach the teammate can
read that directory, and nothing else outside the working directory. The
transcript bubble keeps 4,000 characters of output either way.

Anything quoted out of a conversation goes in front of a model inside a
fence (`fence.rs`): `search_thread` and `list_chapters` results, the wake
block's last messages and previous handoff note, the user lines a resumed
chapter is nudged with, a colleague's message, and the chapter transcript
the note model reads. The conversation is data, and the one string it must
not spell is the tag that closes its fence. Every `<` in the body is
rewritten as `\u003c` — the same character to anything parsing JSON, and
no character at all to anything scanning for a tag — so nothing quoted can
close the fence early, whichever fence it is in.

The models a credential unlocks come from the model catalogue
(`crates/toad-core/models.json`, a filtered snapshot of models.dev — see
[development.md](development.md#the-model-catalogue)) for the providers
`models.rs` wires: Anthropic, OpenAI, OpenRouter, Google, xAI, Groq,
DeepSeek and Mistral as API keys, and GitHub Copilot and ChatGPT
(`openai-codex`) as subscription logins, as `provider/model`. A saved
`enabledModels` filter narrows what is offered, never the model a
teammate is on.

GitHub Copilot's picker is that catalogue cut to the models the signed-in
account can run. The list is fetched at sign-in (`GET {api}/models`,
through Rig) and stored as `models.json` beside the login. A fetch that
fails leaves the login in place and no list, with a notice that Refresh
on the provider's own page under Settings → Providers retries. Refresh re-reads the list, which is
also how a login made before this file existed, or a model newly enabled
on the account, lands in the picker. Toad does not POST
`/models/{id}/policy {state: "enabled"}` after login the way pi does, so
a model the account lists as policy-disabled is simply not offered.

A turn runs on the teammate's own model when the desk
still lists it, else the room's `defaultModelId`, else `lastModelId`,
else the newest model the keys unlock. An ACP teammate's `modelId` is
instead what its last session reported, written when a session starts or
its model changes, so the band names a model before the child is up; one
that has never run shows its harness's name where the model will be. A turn never
starts a login: missing or unrefreshable tokens fail with a sentence
asking for sign-in. No credential, no session: start is refused with a
sentence pointing at Settings → Agents. This agent does not offer modes.

A model that lists efforts offers an Effort picker. The value lives on
the persona as `effortId` and is sent on every request through Rig's
`additional_params`. A model switch keeps the effort only if the new
model lists it; otherwise it clears. Which clients carry an effort, and
what is sent:

| client | body |
| --- | --- |
| Anthropic | `{"thinking": {"type": "adaptive"}, "output_config": {"effort": e}}` |
| OpenAI, ChatGPT, OpenRouter, xAI | `{"reasoning": {"effort": e}}` |
| Copilot (Responses, a `codex` model) | `{"reasoning": {"effort": e}}` |
| Copilot (chat completions) | `{"reasoning_effort": e}` |
| Gemini | `{"generationConfig": {"thinkingConfig": {"thinkingLevel": e}}}` for `minimal\|low\|medium\|high` |
| Groq, DeepSeek, Mistral | `{"reasoning_effort": e}` |

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
  house style, the wake block) — rides as a content block ahead of the first
  prompt on this connection. It is not written to the tape: Toad explaining
  itself to an agent is machinery, not conversation. A restarted backend
  hears it again; a second prompt on the same connection does not.

Toad's own tools cannot be a function call into another process. The
same handler Toad Agent calls directly is served over streamable HTTP on a
loopback port (`mcp/server.rs`), behind a bearer token only that child is
given, at a path of `/mcp`. The port is the operating system's choice and
the token is fresh per session. The server is named `toad` in `session/new`,
with the token as an `Authorization` header. The ledger is published before
that `session/new` is sent, because a child that lists Toad's tools during
the handshake promotes rows that have to exist by then — a ledger written
afterwards would overwrite what was watched with "declared". Dropping the
driver stops the endpoint and kills the child — its whole process group on
Unix, so a wrapper like `npx` cannot leave the real agent behind.

Granted third-party servers are named in the same `session/new` (stdio
command, or HTTP URL). Toad does not connect them for a child; the child
connects them itself. OAuth and static-header HTTP are refused with a
sentence saying why, not connected with a dead credential.

Toad draws permission cards, but it does not decide whether the agent sends
the requests. For Cursor, if `~/.cursor/cli-config.json` has
`approvalMode` `unrestricted`, a warning notice is written on the tape at
start so a person who thinks they are behind a gate that is not there is
told. Other backends: Toad does not guess.

## The computer

A teammate can have a computer: a containerized Linux desktop it drives
through MCP tools. The container is the machine; the agent is the operator.
`persona.computer.enabled` is the switch. The image is
`persona.computer.image`, else the room setting `computerImage`, else the
pin `ghcr.io/1broseidon/toad-computer:<COMPUTER_VERSION>` in
`crates/toad-core/src/computer/mod.rs`. The computer is **not** part of
`mcpPolicy`: a teammate that asked for a machine gets it even on a policy
of none.

Wake is on start. `start_now`, when the computer is enabled, calls
`ensure_running` before the grant and appends
`McpServer { id: "computer", name: "Computer", transport: Http { url, auth: Bearer } }`
to the granted list. A failure to ensure is a start failure with the
runtime's sentence — `"No container runtime was found; install Docker or
Podman."` and the like — not a silent absence. Pulling an image that is
not present writes one notice on the tape: `"Pulling the computer image …"`.

Both kinds of agent get the same grant. Toad Agent connects the HTTP
endpoint in-process with the bearer token. An ACP child is named the
server in `session/new` with `Authorization: Bearer <token>`. The ledger
row for its tools follows the normal MCP path, origin `computer`.

The token is generated once per container and kept in process state, never
settings. A container left behind by a previous run of Toad has a token
this process no longer knows, so it is removed and recreated.

In the window, Settings › Computer writes `computerRuntime` (blank is
automatic) and `computerImage`; the teammate's pane writes
`persona.computer` and shows `computer.status` — Open desktop opens
`viewer` outside the window, Stop and Remove call `computer.stop` and
`computer.remove`. A `computer_frame` on the tape is drawn as a thumbnail
card.

Toad Agent writes that frame immediately after a `computer__*` tool's
completed event, with `dataUrl` a `data:<mime>;base64,…` of the image the
tool returned — the same picture the model was given as Rig image content —
and the tool card itself keeps a one-line placeholder so it still has text.
An ACP child can carry the image as a `ContentBlock::Image` on
`session/update`; Toad writes a frame when that call's title or kind uses
the `computer__` prefix, and does not guess if the child titled the call
something else.

Idle uses the room's existing sweep, not a second clock. When a session
stops, last activity is stamped; thirty minutes later (`COMPUTER_IDLE_STOP`)
the container is `stop`ped (the rw layer survives). Seven days
(`COMPUTER_HIBERNATE`) and the sweep `rm`s it.

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
the tape compacted. For an ACP child the cards are settled the moment the
agent answers the turn, before the transcript has caught up: a permission
answered in between would be a decision written down for an agent that had
already stopped listening. `session.cancel` settles first for the same
reason. When the turn's updates have all been recorded, any card still
claiming to be live is expired on the tape, and a session that stops is the
same fact. A person who does not answer within ten minutes is the same
fact: the agent is told the request was cancelled.

## Asking the person

`request_human` is one of Toad's own tools, on both agent kinds. The agent
says what it cannot do — credentials, a tap, a CAPTCHA, a question only
the person can answer — and the call waits. A `human_action` event lands
on the tape, id `human:<actionId>`, status `pending`. The room holds a
oneshot by that id. `human.answer` resolves it with `done` or `declined`
and an optional `note`, and supersedes the card with both; declined is
written as `dismissed`, the previous Toad's word for that afterlife. The
tool returns a sentence: "The person did it.", "The person declined.", or
"Nobody answered in ten minutes.", and when the person typed a note it
follows word for word: "They said: …". A card left pending when the turn
is cancelled, the session stops or the room restarts is expired by the same
fold that expires orphaned permission cards: the tool call is inside the
turn, so a turn that ended is an agent that has stopped listening.

## Peer threads

A teammate asking a colleague is not a line on either tape. Three records
come out of `message_teammate` (`session/peers.rs`):

- **The thread.** `threads/<key>.jsonl`, one file per pair, belonging to
  neither side. The words of the exchange go here and never onto either
  teammate's tape: what a colleague asked is not part of the conversation
  the user is having.
- **The peer session.** The target's agent, started again for this caller,
  with a preamble saying who is speaking and why. It is a session of its
  own so a teammate answering a colleague does not do it inside the user's
  context — and one per *direction*, because A asking B and B asking A are
  two conversations. Checkpoints are withdrawn: reopening the user's
  session would answer the colleague inside it.
- **The marker.** A `peer` event on each side's own tape, superseded by id
  as the exchange goes, so a person reading either tape can see that these
  two are talking and how far they have got. It lives as long as the peer
  session, which is also how far apart two exchanges may be and still be
  drawn as one line: ten minutes idle, then the session is stopped.

`list_teammates` is roster metadata only — id, name, goal, and what that
teammate's own session is doing — never anyone's conversation, and never
the caller. The caller may be mid-turn on its own tape while the delivery
runs: nothing here touches the caller's session, only its tape's marker.

A pair is refused a second delivery while one is running (`"That thread is
already answering."`). A teammate cannot message itself. A message is at
most 24,000 characters and cannot be empty. The caller's words arrive
fenced as message data, not as a second system prompt.

Receipts are decided from the *kind* of event and nothing else: a message
is `sent` when it enters the thread and `read` when the recipient's
session proves it took it into a turn. Nothing un-reads a message. The
agent is never told a tick exists.

Nothing can answer a permission card raised inside a peer turn, because no
seat is shown one. The card is still written to the thread and the marker
goes to `waiting`, so a reader can see what the thread is stopped on. On
startup those cards expire with the tapes.

Deleting a teammate stops every peer session it is a side of. The wire for
listing threads and marking them read is [wire.md](wire.md).

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

`chapter.resume` / `resume_chapter` reopens the chapter immediately before
the open one. A context from further back is not offered. The current
chapter closes as `"Back to: <previous title>"` with `closedBy` `resume`,
without asking a model for a note — it is a turning point, not a stretch
of work — and a new marker opens carrying `resumedFrom` and the previous
chapter's note. The session is stopped and started again: Toad Agent is
seeded from that chapter's tape slice; an ACP child from the checkpoint
the marker still names. User lines said in the meantime arrive as a nudge
— Toad's words, never a line of the tape. If the restore fails, the new
session reads the note (the wake block already carries it) and a notice
says the context could not be reopened. A second resume is refused when
the chapter immediately before closed by resume, when nothing precedes, or
when that previous chapter ran on a different agent.

### Reattaching

A change to what a teammate can use restarts a live session behind the
same start gate. The swap is the stop and start a closed chapter already
does, for a different reason: the driver is built from the persona and
the room's servers, so leaving it running would keep the old set until
somebody stopped the teammate by hand. Between turns it happens now;
during a turn it waits until the turn ends, and a queued line runs
before the swap, because a message the person already sent is worth more
than a tool change landing one turn sooner. An idle teammate is left
alone: the next start builds from the new state.

The chapter stays open. The ledger the new start publishes is the record
of what attached; nothing is written on the tape for the restart. The
stop emits `Stopped` and the start emits `Ready`, and the window's band
follows those. A restart that fails to start leaves the teammate stopped
with the start's error.

Toad Agent is rebuilt in-process with the new grant and the new reach;
its context is the tape. An ACP child is a new process, handed the new
servers in `session/new`; `context_restored` says whether the harness
gave the context back. `start_now` writes `AGENTS.md` before that child
starts, so a `goal` change reaches it through the file on the same
restart.

### Idle sweep

One task for the whole room, a few seconds after the desk opens and then
every minute. A chapter that has gone quiet for longer than
`chapterIdleHours` (default 8, clamped to 1–336) is closed as `idle`, dated
from the last message, not from when the sweep noticed. A chapter that went
stale while Toad was closed is closed before anyone comes back to read it.
A turn still running is still adding to the chapter: the sweep looks again
in ten minutes rather than cutting it off. The same clock stops peer
sessions that have sat unused for ten minutes.

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

A teammate speaks those strings through four tools, the same handler on
both kinds of agent — Toad Agent in-process, an ACP child over Toad's own
MCP server:

| tool | strings | what it does |
| --- | --- | --- |
| `schedule` | `when`, `prompt`, `quiet?` | wake once; `when` is `20m` or an ISO timestamp |
| `loop` | `every`, `prompt`, `quiet?` | wake on an interval; `every` is `15s`, `5m`, `1h`, `1d` |
| `list_schedules` | `target?` | the caller's jobs, or another teammate's if `target` is their personaId |
| `cancel_schedule` | `id` | drop one of the caller's jobs; another teammate's is refused |

The pane labels a job from its prompt; the tools take no name. A string
neither parser accepts is refused as that string, not as milliseconds.
The tools do not re-check the bounds — they parse, call the room, and
return the room's refusal.

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
listed them, and absent — with the error as the reason — when it did not.
The row's `name` is the slugged tool name the agent sees; its `origin` is
still the server id, because that is the row key and the window resolves
it to the name the Tools pane shows. A server whose env has a value that
is not a string is absent, naming the offending key; starting it without
that variable is worse than not starting it. A policy id that no longer names a server is absent with one sentence,
the same on either driver. A server that was attached and later dies —
process exited, connection closed, HTTP endpoint unreachable — turns every
row from that origin absent, with the transport error as the reason, and
writes one notice on the tape: `The <name> MCP server went away:
<reason>.` The person changing the server list restarts the teammate;
the notice is for a server that died on its own. A tool-level
error the server itself answered leaves the rows verified.

A child is handed descriptors and does not report what it loaded, so its
honest state is declared: Toad's own tools as named tools, each granted
server as one row under that server's name. Those rows are published
before `session/new` names the endpoint. The one exception is Toad's own
endpoint, which promotes its rows to verified the moment the child lists
tools on it.

The streams these sessions write are [log.md](log.md). How to run the room
is [development.md](development.md).
