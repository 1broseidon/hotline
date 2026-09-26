# Sessions

A session is one live conversation with one agent, on behalf of one teammate.
Its **driver** is either Hotline Agent in-process on Rig, or an ACP child
process. The session's rules exist once; a driver knows nothing of tapes.

Those rules live in `crates/hotline-core/src/session/`. A driver in
`crates/hotline-core/src/driver/` runs a turn and hands back updates; the room
turns each update into the tape events it is, in the shapes the previous edition
wrote, and offers the line to the search index. What was said is on the tape
**before** the driver sees it, so a turn that fails cannot lose the message
that started it. Hotline Agent's live history keeps the user's line on failure
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
command returns as soon as the turn is started. Hotline Agent admits new operator
input into its running activity, and so does an ACP agent that offers steering
(see [Steering an ACP turn](#steering-an-acp-turn)). A driver without
active-input support queues it. Scheduled runs and internal nudges remain queued. Joining
the queue and claiming an idle driver are one decision under one lock, so a
line cannot be filed behind a turn that has already stopped coming back for
it. `session.cancel` stops the turn in flight and drops whatever was waiting.
Deltas go out on the tape
subscription as `ephemeral` and are never written; the durable line is the
message that lands when it is whole.

### Pacing

A reply is chat: one string split into bubbles by a function of the text
(`session/pacing.rs::paced`), never by the model's mood. Blank-line units
outside fences become bubbles; a list stays one unit; a stub shorter than
60 characters joins its neighbour, and a unit that ends with `:` joins the
next, so "Here's the fix:" and a fence stay one bubble. The length of a
reply is the prompt's job, not a fold's.

Each bubble is one `agent` event, ids `{id}` then `{id}-2`…, same timestamp.
The model said one thing: history rejoins consecutive agent events with a
blank line. Both agent kinds and peer threads pass the same funnel, so they
get the same pacing. The house style in the preamble tells the agent this
rule, in words it can act on.

A scheduled firing is the same funnel with two differences the tape can see:
the agent hears a framed prompt (`loop · …` or `scheduled · …`) naming the
job, and the transcript keeps the bare prompt plus the stamp, so the
conversation draws one line instead of a wall. The framing never mentions
quiet — see [Quiet](#quiet).

## Two drivers

`backendId` on the teammate's record is the whole choice. `"hotline"` is Hotline
Agent, in this process, on the desk's provider keys. Any other id is an ACP
child; the registry in `driver/acp/registry.rs` is what says whether this
machine can start that harness, and with which command.

`backends.list` answers the picker: Hotline Agent first, then whatever the
catalogue and the PATH say. A row the machine cannot start carries
`unavailable` as a sentence naming what is missing (a CLI not on PATH, an
archive Hotline does not download). A missing login is not missing: that shows
up when the session starts.

The registry is data, not code. It is a table of agents Hotline has been taught
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

## Hotline Agent

Hotline Agent (`driver/rig.rs`) runs the model in this process. Nothing asks
permission: the teammate's one policy is how far its tools reach, and the
session says which with every prompt.

`driver/rig/turn.rs` owns one loop over Rig's ordinary streaming completion
requests and tool execution. Rig keeps provider construction, authentication,
request encoding, and response parsing. Every Hotline Agent provider uses that
loop; `capabilities.activeInput` is true without a provider steering endpoint.

An operator update interrupts inference, including a request still waiting
for its first response. The next request includes the update and completed
history. Incomplete tool calls never execute, and completed calls keep their
result pairing and provider metadata. The line is marked read when the agent
next says or does something. Steering produces no extra logical turn boundary;
Stop remains a separate control. The composer offers Send alongside Stop
while working.

Shell commands run as managed jobs owned by `session/jobs.rs` for the active
conversation activity. `shell` promptly returns a job receipt; `inspect_job`
reads its state, `wait_jobs` waits up to 30 seconds, and `cancel_job` requests
termination. A new operator message interrupts a wait without cancelling its
jobs. The model decides what to keep or cancel, using the current job snapshot
included with each request. These controls use ordinary Rig tool calls on all
providers. Up to four commands can run concurrently; controls remain available
when those launch slots are occupied.

A launch receives exactly one model tool reply. Later results arrive as
labelled execution data and update the original shell card on the tape. The
shell card stays in progress until its actual result, including partial output,
is available. A textual model response cannot finish the logical activity while
jobs remain: the loop waits for an operator message or a job result without
issuing idle model requests. Job completion resumes the loop once with the new
result. Job handles live for that activity; completed facts remain in its
conversation history. Jobs are not restarted after a process or session restart.

Cancellation signals the command's owned Unix process group or Windows job,
then waits for process exit and output collection. `cancelling` is only a
request. `cancelled` requires exit evidence; if termination cannot be confirmed
within five seconds, the result says `interrupted` with that uncertainty. Stop,
revocation, and a failed inference also cancel the activity's jobs and settle
their results before the driver closes. A dropped driver retains process cleanup
guards. Existing effects are never rolled back.

While the loop waits on its jobs, what the model said last is released to
the tape as a reply rather than held as possible narration: the driver sends
`Update::Parked` just before it waits, and the person reads that line while
the work goes on.

### Subagents

A Hotline Agent teammate's own session can hand a task to a **subagent**
with the `subagent` tool: `task` is everything the worker will know, and
`title` is the short label the person sees. A subagent is a managed job
like a shell command — the call returns a receipt at once, `inspect_job`,
`wait_jobs` and `cancel_job` work on it, up to four run at a time, and its
report arrives later as the job's result — so the teammate keeps talking
with the person while it works, and a message from the person reaches the
teammate without waiting for any subagent.

There is one kind of subagent. It is the teammate's agent started fresh by
`Room::run` (`session/runner.rs`): the teammate's working directory, reach,
granted MCP servers, and the model and effort its session is on at that
moment, with a system prompt of its own that makes it a worker reporting
back rather than the teammate talking to the person. It gets none of the
conversation, no checkpoint to reopen, no computer, and of Hotline's own
tools only `search_thread` and `list_chapters`. It cannot start subagents.
Its report is what it said after its last tool call; a run that ends on an
error or a cancel says so and keeps what it had said.

A run writes to its own stream, `runs/<runId>.jsonl`, the way a tape is
written. On the teammate's tape it leaves one `subagent` line, rewritten as
the run goes from `running` to `done`, `failed` or `cancelled`; pressing it
opens the run in the inspector's place. The run id is the job id and the
tool call's id. Stop, revocation or any other end of the teammate's turn
cancels its runs and waits for them to settle; a run the process died under
is settled as `cancelled` on the next start. The authority rules are
[security.md](security.md#grant-lifecycle).

`drive` in the same module is the loop both a run and a peer exchange use to
take one prompt to its end on a stream nobody is watching: the funnel's
narration, the same events, and a tool left running when the driver stops
written down as failed.

Other tools remain synchronous. An update arriving during one of those tools
waits for it to settle, then skips further calls from the old request. An
obsolete `request_human` wait is released without treating the new text as an
answer or approval. Missing usage from an interrupted request is reported as
unknown.

Before anything else it is told a **preamble**: who it is, the goal, the
working directory, how far it can reach, how to read the clock, how to use Hotline's
own tools, the index of the skills in its workspace (name, description and
path; the body is read when the task calls for it — a teammate with a
computer finds that computer's own guide there as `hotline-computer`), and the house style
([Pacing](#pacing)). When it
is joining a conversation that already has chapters behind it, the
[wake block](#the-wake-block) follows. It is seeded with what
was said in the chapter it is joining — user and agent lines only; tool
calls are the agent's own working memory of a turn, not the conversation.

The preamble carries no date, so it reads the same every day and stays a
cached prefix. Every line the agent hears opens with the local day and time
it was said instead, `[Thu 24 Sep 2026, 15:12] …`: a person's message, a
schedule firing, Hotline's own nudge, a subagent's brief and a colleague's
question alike, and the seeded conversation carries each line's own time.
The tape keeps the words alone; the time is already the event's `ts`.

On each turn it is given:

| kind | what | how |
| --- | --- | --- |
| workspace tools | `ls`, `read`, `grep`, `glob`, `write`, `edit` | in-process, on cap-std; a path that leaves the working directory is refused unless reach is the whole machine, except a read under the teammate's own `tool-output` directory |
| shell | `shell` | in-process. Machine reach is a command in the working directory with no wall. On Linux, workspace reach exposes the working directory and selected read-only installed tools, with a private home and `/tmp`; other host files are hidden. Network stays on: agents install things. The restrictions depend on the OS, as listed below. |
| Hotline's own tools | `search_thread`, `list_chapters`, `resume_chapter`, `new_chapter`, `request_human`, `react`, `send_file`, `list_teammates`, `message_teammate`, `schedule`, `loop`, `list_schedules`, `cancel_schedule`, `computer_status` | the same functions, as Rig tools — a transport between two halves of one process would only be a way for this to fail |
| granted MCP tools | every server the teammate's `mcpPolicy` selects; none by default | Hotline connects them as the client (`mcp/mod.rs`). Each tool is named `{server name as a slug}__{tool}` and listed in the preamble, one line each; the tool list carries only `tool_schema`, which answers one tool's description and parameters, and `call_tool`, which calls it (`driver/rig/granted.rs`). The computer's tools are registered as tools of their own |

Every tool definition goes out with every request, and a large server's can
outweigh the rest of the request, so Hotline Agent keeps a granted server's
schemas out of it until the agent asks for one. The two tools that stand in
for them are the same whatever is granted, so the tool list is a stable
cached prefix; the preamble's list changes only when the granted servers do,
which restarts the session anyway. A `call_tool` shows in the transcript as
the tool it called, and a call the server refuses comes back with that
tool's parameters, so the next try has them.

Settings → Tools is the MCP gateway: configuring a server makes it available
to grant, not automatically available to every teammate. New teammates start
with **No servers**. **Selected servers** grants only named servers; **All
servers** grants every configured server, including ones added later. Existing
saved choices remain intact. Imported teammates without a valid saved policy
get no gateway servers. Changing reach never changes an MCP grant.

For an HTTP server marked OAuth 2.1, Settings → Tools discovers protected
resource and authorization server metadata, requires advertised authorization
code and PKCE S256 support, and opens a native loopback callback. The rmcp
client performs DCR only when the server advertises a registration endpoint;
saved registrations and configured preregistered client ids take precedence.
Servers that publish only CIMD metadata, omit required discovery or PKCE
metadata, or rely on legacy guessed endpoints are reported as unsupported.
Token endpoint client authentication follows the methods advertised by the
authorization server through rmcp; Hotline does not invent another method or
persist a client secret in settings.
Tokens, refresh tokens and DCR client secrets live in the private vault under
the server URL and issuer binding. They are never settings, stream, tape,
prompt, ACP descriptor or log data. Refreshes and rotated refresh tokens are
serialized across sessions, and sign-out clears the registration and tokens.

An HTTP server can instead send a token the person pastes: **bearer** mode
sends `Authorization: Bearer <token>`, **header** mode sends the token in a
header the person names. The token goes to the protected vault through
`mcp.secret_set`, bound to the server URL, and never into settings; a server
whose token the vault does not hold is absent from the ledger with a sentence
saying so. Forgetting it is the same sign-out as OAuth.

Hotline Agent uses rmcp's auth-aware Streamable HTTP client, so expiry and refresh
remain inside the gateway; a pasted token rides the same client as one header.
ACP receives a per-session loopback URL and a separate proxy bearer token. The
handler verifies that token and the capability lease, then puts the vault's
credential on each request — a current OAuth token, or the pasted one; the
child never receives either. Signing in authorizes the
gateway connection and does not change any teammate's none, selected or all
policy.

A grant authorizes the server's own capabilities, including any access it has
outside the teammate's workspace. Hotline does not put granted servers inside the
shell sandbox. Hotline's own tools and a separately enabled computer remain
available independently of the gateway policy. Both drivers receive the
selected gateway list; an ACP harness may also load its own configured tools.
Grant changes revoke existing handles, cancel current execution, and clear
queued turns before rebuilding a live session. Cached peer sessions are
invalidated whether the changed teammate was their caller or recipient.
Already dispatched external side effects cannot be undone; a remote tool
call may finish after its connection is closed.

The Linux shell uses the host's installed tools without mounting the host's
whole filesystem. The runtime is an explicit exception to workspace reach:
executables, their libraries, and public configuration are readable. Other
projects, host home contents, Hotline's vault, and host control sockets are not
mounted. The synthetic root and parent directories are read-only. Listing those
directories shows the sandbox's mount layout, not the host's directory contents.
The workspace and private `/tmp` remain writable separate mounts, so a workspace
under `/tmp` can still write to its synthetic parent within that private scratch.
Workspace contents themselves remain available, including any secrets the user
puts in that workspace.

| OS | workspace reach |
| --- | --- |
| Linux | System `bwrap` starts from an empty filesystem. System binary, library, header, and shared-runtime directories are mounted read-only, including their `/usr/local` counterparts and Go/Swift installations. Other system trees such as `/usr/local/src` stay hidden. Selected toolchain installations and public configuration are added, then the writable workspace. `/tmp`, `/dev`, and `/proc` are private; PID and IPC namespaces isolate host processes. A missing or unusable sandbox omits the tool and the ledger says why. |
| macOS | System `/usr/bin/sandbox-exec` applies a default-deny Seatbelt policy. The workspace is writable; selected runtimes are read-only. Other host files and workspaces are inaccessible. HOME and TMPDIR are private workspace directories. An enforcement probe must demonstrate allowed workspace access and denied outside reads/writes before the ledger offers the shell. |
| Windows | No confinement Hotline can ship, so the tool is not offered; the ledger reason says to give the teammate machine reach. Machine reach keeps `cmd /C`. |

On Windows, shell commands, ACP agents and stdio MCP servers start suspended,
enter a kill-on-close job, and resume only after assignment succeeds. Closing
the job cleans up descendants as well as the direct child. A job controls
process lifetime; it does not provide workspace confinement.


On Linux and macOS, `.hotline-home/` inside the workspace is the shell's persistent `HOME`,
with private XDG and Cargo directories. It is created inside the sandbox so a
project-controlled symlink cannot make Hotline write outside. Workspaces do not
share these caches; teammates deliberately using the same workspace do.
The shell inherits no host environment, credential variables, or shell startup
configuration. A command needing a credential must receive it deliberately.

Supported runtime layouts include Linuxbrew's `Cellar`, `opt`, binary, library,
and share directories; Cargo binaries and Rustup toolchains/settings; nvm's
Node versions; pyenv's versions, shims and runtime; mise installs/shims; uv's
Python installations; and Bun binaries. Home toolchain roots redirected by
symlink are not automatically mounted. Rustup copies its initial settings to
the private home and links installed toolchains read-only; update hashes and
Cargo caches remain private. The system configuration
mounts are the loader cache, alternatives, public CA certificates, DNS/hosts,
NSS configuration, and timezone file, not all of `/etc`.

On Linux, for other absolute `PATH` entries, standalone executable ELF files and scripts
with a shebang are mounted individually. This exposes executable code, not the
parent directory: a neighboring `.env` stays hidden. A tool with additional
resources in an unsupported location may fail; Hotline never exposes its entire
parent directory to make it work. Install that tool and its dependencies inside
the workspace when its layout is unsupported. Runtime installations are trusted
code locations and should not contain project secrets.

On macOS, runtime exceptions cover system executables, libraries and frameworks,
Command Line Tools, `/Applications/Xcode.app` and versioned `Xcode_<version>.app`
bundles in `/Applications`, Homebrew runtime directories under
`/opt/homebrew` and `/usr/local`, and the home toolchain layouts listed above.
Homebrew `etc` and `var`, host Cargo credentials, and neighboring projects stay
outside the allowlist. Canonical installation roots are required: redirecting
one to another project does not expose that project. Arbitrary PATH entries do
not grant access on macOS; unsupported tools should be installed inside the
workspace. Installations are trusted code locations, not places for secrets.
OpenSSL uses its built-in providers with an empty configuration and the public
system CA bundle; host Homebrew `etc` is not exposed for TLS configuration.
Rustup metadata is copied to the private home on first use; installed toolchains
are linked read-only. Cargo, npm, Go and XDG caches use the private home.

macOS scratch is `.hotline-home/.tmp`, available as `TMPDIR`. Programs that hardcode
`/tmp` instead of respecting `TMPDIR` cannot write there. Setup runs inside
Seatbelt, including home and scratch creation, so hostile symlinks cannot make
the core write outside. Descendants inherit the policy. AppleEvents,
LaunchServices, launchd control, arbitrary Mach services, and Unix control sockets
are denied; only named logging, directory, certificate trust and network configuration services and
the system DNS socket are permitted. IP networking remains enabled. There is no Linux mount or PID
namespace: root directory names and required ancestor metadata can remain
visible, and cancellation uses the existing process group, not a PID namespace.

Seatbelt's command-line interface is deprecated and its SBPL language is
undocumented for third-party use; see [Apple's support guidance](https://developer.apple.com/forums/thread/661939).
The probe fails closed if the system launcher disappears or stops enforcing the
policy. This reduces failure risk but is not an Apple compatibility guarantee.
Release validation must run the isolation and toolchain tests on each supported
macOS version and architecture. BRO-14 is validated locally on macOS 26.4.1
(25E253), Apple Silicon, and by `make check` in macOS 15 CI on both architectures.
The PR records each runner's exact version and outcome. macOS 13/14 remain
untested; the app's macOS 13 minimum is not proof of this policy's compatibility.

Network access still uses the host network, including localhost. This is
filesystem isolation, not network isolation: local services can expose files or
privileged actions of their own. Granted MCP servers, computer access, and ACP
agents retain their own permissions; this shell boundary does not sandbox them.

The shell tests exercise outside reads through direct paths, symlinks, child
processes and `/proc`, clean environment, persistent private home, standalone
PATH tools, installed Python/Node/Go/Rust, and cancellation. The desk harness
switches reach over the wire and checks the real shell and read tool against
another project's `.env` and checks the live session's tool ledger. Linux tests
require working bubblewrap; macOS isolation tests fail rather than skip if
Seatbelt cannot enforce the policy. Mac-specific tests exercise hostile home
and scratch symlinks, shared temporary files, helper services, Unix sockets,
and missing or ineffective launchers. The explicit network smoke tests
(`cargo test --workspace tools::shell::macos::tests -- --ignored`) additionally
check public DNS/HTTPS and npm/Go downloads into private caches; they require
access to public package registries.

On Ubuntu 24.04 and later, `bwrap: setting up uid map: Permission denied`
means the kernel's `apparmor_restrict_unprivileged_userns` is on. Ubuntu's
own answer is a profile that lets `bwrap` alone make a user namespace and
strips capabilities from everything it starts. On Ubuntu 25.04 and later
it ships loaded with the `apparmor` package and installing `bubblewrap` is
the whole setup. On 24.04 it is an extra profile that has to be copied
into place and loaded; the next teammate start then offers the shell
again:

```
sudo apt install bubblewrap apparmor-profiles apparmor-utils
sudo install -m 0644 /usr/share/apparmor/extra-profiles/bwrap-userns-restrict /etc/apparmor.d/bwrap-userns-restrict
sudo apparmor_parser -r /etc/apparmor.d/bwrap-userns-restrict
```

Inside a sandbox the labels then read `bwrap (enforce)` for bwrap and
`bwrap//&unpriv_bwrap (enforce)` for the shell it runs, which is the
proof the profile took.

A granted stdio server is spawned in its own process group on Unix, so a
launcher like `npx` does not leave the real server behind when the session
stops.

What the model is shown of a result stays in the conversation and goes
out again with every later request in the chapter, so each kind of result
has a budget (`Budget` in `driver/rig.rs`). A command's output reads as a
terminal left it, without colour codes or redrawn progress lines. Past
16 KiB the model sees how stdout and stderr each start and end, a quarter
from the start and the rest from the end, where a build or a test run
says how it went. A granted server's result, the computer's included, is
cut past 64 KiB, and Hotline's own tools past 256 KiB, half from each end.
A file read already stops at 2,000 lines. Whatever is cut is kept whole
under `tool-output/<personaId>/` in the data directory, and the model is
told its size and path. Under workspace reach the teammate can read that
directory, and nothing else outside the working directory. A finished
job's output reaches the model once, as the job's result; `wait_jobs` and
`cancel_job` report only states. The transcript bubble keeps 4,000
characters of output either way.

Anything quoted out of a conversation goes in front of a model inside a
fence (`fence.rs`): `search_thread` and `list_chapters` results, the wake
block's last messages and previous handoff note, the user lines a resumed
chapter is nudged with, a colleague's message, and the chapter transcript
the note model reads. The conversation is data, and the one string it must
not spell is the tag that closes its fence. Every `<` in the body is
rewritten as `\u003c` — the same character to anything parsing JSON, and
no character at all to anything scanning for a tag — so nothing quoted can
close the fence early, whichever fence it is in.

The bundled fallback and metadata for models come from the model catalogue
(`crates/hotline-core/models.json`, a filtered snapshot of models.dev — see
[development.md](development.md#the-model-catalogue)) for the providers
`models.rs` wires: Anthropic, OpenAI, OpenRouter, Google, xAI, Groq,
DeepSeek, Mistral, Ollama Cloud, Z.ai Standard and Z.ai Coding Plan as API keys, and GitHub Copilot and ChatGPT
(`openai-codex`) as subscription logins, as `provider/model`. OpenRouter also
offers browser sign-in: PKCE issues an API key, kept privately beside the
login and passed to Rig's existing OpenRouter client. It uses the same
OpenRouter models and billing as a pasted key. A saved
`enabledModels` filter narrows what is offered, never the model a
teammate is on.

xAI also offers **Sign in with SuperGrok or X Premium+**. It uses xAI's device
authorization page and stores access and refresh tokens privately. Hotline
refreshes before expiry, serializes refresh across teammates, and retries a
rejected bearer once. Sign-out or revocation removes these tokens. A failed
subscription request never switches to API-key billing. The picker uses the
xAI catalogue; actual model access and usage limits depend on the signed-in
plan, and a refusal is shown on the turn. This follows xAI's documented
[subscription OAuth integration](https://x.ai/news/grok-kilocode).

Z.ai Standard uses `https://api.z.ai/api/paas/v4`; Z.ai Coding Plan uses
`https://api.z.ai/api/coding/paas/v4`. Each has its own key and model list so
a coding-plan request cannot accidentally use the standard billing endpoint.
Both use Rig's native Z.ai client, including streaming and tools. See the
[Z.ai quick start](https://docs.z.ai/guides/overview/quick-start) and
[Coding Plan guide](https://docs.z.ai/devpack/overview).

Ollama Local connects to an HTTP or HTTPS server URL (default
`http://localhost:11434`) without a key. Connection first reads `/api/tags`
through Rig, so custom installed model ids enter the picker unchanged, even
when absent from models.dev. Ollama Cloud uses the same Rig client at
`https://ollama.com` with its API key. Its discovered models replace the
bundled cloud list; a failed refresh keeps the last successful list. Both
connections offer Refresh in Settings → Providers. Pull local models with
Ollama itself; cloud models exposed by a local server after `ollama signin`
also work through the Local connection. Choose models that support tools
for Hotline Agent's workspace and teammate tools.

Custom servers use **Settings → Providers → OpenAI-compatible**. Give the
connection a name, enter its API base URL (including `/v1` when required),
choose Responses or Chat Completions, and enable API key authentication only
when the server requires it. Discover models or enter one exact model ID per
line, then save. Choose models that support tool calls. Discovery replaces the
form's list; a failed discovery preserves what you typed. It does not change
the saved list until you save. Use Edit connection to discover again.

| Connection recipe | Base URL | API | Authentication |
| --- | --- | --- | --- |
| Together AI | `https://api.together.ai/v1` | Chat Completions | Together API key |
| LM Studio | `http://localhost:1234/v1` | Responses | None by default; token if enabled in LM Studio |

For Together, use an exact model ID from discovery or the Together model
catalogue, including its owner prefix. Together does not implement Responses;
see its [OpenAI compatibility guide](https://docs.together.ai/docs/inference/openai-compatibility).
For LM Studio, load a tool-capable model and start its API server before
connecting. See [supported endpoints](https://lmstudio.ai/docs/developer/openai-compat)
and [optional authentication](https://lmstudio.ai/docs/developer/core/authentication).

Multiple custom connections can offer the same model ID. Hotline groups them by
your connection names and stores each selection as `custom-<connection-id>/<model-id>`.
Editing preserves that identity; deleting the connection removes its key and
models. Leaving an existing key blank preserves it, but changing the URL
requires entering the key again or turning authentication off. Custom models
have no inferred context limits or effort controls; compatibility depends on
the server and model. As with other unverified providers, images returned by
tools go to the tape with a text placeholder sent to the model.

GitHub Copilot's picker uses the models its signed-in account lists,
including IDs absent from the bundled catalogue. Metadata is added when an
exact match exists. The list is fetched at sign-in (`GET {api}/models`,
read directly with Rig's stored token) and stored as `models.json` beside
the login, with each model's `supported_endpoints` in `endpoints.json`
beside it. A model the list offers on `/responses` and not on
`/chat/completions` (Grok and the newer OpenAI models) is driven over
`/responses` through Rig's OpenAI Responses client, with a transport that
re-reads the token Rig keeps fresh and signs each request the way Copilot's
own client does; every other model stays on Rig's Copilot route, which
sends the Codex family to `/responses` on its own. Tools on the
`/responses` route are sent strict, as on Rig's own Copilot Responses
route. A login with no `endpoints.json` yet, such as one made before
0.17.3, fetches the list on its first Copilot turn (at most 10 s, and a
failure is not retried for 5 minutes, the turn staying on Rig's route
meanwhile). A fetch that
fails leaves the login in place and no list, with a notice that Refresh
on the provider's own page under Settings → Providers retries. Refresh re-reads the list, which is
also how a login made before this file existed, or a model newly enabled
on the account, lands in the picker. Hotline does not POST
`/models/{id}/policy {state: "enabled"}` after login the way pi does, so
a model the account lists as policy-disabled is simply not offered.

Provider model discovery is also available through **Refresh** for OpenAI
API, Anthropic API, OpenRouter, Gemini, Groq, DeepSeek, Mistral, xAI and
ChatGPT. It uses the configured provider's Rig client where Rig has a
lister. Rig lists neither xAI nor ChatGPT, so Hotline reads those itself.
xAI's list comes from `/v1/language-models`, with the key or the Grok
subscription's refreshed bearer, and keeps only models that answer in text.
ChatGPT's comes from the endpoint the Codex CLI reads,
`https://chatgpt.com/backend-api/codex/models`, and keeps only the models
Codex shows. That backend leaves out models newer than the `client_version`
it is sent. The version is `CLIENT_VERSION` in `providers/chatgpt.rs`, so
raise it when a new ChatGPT model is missing after a refresh. Supported
connections also refresh
when added in Settings; failure leaves the saved connection available for a
later retry. A discovered ID does not need to wait
for the next models.dev snapshot or Hotline release. Listed models can still
have account or capability restrictions; the provider decides whether a
request is allowed. A failed refresh preserves the last successful list.

Use **Manual model IDs** on a connected provider's page to add an exact ID
when discovery is unavailable or incomplete. Add and Remove save immediately;
these entries survive refreshes and app upgrades. The model uses that
connection's existing credentials and endpoint. Copilot manual IDs must also
appear in the account list; use Refresh first. Custom connections edit their
model IDs in Edit connection. No advanced metadata form is required.

Hotline matches IDs against its bundled models.dev snapshot for effort options,
limits, capabilities, and pricing. Provider-reported names and limits take
precedence where available. Missing metadata stays unspecified, with a notice
when the model has no catalogue match. Unknown Anthropic models receive a
conservative 4,096-token request ceiling because that API requires one; this
is a request default, not a claim about the model's maximum output. Hotline does not infer tool support from the fact
that a model appears in a provider list. Refresh and manual additions do not
switch a teammate's selected model or replace its saved model filter.

A turn keeps the teammate's explicit model, otherwise the room's
`defaultModelId`, otherwise `lastModelId`. A saved choice survives a refresh
or filter that no longer lists it; an unavailable model reports an error
rather than silently routing to another model. Only a teammate without a
preference takes the first available model. An ACP teammate's `modelId` is
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
| Ollama (catalogued effort levels) | `{"think": e}` |
| OpenAI, ChatGPT, OpenRouter, xAI | `{"reasoning": {"effort": e}}` |
| Copilot (Responses: a `codex` model, or one the account offers only on `/responses`) | `{"reasoning": {"effort": e}}` |
| Copilot (chat completions) | `{"reasoning_effort": e}` |
| Gemini | `{"generationConfig": {"thinkingConfig": {"thinkingLevel": e}}}` for `minimal\|low\|medium\|high` |
| Groq, DeepSeek, Mistral, Z.ai Standard and Coding Plan | `{"reasoning_effort": e}` |

## An ACP child

An ACP teammate (`driver/acp.rs`) is another process. Selecting it trusts
that harness's tools, configuration, and permission policy. Hotline holds no
credentials for it — these agents sign themselves in — and does not apply
its shell sandbox to the harness's own tools.

For ACP teammates, Settings → Reach shows the harness's advertised runtime
mode and labels it **Externally managed**. The chat header shows only model
and reasoning effort when the harness advertises them. Other ACP settings
are hidden. Hotline sends the actual mode and configuration ids supplied by
the harness; it does not invent a shared set of permission levels. Without
an advertised runtime selector, Reach reports that it is unavailable.
Harness notifications refresh these controls even between turns, including
removing choices the harness withdraws. These updates change the live
session view without adding messages to the conversation.

ACP file callbacks are different: Hotline performs those reads and writes.
They use the workspace directory handle, reject outside paths and symlink
escapes, and carry the session's revocable authority. They stay confined
even if an older ACP teammate has `reach: "machine"` saved, because there
is no longer a Hotline reach toggle for that harness. An ACP runtime mode
does not widen these callbacks. The preamble distinguishes the two
boundaries.

ACP has no system-prompt parameter, so the two things Hotline must say arrive
elsewhere:

- **Who the teammate is** is written to `AGENTS.md` in the working directory
  before the child is started. Only a file that *opens* with
  `<!-- managed by Hotline -->` is replaced; a hand-written `AGENTS.md` in a
  real repository is left alone, including one that merely mentions the
  marker. Materialization uses the same workspace boundary; an `AGENTS.md`
  symlink cannot redirect a Hotline write outside it.

Skills reach both drivers the same way. At every session start Hotline writes
the built-in skills and the offered skills the teammate's `skillPolicy`
grants — the gateway's, and the person's own from `~/.agents/skills` that are
switched on — into `.agents/skills/<name>/` in the working directory, each
entry carrying a `.managed-by-hotline` file. Only an entry with that file is ever
replaced or removed, so a skill the person or the teammate put there stays,
and shadows a grant of the same name; a revoked grant's entry is removed at
the next start. A `.agents` or `.agents/skills` that is a symlink refuses the
start rather than following it. Changing `skillPolicy` reattaches the
session, like `mcpPolicy`, so the preamble's index matches the folder.

That one channel is enough was checked against real harnesses (the ACP
harness test in `crates/hotline-core/tests/desk.rs`, run with `HOTLINE_HARNESS_ACP`
set to each backend on 2026-09-16): a gateway skill whose body alone holds a
sentinel word is granted, the child is asked for the word with nothing in the
prompt naming the folder, and it answers. Codex scans `.agents/skills` on its
own — its adapter says so, warning about its skills context budget — and
Claude Code and Cursor reached the file through the index in the preamble
block that rides ahead of the first turn (Claude Code's own skills folder is
`.claude/skills`, which Hotline does not write). No harness needed a line in
`AGENTS.md`, so none is written. Gemini CLI could not be tried: its client for
individuals is retired. The same run has each child write a skill of its own,
which `skills.list` reports under the workspace source with the description
the child gave it.
- **What kind of room this is** — the preamble (identity, standing, the
  house style, the wake block) — rides as a content block ahead of the first
  prompt on this connection. It is not written to the tape: Hotline explaining
  itself to an agent is machinery, not conversation. A restarted backend
  hears it again; a second prompt on the same connection does not.

### Steering an ACP turn

A line sent while an ACP teammate works reaches the running turn when the
agent offers the steering extension. The agent says so in `initialize`, under
`_meta.steering.supported`, which is what sets `capabilities.activeInput`.
Hotline then sends the line as a `_session/steering` request carrying
`idleBehavior: "promptRequired"`. Claude Code's and Codex's adapters offer it.
Gemini CLI, Cursor, opencode and Grok Build did not as of 2026-09-24; their
lines wait for the turn to end, as every ACP line did before.

Lines go one at a time, in the order they were said, and only while Hotline's
`session/prompt` is out. A line the agent does not take comes back to the
session, with every line after it, and is said as a prompt of its own once the
turn ends. That covers an agent that had already gone idle, an error, and no
answer within 30 seconds. An agent can start a turn of its own for the line
anyway (`startedNewTurn`). Codex's adapter does this when a line lands just as
its turn ends. Hotline cancels that turn and says the line again, because it
drops updates for a turn it did not open. An agent that advertised the method
and then answers that it has no such method is not asked again on that
connection. Stop drops held lines, as it drops queued ones. The agent's own
message ids keep a reply cut off by a steered line apart from the reply to it.

Hotline's own tools cannot be a function call into another process. The
same handler Hotline Agent calls directly is served over streamable HTTP on a
loopback port (`mcp/server.rs`), behind a bearer token only that child is
given, at a path of `/mcp`. The port is the operating system's choice and
the token is fresh per session. The server is named `hotline` in `session/new`,
with the token as an `Authorization` header. The ledger is published before
that `session/new` is sent, because a child that lists Hotline's tools during
the handshake promotes rows that have to exist by then — a ledger written
afterwards would overwrite what was watched with "declared". Dropping the
driver stops the endpoint and kills the child — its whole process group on
Unix, so a wrapper like `npx` cannot leave the real agent behind.

Granted third-party servers are named in the same `session/new` (stdio
command, or HTTP URL). Hotline does not connect them for a child; the child
connects them itself. An authenticated HTTP server is represented by the
per-session loopback proxy described above, whether its credential is an
OAuth token or a pasted one. OAuth servers that are not signed in and token
servers with no saved token are absent with a sentence saying why, and are
never labelled connected.

Hotline draws permission cards, but it does not decide whether the agent sends
the requests. For Cursor, if `~/.cursor/cli-config.json` has
`approvalMode` `unrestricted`, a warning notice is written on the tape at
start so a person who thinks they are behind a gate that is not there is
told. Other backends: Hotline does not guess.

## The computer

A teammate can have a computer: a containerized Linux desktop it drives
through MCP tools. The container is the machine; the agent is the operator.
`persona.computer.enabled` is the switch. The image is
`persona.computer.image`, else the room setting `computerImage`, else the
pin `ghcr.io/1broseidon/hotline-computer:<COMPUTER_VERSION>` in
`crates/hotline-core/src/computer/mod.rs`. The computer is **not** part of
`mcpPolicy`: a teammate that asked for a machine gets it even on a policy
of none.

Wake is on start. `start_now`, when the computer is enabled, calls
`ensure_running` before the grant and appends
`McpServer { id: "computer", name: "Computer", transport: Http { url, auth: Bearer } }`
to the granted list. A failure to ensure is a start failure with the
runtime's sentence — `"No container runtime was found; install Docker or
Podman."` and the like — not a silent absence. Pulling an image that is
not present writes one notice on the tape: `"Pulling the computer image …"`.

Both kinds of agent get the same grant. Hotline Agent connects the HTTP
endpoint in-process with the bearer token. An ACP child is named the
server in `session/new` with `Authorization: Bearer <token>`. The ledger
row for its tools follows the normal MCP path, origin `computer`.

The token is generated once per container and never stored in room settings.
After Hotline restarts, runtime inspection recovers the token from the existing
container's environment so its jobs and viewer remain available. A container
without a recoverable token is recreated.

The container's limits are the teammate's: `persona.computer.memory` is the
runtime's own spelling of a size (`"8g"`, `"512m"`; absent is `4g`) and
`persona.computer.pids` is the process limit, threads included (absent is
1024, zero is unlimited). A teammate that compiles asks for more of both,
because a parallel build spawns more threads than the default allows and a
linker wants more memory than a browser does. A size that is not digits
and one unit letter is a start failure, not a guess.

Four mounts are Hotline's. The room's cwd is bound at `/home/agent/workspace`,
the person's folder. A named volume `hotline-home-<persona id>` is bound at
`/home/agent`, the teammate's home: the environments it prepared for a
workspace, its jobs and their output, its shell history and its browser
profile outlive the container the hibernate cycle removes, so a woken
computer picks up where the last one stopped. The image keeps nothing of
its own in the home, so an empty volume is what a fresh container would
have had. A named volume `hotline-src-<persona id>` is bound at
`/home/agent/src`, the teammate's own scratch: a checkout or a build it
starts there outlives the container too. `computer.remove` leaves every
volume; only the runtime's own volume commands delete them. A named volume
`hotline-nix-glibc` is bound at `/nix`, one Nix store
shared by every teammate. The image ships single-user Nix with a seeded
store, so an empty volume is populated on first use and a `nix develop`
against a flake is a download the first time and a cache hit after, for
every teammate. The glibc image uses a new volume name so an old Alpine store cannot supply an unwritable layout; the old volume remains untouched. Store paths are immutable, so sharing is safe; the one
hazard is `nix-collect-garbage` from inside a container, which cannot see
the processes of another. Apple `container` gets no named volume; its
rw layer is what it has.

`persona.computer.mounts` binds host folders the teammate needs besides
the workspace, so a checkout it has to test is reachable without a clone
through a remote it cannot sign in to. Each is `{ host, path, readonly }`:
an absolute host path (`~` expands) that must be a folder that exists,
because a runtime creates a missing one as root; an absolute container
path that may not equal, contain, or sit inside `/home/agent/workspace`,
`/home/agent/src` or `/nix` (a path inside `/home/agent` is fine, one that
covers it is not); and `readonly`, which defaults to false in the JSON and to true in the window,
where a teammate tests a checkout rather than edits it in place.

In the window, Settings › Computer writes `computerRuntime` (blank is
automatic) and `computerImage`; the teammate's pane writes
`persona.computer` and shows `computer.status` as the Desktop row under
the Computer switch. That row opens to the image, memory and process
fields (blank is absent on the wire, so the default), the mounts with a
form that adds one read-only unless unticked, and Stop or Remove, which
call `computer.stop` and `computer.remove`. Every edit spreads the rest
of `persona.computer`, so no field drops another. Opening the desktop is
the conversation band's Screen key while the status says running. A
`computer_frame` on the tape is drawn as a thumbnail card.

Hotline Agent writes that frame immediately after a `computer__*` tool's
completed event, with `dataUrl` a `data:<mime>;base64,…` of the image the
tool returned — the same picture the model was given as Rig image content —
and the tool card itself keeps a one-line placeholder so it still has text.
Only a provider that takes a picture inside a tool result is given one
(Anthropic, OpenAI); OpenRouter refuses such a message outright, so on any
other provider the model reads the placeholder line and the frame still
lands on the tape.
An ACP child can carry the image as a `ContentBlock::Image` on
`session/update`; Hotline writes a frame when that call's title or kind uses
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

`request_human` is one of Hotline's own tools, on both agent kinds. The agent
says what it cannot do — credentials, a tap, a CAPTCHA, a question only
the person can answer. A `human_action` event lands on the tape, id
`human:<actionId>`, status `pending`, and a phone is pushed.

From a teammate's own session the call **returns at once** ("Asked. …") and
the card carries `delivers: true`. Nothing is parked on it, so the turn goes
on, and neither a cancelled turn, a stopped session nor a restart expires
it: the orphan fold leaves a card that delivers alone. `human.answer` reads
it off the tape, supersedes it with the outcome and the note, and hands the
teammate a `delivery` with `cause: {kind: "answer", actionId, status,
about}` and the note as its `text`, behind the turn in flight or on a turn
of its own. The agent hears "The person answered your request (…)" or
"The person declined your request (…)", then the note fenced, word for word.
One lock settles a card once, however many answers race for it. A card that
delivers and goes a day (`ASK_TTL`) without an answer is expired by the
room's sweep, and the teammate is told the same way.

A colleague's side session (`TeammateTools::for_peer`) has no conversation
of its own to be answered in, so there the call still waits, as it always
has. The room holds a oneshot by the action id. `human.answer` resolves it with `done` or `declined`
and an optional `note`, and supersedes the card with both; declined is
written as `dismissed`, the previous edition's word for that afterlife. The
tool returns a sentence: "The person did it.", "The person declined.", or
"Nobody answered in ten minutes.", and when the person typed a note it
follows word for word: "They said: …". A waiting card left pending when
the turn is cancelled, the session stops or the room restarts is expired by
the same fold that expires orphaned permission cards: the tool call is
inside the turn, so a turn that ended is an agent that has stopped
listening.

A teammate with a computer is told in its preamble that the person can see
that desktop and take it over, and to get the page that needs them on
screen before asking. The window's card for a pending `human_action` opens
the desktop's viewer in a Hotline window of its own (`computer-<personaId>`,
the page the computer serves at `viewer`); the agent is inside the
waiting tool call the whole time, so the person and the agent never drive
the desktop at once, and what the person types goes to the desktop, never
through the agent or the tape. Done hands the desktop back. The card
shows Open the screen only while `computer.status` says running, which the
conversation asks every five seconds while the teammate has a computer and
a session; the band shows Screen on the same condition. Outside the desk
(a browser tab) the viewer opens as a link instead.

## Sending the person a file

`send_file` is one of Hotline's own tools, on both agent kinds
(`session/files.rs`). The teammate names where the file is and says a
caption, and the file arrives as its message: an `agent` event whose `text`
is the caption, possibly empty, carrying the file in `attachments`. The
person is told the way they are told of any reply — it counts as unread,
and a phone is pushed the caption, or "Sent <name>" without one.

Where the file comes from:

- **`workspace`** — a path the teammate's own read tools can open, through
  the same `Workspace` and the same session lease: its working directory,
  its `tool-output` overflow, or anywhere under whole-machine reach. What
  its reach refuses to `read`, `send_file` refuses to send.
- **`computer`** — a path on its own computer, relative ones starting at
  `/home/agent`, fetched over the computer's bearer download door. The
  computer hands over nothing outside its home.
- **`screen`** — its computer's screen now, one window of it, or one
  region, as the computer's `capture` tool takes it.

A computer too old for either — before 0.7 for a file, whose download door
took its token only in the address or did not exist, and before 0.10 for
the screen, whose `capture` had no `image` mode — is said as the pane's
Update rather than as the error it answered.

The desk keeps its own copy under `files/<teammate>/<message id>/` in the
data directory, so the conversation still holds the file after the
workspace changes or the computer is removed. A picture — PNG, JPEG, GIF or
WebP, known by its bytes — is prepared by the same policy as a picture the
person attaches (`images.rs`): upright, flattened, at most 2000 px and a
JPEG of at most 1 MiB, with its width and height on the attachment. A
picture that policy cannot make is sent as the file it is, and the
teammate is told why. Anything else is kept byte for byte, up to 25 MB; a
larger file is refused in a sentence that names the cap. No kind of file
is refused for what it is, because the desk never opens one on its own: a
picture or a PDF opens in the system's viewer when the person presses it,
and only when its name and its first bytes agree — the system picks the
program by the name, so a PDF is always kept under a `.pdf` name, and a
script that starts like a PDF is never handed over as one. Any file can be
saved where the person picks, or shown in its folder.
`attachments[].origin` says where the file came from, in words.

A quiet scheduled run is refused rather than demoted. Its words become
thinking, and a file in thinking reaches nobody while the teammate believes
it was sent, so the tool says it cannot send from a quiet run. A window
that opens while the file is being fetched is caught when the message is
stamped, and the kept copy is removed.

The model remembers a sent file by name: the conversation a driver is
seeded with, a chapter's handoff note and the search index read a message
as its words followed by `[file: <name>]` for each file it carries. The
roster's preview of a file sent without a caption is its name.

A client reads the file with `file.read`, by the message's id, a part of at
most 512 KiB at a time; the phone seat may too. Kept copies follow the tape:
nothing deletes them while the conversation they belong to is kept.

## Teammate collaboration

`message_teammate` is authority to ask another teammate to use its own
workspace and enabled tools, so the room checks collaboration before opening a
thread or starting a peer session. A Hotline Agent caller with explicit Whole
machine reach has that authority implicitly. A workspace caller needs a
first-contact operator decision for each caller and recipient direction,
regardless of whether the two teammates happen to have the same tools. ACP
trust or mode labels and the separate Computer capability do not count as
Whole machine reach.

The card says `Allow <caller> to ask <recipient> to work?` and explains that
the recipient can use its workspace and enabled tools to fulfill the caller's
requests and return results. `Allow this session` binds a temporary grant to
both live capability leases. It expires when either session or chapter is
replaced, when the room restarts, or when a cached peer session is revoked.
`Always allow <caller>` appends the caller's stable id to the recipient's
`allowedSenders` list. The recipient's settings show that list as `Accept
requests from <caller>` and removing one revokes active and queued delegated
work and cached peer sessions. New and imported records have an empty list.

An authorized exchange does not need a reverse grant for its reply or receipt,
and it does not authorize a reverse or transitive request. Further delegated
work depends on the original request's authority: revoking it stops those
delegated sessions without revoking the recipients' independent main sessions.
Pending approvals expire after ten minutes or when the requesting call is
cancelled. They also settle when a participant stops, a policy changes, or the
room closes. One pending first-contact decision is held per direction.

## Peer threads

A teammate asking a colleague is not a line on either tape. Four records
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
- **The answer.** A `delivery` event on the sender's own tape. The tool
  returns as soon as the message is sent (`{"sent": true, "to": …}`) and the
  sender carries on; the exchange, including a first-contact approval, runs
  on its own task. What came back, or why nothing did (`cause.status` is
  `done` or `failed`), is written to the sender's tape and then handed to its
  driver behind the turn in flight, or on a turn of its own, starting the
  sender if it is not running. A delivery never steers into a turn. The
  agent hears the answer fenced, as the recipient heard the message, and the
  seats draw the event as the reason a turn began, not as a bubble.

`list_teammates` is public roster metadata only — id and name,
never anyone's conversation, session details, working path or tool inventory,
and never the caller. The caller may be mid-turn on its own tape while the
exchange runs: nothing touches the caller's session until the answer is
delivered.

A peer session is the one caller that still waits. It has no conversation of
its own for an answer to come back into, so a colleague's side session that
asks a third teammate gets the reply as its tool result, as before.

A delivery is heard exactly once. Its receipt goes `sent` → `read` like a
message's, and when the room opens it hands the driver any delivery in the
open chapter that was never read. An exchange whose peer turn died with the
desk has its markers closed as `failed`, and if it was going within the
last hour, the sender gets a `failed` delivery saying so.

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

Hotline Agent's memory is the tape. It has no session id to checkpoint.

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
chapter's note. The session is stopped and started again: Hotline Agent is
seeded from that chapter's tape slice; an ACP child from the checkpoint
the marker still names. User lines said in the meantime arrive as a nudge
— Hotline's words, never a line of the tape. If the restore fails, the new
session reads the note (the wake block already carries it) and the desk's
log says the context could not be reopened; the tape does not. A second resume is refused when
the chapter immediately before closed by resume, when nothing precedes, or
when that previous chapter ran on a different agent.

### Reattaching

A change to reach, workspace, tools, background work, goal, or harness
revokes existing execution before the new record is written. Policy updates
from separate clients are serialized through persistence and reattachment.
New starts cannot acquire usable authority during that interval. A driver
still starting must validate its captured authority before publication.
If persistence fails, affected sessions remain stopped and their old
handles stay revoked. Retrying the settings change restores execution;
Hotline does not silently restore a grant the operator tried to remove.

The room cancels the old driver, clears its queued turns, and rebuilds a
live session behind the same start gate. It also drops cached peer sessions
involving the teammate in either direction. Cloned tool handles retain a
revocable lease, so removing a session from the cache alone is never the
permission check. An idle teammate stays idle and uses the new policy on
its next start. A gateway change applies this boundary to the whole room,
including peers whose main session is stopped.

Stop ends current main and peer execution and invalidates their handles.
It also invalidates a replacement that is still starting.
It leaves standing grants intact: a later operator prompt, an authorized
peer request, or an authorized schedule may start fresh work. An old turn
cannot publish a checkpoint or replace the new session's state after it
has been revoked.

The chapter stays open. The ledger the new start publishes is the record
of what attached; nothing is written on the tape for the restart. The
stop emits `Stopped` and the start emits `Ready`, and the window's band
follows those. A restart that fails to start leaves the teammate stopped
with the start's error.

Hotline Agent is rebuilt in-process with the new grant and the new reach;
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
stale while Hotline was closed is closed before anyone comes back to read it.
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
fires through the same funnel as a person typing. A missed tick while Hotline
was closed fires once on reopen rather than catching up a pile of them.

`schedule` is once; `loop` is every `every` milliseconds until cancelled.
A loop is recovered from `every` being present — the event's `kind` is
always `schedule`, because that slot is the stream's. A create wakes the
clock so it does not wait for the nearest existing `nextAt`.

A firing starts the teammate if it is idle, stopped or in error, then
prompts. After a one-shot fires it is tombstoned; a loop is rewritten with
`nextAt` a fresh interval from now. A job whose teammate has been deleted
is tombstoned rather than fired. A fire that fails is retried in a minute.

**Background work** is the standing grant for a teammate to create its own
schedules and loops. It defaults off, including on older teammate records.
The tool handlers enforce it when creating a job, and the scheduler checks
it again before a wake and immediately before a queued turn reaches the
driver. Each queued firing carries its own trusted provenance, because a
one-shot job may already have been tombstoned. Turning the grant off pauses agent-created jobs still on
the room stream; it does not delete them. Enabling it again permits a missed
tick to run once, under the usual scheduler rule.

A job created through the authenticated desk's `schedule.create` command is
an explicit operator instruction. Hotline records `operatorCreated: true` on
that job, so it may run with Background work off. Agent tools cannot choose
that provenance. Older jobs without it require Background work. Stopping a
session ends its current execution; it does not revoke standing background
work or cancel operator-created jobs. Cancel a job to remove its future
wakes.

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
both kinds of agent — Hotline Agent in-process, an ACP child over Hotline's own
MCP server:

| tool | strings | what it does |
| --- | --- | --- |
| `schedule` | `when`, `prompt`, `quiet?` | wake once; `when` is `20m` or an ISO timestamp |
| `loop` | `every`, `prompt`, `quiet?` | wake on an interval; `every` is `15s`, `5m`, `1h`, `1d` |
| `list_schedules` | none | only the caller's jobs; a supplied `target` is refused |
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

A quiet run that finds something has one way to be heard, and has to ask
for it by name. Its turn, and only that turn, is handed a `tell_person` tool
(`session/escalation.rs`, armed through `Driver::escalate_next`). A call
does not post anything: it hands the teammate a note, once per run (a
second call is refused). When the quiet turn is over, the room writes the
note as a `user` line stamped with the job's `scheduled` (not quiet, so no
window opens) and queues it ahead of anything else waiting. The teammate
hears which job found what and is asked to tell the person, so the answer
is an ordinary reply in its own voice: it lands in the chat, lights the
rail, and pushes like any other. A run that finds nothing is exactly as
silent as before, and the desktop toast skips a turn whose last visible
line is the person's own. Hotline Agent teammates get the tool; an ACP agent
brings its own tools and is not offered one, so its quiet runs stay silent.
A subagent never gets it: it reports to its teammate, which decides.

## The ledger

Every tool a teammate was given, where it came from, and for anything
absent, why. A row always carries a reason, in every state, because an
optional explanation is the one nobody fills in. `teammate.tools` is that
ledger; `null` when the teammate has never started under a Hotline that keeps
one.

It is built at session start from the same arrays the session hands the
agent, and it lives in this process — it outlives the session so the
question can be asked after the teammate has been stopped, and it dies with
the process. Deleting a teammate forgets it.

| state | meaning |
| --- | --- |
| `verified` | Hotline watched the agent take it |
| `declared` | Hotline handed it over and cannot see what happened next |
| `absent` | it is not there, and `reason` says why |

Hotline Agent's built-ins and Hotline's own tools are verified: they were
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
honest state is declared: Hotline's own tools as named tools, each granted
server as one row under that server's name. Those rows are published
before `session/new` names the endpoint. The one exception is Hotline's own
endpoint, which promotes its rows to verified the moment the child lists
tools on it.

The streams these sessions write are [log.md](log.md). How to run the room
is [development.md](development.md).
