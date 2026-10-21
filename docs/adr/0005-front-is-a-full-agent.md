# ADR 0005: The live voice is a full agent on the call

- Status: Amended by [ADR 0008 §5–6](0008-one-agent-thread-layer.md) (S6, task-225)
- Date: 2026-10-16
- Supersedes: the relay-only guardrails of
  [ADR 0004](0004-live-calls-get-a-voice-front.md) ("The front relays and
  clarifies", "The client picks the spoken text") for direct calls
- Design: [voice-calls.md](../voice-calls.md), [voice.md](../voice.md),
  [security.md](../security.md)
- Tracking: epic-7 (task-74 tools, task-79 GPT-Live's tool loop, task-75
  summaries, task-77 prompt and settings, task-78 the proving call)

## Amendment

The 2026-10-20 decisions are recorded in ADR 0008 §5, including explicit-only
Legacy after a live failure, call-thread work and the closing note. The
original decision below is retained as history, subject to that amendment.

The voice keeps conversation and read-only lookups. Its one work tool,
`work_on_it`, starts or steers its own clone on the call thread, using the
teammate’s model, harness and standing grants. GPT-Live’s client delegation
uses that same tool. Direct writes, raw MCP tools and the call-assistant
delegation loop are removed from the front. The home session is untouched.

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
modality, sat idle while a text model did its thinking for it. The first step
(talk about the call stays in the voice) was built as a 0.38.1 that was never
tagged, and shipped in 0.39.0 with the rest, which is this.

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
- **GPT-Live** takes no tools on the realtime session: it delegates every
  request as a client `delegation`, which the adapter reports as
  `send_to_teammate` (task-84). On a teammate call the desk answers it with a
  tool loop: the call assistant (the Voice settings' dispatcher model), told
  what the front is told, runs the registry and its own `send_to_teammate`,
  and its answer goes back as commentary on the delegation for GPT-Live to
  say. When it calls `send_to_teammate`, fails, says nothing or takes over a
  minute, the request goes to the working session as before; with no call
  assistant every delegation does. A desk call holds no registry and always
  hands on. The `responses` route (the registry declared on a backend model)
  is not used: on the ChatGPT sign-in the service refuses its validation
  connection (`responses_validation_failed`, HTTP 401), which left every
  tool request unanswered.

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
- **Summary voice.** Settled in task-80: the front writes it, in the
  teammate's first person, composed mechanically (no model call) from what
  was asked and answered. A model-written summary is not built.
- **GPT-Live's backend.** Which delegation route the ChatGPT sign-in takes
  (above), and, on client delegation, which model runs the desk-side tool
  loop and whose budget it is charged to.

## Consequences

- [voice-calls.md](../voice-calls.md) Direct turns and
  [voice.md](../voice.md) "One brain, two outputs" describe the full agent,
  and `tests/voice/front_tools.rs` (`one_call_proves_the_whole_front`) proves it end to
  end on a function-tool dialect; the GPT-Live case proves lookups, denial
  and hand-off through its backend tool calls.
- ADR 0004's latency, barge-in, subscription and fallback decisions stand.
