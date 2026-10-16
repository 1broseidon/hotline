# ADR 0005: The live voice is a full agent on the call

- Status: Accepted (0.39, landing over epic-7)
- Date: 2026-10-16
- Supersedes: the relay-only guardrails of
  [ADR 0004](0004-live-calls-get-a-voice-front.md) ("The front relays and
  clarifies", "The client picks the spoken text") for direct calls
- Design: [voice-calls.md](../voice-calls.md), [voice.md](../voice.md),
  [security.md](../security.md)
- Tracking: epic-7 (task-74 tools, task-79 GPT-Live's tool loop, task-75
  summaries, task-77 prompt and settings, task-78 the proving call)

## Context

ADR 0004 put a realtime voice in front of the teammate and kept it a relay:
everything that was not talk about the call went to the working session as a
turn, and the front said back what the session reported. That kept one author
for what is done, and it cost the call what the front was there for.

George tested Grok Voice and GPT-Live from the car on 2026-10-16. Every
request paid a full round trip through the working session, even "what did we
do yesterday", which is one search. Every relayed line became a turn in the
DM, so the chat filled with the call: "No, stop" came back as two "Stopped"
messages. And the front, a model that can run tools and think in the voice
modality, sat idle while a text model did its thinking for it. 0.38.1 took the
first step (talk about the call stays in the voice); the rest is this.

## Decision

On a direct call the front **is the teammate on the call**: a full agent that
answers the person itself, runs the teammate's desk-side tools itself, and
hands the working session only the work that needs it. The chat is the
call's record by summary, not by mirror.

### The front answers

The front holds the conversation. It answers what it can from what it knows
and what its tools return, asks when a request is unclear, and says what it
did. It no longer sends a question to the working session to be answered,
and what it says is its own: the core no longer picks the spoken text of
every exchange, only of a report from handed-off work, which the front still
says as given.

### It runs the teammate's desk-side tools

The front gets the teammate's grants that run on the desk, as function
tools (each provider's route is under Tools by provider): the conversation's search, chapters, files and links, its
schedules, web search, the MCP servers its Reach grants, and its computer
within its reach and mode. Each runs desk-side through the same handler as a
typed turn, with the same permission asks and audit; a tool result goes back
to the model as the function's output, clipped before the model hears it, and
nothing is posted to the DM for it. The registry lives in
`voice/front_tools.rs` (task-74).

### It hands off heavy work, by its own choice

Long or code-changing work still goes to the working session, in its open
chapter, with its grants and harness, through `send_to_teammate` as today:
one short acknowledgement, then quiet until the report. What changes is who
decides: the front sends what it judges heavy, not every utterance. Hanging
up, holding or interrupting still does not cancel handed-off work.

### The chat gets summaries

The call keeps its own transcript (`calls/<callId>.jsonl`, the call card's
transcript key). Nothing mirrors into the DM per turn: the DM holds the turns
of handed-off work, and one summary of the call, at hang-up and every few
minutes on a long call, saying what was asked, what was done and what is left
open. "Put that in the chat" posts at once, through a `post_to_chat` tool
(task-75). The follow-up for a question the call ended on stays.

### Settings follow the teammate

The front's model and voice are the call's settings; its reach and grants are
the teammate's card, so revoking a grant on the card revokes it on the call
(task-77).

### Why

- **Latency.** A lookup answered by the front is answered in voice time, not
  after a text model's turn.
- **One teammate, not two.** The voice and the chat read as one colleague: the
  chat carries what the call did, not every line of it.
- **The model's own strength.** A realtime model that runs tools and thinks in
  speech is the agent on the call; using it as a microphone wastes it.

### What stays

- A front is enforced in the core outside the model: its seat, budget
  reservation and lease are the call's, and it is revocable by hanging up.
  Each tool is a capability change under [security.md](../security.md): proved
  allowed and denied through the real handler, revocable through the lease.
- A pending permission ask is still the person's to answer on the desk or
  phone; the front says so and does not answer it.
- The pipeline (transcribe, teammate turn, synthesize) stays the fallback when
  no front opens or one fails mid-call.
- A desk call is unchanged: its front speaks for the desk and relays to the
  dispatcher, which routes. The desk has no teammate's reach to lend it.

## Tools by provider

None of the three fronts stays delegate-only; each reaches the same registry,
by the route its protocol gives.

- **Grok Voice and GPT Realtime** take function tools in `session.update`.
  The registry is declared on the realtime session, and each
  `function_call` the model makes is run desk-side and answered with its
  `function_call_output` (task-74).
- **GPT-Live** takes no tools on the realtime session: it delegates
  reasoning and tool use to a backend, and that backend's function calls
  come back to the client as response events, answered with
  `function_call_output`. The desk serves it one of two ways (task-79):
  - **Responses delegation**: delegation type `responses` with a backend
    model, the registry declared as its functions, each call run desk-side.
  - **Client delegation**, the ChatGPT sign-in route the adapter uses today:
    a `delegation` is answered by a desk-side tool loop with the registry,
    not by handing it to the working session, which gets only what the loop
    judges heavy, through `send_to_teammate` as on the other fronts.

  Whether the ChatGPT sign-in route accepts `responses` delegation is
  unverified: it could not be spiked without a live login. task-79 built the
  `responses` route (a teammate call declares the registry on a backend model;
  a desk call keeps client delegation); a call the sign-in refuses falls back to
  the pipeline. The client-delegation tool loop needs a desk-run model and is not built.

## Decided

- **Whose words the summary is.** The front's, from its transcript (task-80):
  cheap and live, and it needs no turn of the working session. It is posted as
  an agent message from the teammate, so the chat stays in one voice, and the
  session carries it into its next turn, so a typed follow-up does not start
  cold.

## Open questions

- **A harness teammate's own tools.** Claude Code's edits, Codex's shell and
  the tools any ACP harness carries live inside the harness, not on the desk;
  the front cannot run them and gets only the desk-side grants. For a harness
  teammate, work that needs those tools is a hand-off. Whether the harness can
  lend them to a call (an ACP tool surface the front calls through) is open.
- **Budget for long, tool-heavy calls.** A per-minute key is charged for
  minutes, not tool calls, but tool output read back fills the model's context
  and a session renewal carries it. Clipping bounds each result; a call-level
  bound is not set.
- **GPT-Live's backend.** Which delegation route the ChatGPT sign-in takes
  (above), and, on client delegation, which model runs the desk-side tool
  loop and whose budget it is charged to.

## Consequences

- [voice-calls.md](../voice-calls.md) Direct turns and
  [voice.md](../voice.md) "One brain, two outputs" describe the relay that runs
  today and point here; each epic-7 task rewrites them as its part lands, and
  task-78 removes the pointer once the whole front ships.
- ADR 0004's latency, barge-in, subscription and fallback decisions stand.
