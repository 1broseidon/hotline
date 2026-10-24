# Security — the standing method

This is the checklist an agent follows when it changes what a teammate can
reach, and the record of what the shipped code enforces. It was written
2026-09-14, after the five steps of the "Whole machine" plan landed
(revocation, collaboration grants, private room data and background work,
ACP trust boundaries, macOS sandbox parity). It describes measured
behaviour; where something is planned rather than shipped, it says so. The
mechanics of each boundary are in [sessions.md](sessions.md); the reasons
are in [design.md](design.md). This document is the method, and the index
of proofs.

## The threat model

A teammate does ordinary assistant work on content it did not write: a
cloned repository, a web page, an email, a tool result, a colleague's
message. Any of that content can carry instructions. Hotline's position is
that the model will sometimes follow them, so nothing the model says or
decides is an enforcement point. The person's standing choices are enforced
in the core, outside the model, at every entry a request can arrive
through — Hotline Agent's own tool closures, the in-process MCP server an ACP
child calls, the file callbacks Hotline performs for that child, the scheduler,
and the wire.

Two things are deliberately not defended against. The person: whoever holds
the desk token may do anything the desk may do, and the phone seat is a
smaller fixed set, not a policy engine. And the workspace: a teammate can
read and change everything in its working directory, including any secret
the person left there.

## Standing consent

The person decides once, in settings, and the agent is never asked to ask
again. Each capability below is a field on the teammate's record in the
`room` stream, folded on load; each has a default that grants nothing
beyond the workspace, and an explicit migration for records that predate
it.

| Capability | Field | Default | What it authorizes |
| --- | --- | --- | --- |
| Reach | `reach` | `workspace` | Hotline Agent's workspace tools and shell. `workspace`: the working directory is a wall; the shell runs confined (bubblewrap on Linux, Seatbelt on macOS, not offered on Windows) with selected host toolchains read-only, a private `HOME` and scratch, no host environment, and the host network. `machine`: the same tools with no wall. |
| MCP gateway | `mcpPolicy` | `none` | Each selected server's own capabilities, wherever it reaches. Hotline connects it as the client; it is not inside the shell sandbox. `all` includes servers added later. An imported or invalid policy grants nothing. |
| Collaboration | `allowedSenders` on the recipient, and the caller's reach | empty | Asking another teammate to use its workspace and tools. A `machine` Hotline Agent caller has this implicitly. A `workspace` caller needs the operator's first-contact decision per direction: a session grant bound to both live leases, or a standing grant recorded by the sender's stable id. Discovery gives any caller a colleague's public profile, never its paths, policies or servers' addresses ([What a colleague sees](#what-a-colleague-sees)). A teammate on a linked desk is never implicit, whatever its reach there: it is granted by its qualified id or decided on the recipient's desk ([Linked desks](#linked-desks-bro-177-adr-0003)). |
| Background work | `backgroundWork` | `false`, including on older records | Creating its own schedules and loops. Jobs the person creates over the desk wire carry `operatorCreated` and run without it; an agent tool cannot set that flag. |
| Computer | `computer.enabled` | off | A containerized desktop, `--cap-drop=ALL`, `no-new-privileges`, with the workspace and the teammate's declared mounts bound in. It is a per-teammate capability, not a gateway server, and does not widen reach. Its threads share it through an exclusive lease, below. |
| Secrets | `computer.secrets` | none | Named values from the operator's store (see below), in the environment of every job that computer runs. The computer redacts each value from what its tools answer and nothing returns one. A record from before the field, or a name nobody ticked, grants nothing. |
| Extra folders | `folders` | none, including on older records | Folders besides the workspace, each read-only unless `writable`. Under `workspace` reach Hotline Agent's file tools open each through its own cap-std handle and the confined shell binds or allows it (read-only or read-write) at its own path; nothing else outside the wall opens. `machine` reach makes them moot. Hotline's ACP file callbacks honour them the same way; a harness that advertises `additionalDirectories` is handed them, and its own reach stays its own. A computer mounts each at `/home/agent/folders/<name>`, `:ro` unless writable. Set only through `persona.create`/`persona.update`, which refuse `/`, the home directory or anything holding it, Hotline's data directory (other than a teammate's workspace inside it), the teammate's own workspace, overlapping folders and more than 16. |
| ACP harness | `backendId` other than `hotline` | Hotline Agent | Trust in that harness: its process, tools, configuration and permission policy are its own, outside Hotline's sandbox. Hotline's file callbacks for it stay in the workspace and its granted folders whatever its saved `reach` or advertised mode says. Its runtime mode is shown as *Externally managed*. |

Reach, gateway, collaboration, background work, extra folders and the
computer are independent axes. Changing one never changes another; in particular
`machine` reach grants no gateway server, and neither ACP mode nor a
computer counts as `machine` reach for collaboration.

## Extra folders

- **Default and old records:** `folders` absent is none, and every record
  from before the field has none. No older reach, mount or path is turned
  into a folder grant.
- **Grant source:** the person, through `persona.create` and
  `persona.update` at the desk or an owner seat. No agent tool sets it, and
  the phone's narrow commands cannot name it. `wire/commands.rs`
  `checked_folders` checks each path and stores the directory it resolves
  to, before anything is revoked or written.
- **Enforcement:** `tools::Workspace` opens each granted folder as its own
  cap-std `Dir` and resolves an absolute path in it inside that handle, so
  `..` and symlinks cannot leave it; a write, edit, create or ACP write in a
  read-only one is refused before anything (even a parent directory) is
  made. A folder whose name no longer resolves to the stored directory is
  not opened. The confined shell gets the same list: Seatbelt `subpath`
  rules with or without `file-write*`, and bubblewrap `--ro-bind`/`--bind`
  before the workspace. Every handle carries the session's lease. The ACP
  callback workspace (`driver/acp/session.rs` `callback_workspace`) is built with the
  same folders. Windows offers no shell under workspace reach, as before;
  its file tools are held to the folders.
- **Revocation:** `folders` is a reattaching key, so a change invalidates
  the teammate's lease before the new record is written: every handle built
  with the old set refuses its next call, and the restarted session opens
  the new set. A computer records the folders it was made with
  (`HOTLINE_FOLDERS`) and is removed and made again at the next start when
  they differ, so a folder taken away is no longer mounted.
- **What Hotline does not enforce:** whole-machine reach already opens
  everything, so its folders are advice in the preamble. An ACP harness is
  handed the folders as `additionalDirectories` when it advertises them, and
  what its own tools then reach is its own; ACP has no way to say a folder
  is read-only, so only Hotline's callbacks hold it to that.
- **Residual risk:** a writable folder is the agent's to change as much as
  its workspace is, including any secret in it; a read-only one can still
  be read whole and what is read can leave over the network. A workspace
  Hotline made, in its data directory, cannot be granted, so its
  `.hotline-home` stays its teammate's; a folder the person chose as two
  teammates' workspace is theirs to share. Remaking a computer when its folders change keeps
  its named volumes (home, store, scratch) on Docker and Podman, but Apple
  container keeps nothing but the rw layer, so its desktop starts over.
  A file already written before the change stays written.

## Accepted risks and known limits

These are the choices, so a change that "fixes" one of them is a product
decision for George, not a bug fix.

- **Network is on under every reach**, including localhost. Agents install
  things. The shell boundary is filesystem isolation, not network
  isolation: a local service can expose files or privileged actions of its
  own, and data the agent can legitimately read can leave.
- **A grant authorizes the grantee's own permissions.** A granted stdio MCP
  server, a computer, and an ACP harness each run with whatever the host
  gives them. Hotline states this in the ledger and the Reach card and does
  not pretend otherwise.
- **The workspace is the agent's**, along with `.hotline-home/` inside it and
  the private caches there. Teammates sharing a working directory share
  those.
- **Dispatched side effects survive revocation.** Revocation refuses the
  next call; it cannot recall a remote tool call already in flight or undo
  a file already written.
- **Platform differences.** Linux has mount, PID and IPC namespaces. macOS
  has default-deny Seatbelt with an enforcement probe, private `TMPDIR`,
  and denied AppleEvents, LaunchServices, launchd and Unix control sockets,
  but no namespaces: root directory names and ancestor metadata can remain
  visible, and programs that hardcode `/tmp` fail. Seatbelt's CLI is
  deprecated and its policy language undocumented; the probe fails closed
  if enforcement stops, which is protection against silent breakage, not a
  compatibility guarantee. Windows has no confinement Hotline can ship, so the
  confined shell is not offered there. No platform has resource limits or
  syscall filtering on the shell.
- **A served desk's secrets are files.** `hotline serve --store file`
  keeps provider keys, stored secrets and the Remote identity as 0600 files
  under `<data>/secrets/` (0700), because a server has no session bus and no
  unlocked Secret Service. It is chosen by the operator on the command line
  and recorded in the room (`store.json`); nothing falls back to it from the
  native store, and a desk started on the other store refuses the room
  instead of starting empty. There is no encryption layer of its own: a key
  handed in by systemd (`LoadCredentialEncrypted=`) would protect only these
  files. ChatGPT and Copilot login tokens use this same selected backend
  (see [ChatGPT and Copilot sign-ins](#chatgpt-and-copilot-sign-ins)). What
  protects them is the service account and the disk. Root on the machine, or a copy
  of its disk, snapshots or backups, reads every one: encrypt the volume and
  its backups (`docs/serve.md`).
- **The person is asked nothing at call time.** There are no approval cards
  for Hotline Agent's tools and no per-path ACLs. A capability is standing or
  it is absent.

## Voice calls

Desk and direct teammate calls use the existing owner/local desk seat; a
companion cannot start, upload audio or device text to, or subscribe to either
kind of call.
The core validates a target against the living roster before replacing a
call, and rejects a changed target on a reused call ID. Direct messages use
`Room.start`/`Room.prompt`, so the existing harness, chapter, workspace,
capability lease and revocation checks remain the enforcement points.
Direct-call readiness depends on speech and budget, independently of the desk
dispatcher; advertising it does not widen the owner/local desk seat.

Voice origin is assigned by the core, carried in opaque user event IDs, and
restored when queued input runs or the driver accepts steering. Ordinary
wire prompt parameters cannot claim it. Only committed agent messages from
the chosen persona and current call turn can enter a direct call; tool
narration remains thoughts. Holding, interrupting and disconnecting cancel
speech and unfinished input, while accepted teammate work retains the
ordinary session lifecycle. No action runs on a partial transcript.

Negotiated `inputMode: "text"` accepts one finalized `voice.text` transcript
instead of remote STT. It uses the same owner-seat handler, connection-bound
call, increasing utterance sequence, unfinished-turn gate, cancellation tokens,
internal origin and existing session grants as audio. The core rejects blank
text, more than 8,000 characters/32,000 UTF-8 bytes, duplicate commits and input
from the wrong mode before dispatch. Text carries no caller-supplied origin,
new agent tools or authority. It requires output configuration only, reserves
no remote STT spend, and retains the Chat and Voice budget gates for a desk
call's billed call assistant and paid speech. A direct call's text is a turn
of the teammate's session with the session's own grants; the contract added
after the words is text, not authority.

Live PCM is negotiated, ordered and capped at 20 seconds/640,000 bytes,
with 32 KiB frames, bounded queues and deadlines. Provider keys stay on the
desk. Paid xAI speech requires an API key; the separate explicit Grok
subscription choice requires its stored login and never uses a paid speech
fallback. Both output paths check fallback credential revocation before use.

A voice front (ADR 0004) fronts calls by default when the vault holds a
login or key that can run one: Grok Voice on a Grok login or xAI key, or GPT
Realtime on an OpenAI API key, or GPT-Live on the ChatGPT sign-in, when the
owner picks one in Settings (the call's audio then goes to OpenAI; for
GPT-Live the login's bearer goes to `chatgpt.com` to create the call and to
`api.openai.com` for its sideband, as the Codex app sends it, and the call's
media is a WebRTC peer the desk opens with host candidates only). Legacy
voice turns it off, and a client asks for it per call. The Grok login is
only ever sent to xAI's voice origin, and GPT Realtime refuses it. GPT-Live
uses client delegation to invoke the same `work_on_it` handler as the other
fronts. The front holds read-only lookups and one work tool. The core binds
reads to the call thread and the teammate's current capability lease; writes,
MCP calls and shell work run on the call's independently leased clone. The
front has no direct posting or raw MCP tools. A policy change revokes both
paths. `validate_call_thread` checks ownership and lifecycle before work or
cancellation; `work_on_it` refuses a held or non-live call. The clone uses the
teammate's own model and harness without steering the home session.
Its starting prompt is a scrubbed, budgeted slice of the DM and identity, never credentials. Mid-call instruction updates add bounded scrubbed speech, interruption notices, running work and confirmed posts from the same call. Speech-to-speech audio cannot be filtered before it is heard: the identity prompt, tool framing and opt-in live scripted-front regression deck enforce phrasing, rather than a runtime audio filter.
The work brief includes the whole call transcript, including tools. Hanging
up closes the voice socket but leaves accepted work running. Budget exhaustion
or a revoked login ends the front visibly. Residual risk: the front can
misinterpret a request; its recorded words and tool calls permit review.
ACP harness tools retain their existing sandbox boundary. Draft-versus-send
intent is a model instruction; it does not grant new capability.

Proofs: `wire::tests::voice_is_owner_only_through_the_real_handler`, the
headless `tests/voice/direct.rs::device_text_uses_the_call_thread_and_preserves_its_spoken_reply`,
`tests/voice/front.rs::a_front_call_works_on_its_thread_and_says_its_spoken_line`,
direct-call and dispatcher-unavailable readiness tests, and the finalized-text
replay, pending, mode, bounds, budget, STT-accounting, direct origin, live commit,
cancellation and streaming failure tests in `voice::room::tests` and
`session::tests`. Provider adapter tests use local HTTP/WebSocket fixtures;
live provider and iPhone compatibility still require device verification.

## The desk's own speech models

The desk can hear with a speech model on its own machine (`voice.md`, Hearing
on the desk). It is an operator action on the desk, not a capability of any
teammate: no tool, prompt or driver path reaches it, and it hears only audio
the owner's own clients send to calls and to `voice.transcribe`.

- **Default.** Nothing is installed, offered or loaded. A room from before
  the models has none.
- **Grant source.** The owner's `voice.model_install`, over the local desk or
  an owner seat. `voice.models`, `voice.model_install`, `voice.model_cancel`,
  `voice.model_remove` and `voice.transcribe` are absent from the companion
  allowlist in `wire::Seat::permits`, so a phone companion is refused all of
  them; no agent tool names them.
- **Enforcement.** The core downloads only the archives in its catalogue,
  each pinned by size and SHA-256 in `hotline-voice`'s `speech/local/install.rs`; an archive is hashed
  as it arrives and deleted unread unless it matches. Only the four file
  names a model is made of are taken from it, into a directory of Hotline's
  choosing, so no path, link or extra entry in an archive chooses where a
  byte lands. A model appears by renaming a complete directory into place.
  Before a model's first load in a run each file is hashed against what was
  recorded when it was unpacked, because the engine's C++ exceptions cannot
  be caught and a damaged file would stop the desk; audio under a tenth of a
  second, which also throws, never reaches it.
- **What it changes for spending.** Its price is zero and `voice.transcribe`
  asks no budget, so a zero limit never stops it and it never spends.
- **Residual risk.** The pin trusts what NVIDIA trained and sherpa-onnx
  converted on the day it was pinned; a model is data that onnxruntime
  parses, so a hostile one could attack the parser, which is why only pinned
  archives are fetched. The engine's static library comes from that
  project's GitHub release for the exact crate version in `Cargo.lock`:
  `scripts/fetch-speech-engine` keeps the archive only if it matches its
  SHA-256 in `scripts/speech-engine.sha256`, and the build links only what
  that script left in `.speech-engine`, never downloading one itself. The
  build script comes from an upstream commit (the `[patch.crates-io]` in
  `Cargo.toml`) until a sherpa-onnx release builds with ZIP 9. Anyone who can write the data directory can replace a
  model's files and its record together, as they can every other file there.
  Audio sent to `voice.transcribe` is decoded in the desk's process, AAC by
  symphonia, which is pure Rust.

Proofs: `wire::tests::the_desks_speech_models_are_the_owners_through_the_real_handler`;
the headless `tests/local_speech.rs::a_model_is_downloaded_verified_unpacked_offered_and_removed_only_when_the_owner_asks`;
`hotline-voice`'s `speech::local::tests` for what counts as installed, a damaged model
refused before loading and a click heard as nothing; `hotline-room`'s `voice::ledger::tests::the_desks_own_hearing_costs_nothing_batch_or_live`.
`a_real_model_installs_and_hears_what_was_said` runs a real model with
`--ignored` when `HOTLINE_TEST_SPEECH_MODEL` names one.

## Generated images (BRO-174)

`generate_image` is available to teammate sessions on Hotline Agent and ACP,
not to their subagent runs. It uses only providers already connected by the
owner; no key or provider response body appears in the result or a failure.
`settings.images`, `settings.spending`, `images.status` and `capabilities.options` require the local
desk or owner seat. Companion status requests are refused before dispatch.

Reference files use the same cap-std workspace handle and current capability
lease as the file tools. Workspace reach refuses traversal and escaping
symlinks; machine reach can read elsewhere. Output always lands inside the
teammate's workspace, even with machine reach, using create-only publication
so an existing file or symlink is never overwritten. The kept conversation
copy uses `send_file`'s posting and readback path, and the `post_to_chat`
gate it shares on a job's turn; generated
PNG, JPEG, WebP and SVG bypass photo-to-JPEG normalization, preserve alpha
and are limited to 2048 pixels on their longer edge. Decoding and reference
reads have byte and allocation ceilings.

Each dollar-billed provider attempt reserves its estimate durably before the request,
including the single fallback. A successful result settles its reported
cost, or the estimate if absent. Provider 400–499 refusals settle at zero;
timeouts, server failures, malformed responses and cancelled requests retain
their estimates, because they cannot prove the provider did no paid work.
The minimal ledger lives at `<data>/spending.json`, under the room's one
writer, with UTC day/month caps and atomic replacement. Corrupt saved caps
or a corrupt, missing-after-use, unreadable or unwritable ledger block
spending rather than restore an empty budget. Reads and saves retry on the
next call after storage recovers; a bad ledger keeps refusing until repaired.
Failed settlements are retained and retried before admitting more spending;
once replacement succeeds they are not applied twice if directory sync fails.
A backward clock keeps the future ledger period and its higher totals instead
of resetting usage. The image ledger is held to the Images budget of
`settings.spending`; either of its limits at zero disables it, and no limit
is no limit. Reservation and settlement
IO, including fsync, run on blocking workers, not async runtime threads.
No room or ledger mutex stays held across generation. The capability lease
is checked again before fallback, output writing and conversation posting.

`session::generate::tests` drives the real Rig and MCP entry points against
a fake `ImageGen`, and the resolver/HTTP adapter against a connected mock
provider. It proves workspace/chat copies, accounting, denied budgets and
references, fallback reservation, disabled spending, excluded subagents and
revocation. `sent::generated::tests` proves format/alpha retention and size
limits; `spending::tests` proves durable, concurrent, fail-closed accounting.
The wire tests prove owner/desk status and companion refusals.

Subscription images (a Hotline ChatGPT sign-in, or a Grok sign-in) are the
resolver's first automatic choice, ahead of paid keys. A subscription falls
back only to another subscription, never to a paid API; a paid selection may
fall back to a subscription, which costs no dollars. The draw loop enforces
the same boundary: after a subscription attempt it stops at the first paid
generator. ChatGPT reuses Hotline's saved login and noninteractive Rig
refresh, never Codex CLI's credential directory; the image destination is
fixed to the ChatGPT Codex backend. Grok reuses Hotline's saved xAI device
login and its shared refresh lock, sending the bearer only to
`https://api.x.ai/v1`. HTTP redirects are disabled for both. Subscription
requests bypass dollar reservations without modifying the ledger and report
unknown monetary cost (`costUsd: null`, `billing: "subscription"`). Dollar
caps do not constrain subscription quota. The tool still validates settings,
checks reach and capability leases, and posts through the same file path.
The adapter rechecks the caller's lease after authentication refresh and
immediately before dispatch, so stopping the teammate during refresh sends
neither prompt nor references. Refresh timeouts report a retryable delay,
not an invalid login.
`imagegen::tests` proves the JSON request and the login's token and account
headers against localhost, and safe refusals; resolver tests prove subscriptions first and
no paid fallback. `session::generate::tests` proves subscription output and
unchanged dollar accounting even with disabled or exhausted budgets.
`imagegen::chatgpt::tests` covers revocation during refresh with a local HTTP
server, and the tool-handler tests prove that it leaves no file or chat post.
`wire::tests::chatgpt_images_status_is_automatic_without_checking_entitlement`
proves through the real handler that ChatGPT images are used automatically once
the person is signed in, and that a companion is denied. Status follows the
sign-in, not an entitlement check. Live compatibility is unverified;
the internal upstream endpoint can change or reject an account independently
of a successful sign-in. No automatic retry or paid fallback masks that failure.

Residual risk: dispatched provider work can finish and cost money after
revocation; it cannot be recalled. A reported price above its estimate is
recorded and blocks later work if over the cap, not retroactively prevented.
Unknown failed-request costs are estimates, not invoices. Failed settlements
stay in memory until storage recovers; restarting first retains only the
durably reserved estimate. Generated files
and chat copies share the existing retention policy; no automatic cleanup
is added. Voice will use the same spending interface when its branch is
integrated; it is not charged by this image-only branch yet.

## ChatGPT and Copilot sign-ins

The upgrade harness compiles `provider_fixture` under the core's dev-only
`test-support` feature. It can replace Anthropic's origin for one fixture key
and ChatGPT's model and sign-out origins for one fixture login directory
with a loopback HTTP server. This code is absent from normal builds, has no
wire or settings entry point, and keeps the real signing and vault paths in
the test.

Rig reads and writes a ChatGPT or GitHub Copilot sign-in only through a token
store the host hands it. Hotline's `LoginDirStore` (`providers/rig_login.rs`)
uses the vault's `CredentialFile`: tokens live in the OS credential store,
with a private reference in `vault/logins/<id>/`. A desk explicitly started
with `--store file` uses its file backend for these tokens too.

- **Default.** A person signed in stays signed in. The first read moves
  plaintext `auth.json` (ChatGPT) or `copilot.json` (Copilot) into the selected
  store, verifies every chunk, then atomically replaces the plaintext with a
  reference. Copilot's older `access-token` and `api-key.json` move directly
  into that store, field for field; both old files remain until the write
  succeeds. A keychain failure preserves the plaintext and refuses to use it
  as a fallback. New logins start with an empty directory and write their
  first record through the vault. No token is in the room stream, on the
  wire, in a contract type or a failure card.
- **Grant source.** The person's device sign-in on the desk, as before. A
  teammate is never handed a token: the core signs its model requests as
  they leave (`providers/outgoing.rs`), to the origin Rig addresses for that
  provider, with redirects disabled.
- **Enforcement points.** One authenticator per login directory serves turns,
  voice, image generation, discovery and the failure probe. Clients share a
  refresh and an in-memory token; the store's per-record lock coordinates
  other authenticators, including device sign-in. Only the two known record
  keys are accepted. The vault registers its configured backend by login
  root so every caller uses the same store. Credential reads, writes and
  deletes run under `spawn_blocking`; `CredentialFile` verifies chunks before
  swapping references. Abandoning or deleting a login deletes both records'
  chunks before removing the directory. `AuthError::Store` reaches the
  failure card as “Unlock your keychain”, including through Rig's HTTP error
  wrapper, rather than asking for a new sign-in.
  Revoke, delete and abandoned sign-in evict the shared authenticator;
  ChatGPT also calls Rig's `forget()`. The guarded `CredentialSource` checks
  the directory and room record before and after resolving a token, including
  on a retained Copilot transport's next request. Copilot has no `forget()`
  API: invalidation drops its authenticator from every retained guard.
  Local token reads and writes serialize with revoke/delete, so a refresh
  cannot restore a deleted record or save tokens for a revoked login.
  Deleting a ChatGPT login
  first revokes it locally, then calls Rig's `sign_out` with a ten-second
  bound, then removes it locally even if provider revocation failed. Deletes
  serialize per login; Rig revokes each stored refresh and access token once.
  Copilot has no provider revoke API for this sign-in; Settings offers
  “Revoke at GitHub” to open the person's GitHub applications settings.
- **Tests.** The sign-ins row of the [regression matrix](#the-regression-matrix),
  including `rig_vault_*` tests on an injected `MemoryStore` and a failing
  wrapper, plus `rig_revoke_*` tests for retained transports and deletion
  through the real wire handler against successful and failing revoke endpoints.
- **Residual risk.** An unread legacy login remains plaintext until first
  use; a failed migration deliberately preserves it. The explicit file
  backend remains owner-only plaintext. Native keychain prompts and access
  depend on the OS and application signing; George checks a signed-in macOS
  profile before release. A request already dispatched cannot be recalled.
  Failed or timed-out ChatGPT revocation may leave a provider token usable
  outside Hotline; Copilot access at GitHub lasts until the person revokes it
  there. Sign-out needs readable token storage to revoke at the provider;
  local storage failures still surface to the person. Older
  Hotline versions cannot read the references and require sign-in after a
  downgrade.

## Operator actions are not agent capabilities

Some things the person does from the desk are one-shot transfers, not standing
grants: attaching a file to a prompt, adding a mount, and importing a host
browser's cookies into a teammate's computer. They use the person's own
authority through the authenticated desk wire, are gated to owner or local desk authority, and
add nothing to what the agent may reach on its own. They are not a call-time
approval prompt, which the standing-consent rule forbids, because the person
initiates them. Host-cookie offers below are a separate, always-asked path;
the agent has no tool that selects or accepts their source.

Cookie import (`computer.browsers.list`, `computer.cookies.preview`,
`computer.cookies.import`, `computer.cookies.list`, `computer.cookies.forget`)
is the sharpest case, so it is spelled out. The agent has no tool that reads
the host's browsers; these commands require owner or local desk authority, and the companion
allowlist does not name them, so the model cannot pull cookies whatever it is
told. The person chooses the browser, the profile,
and the exact sites; a preview carries domains and counts, never a value. On
import the chosen cookies pass host → desk → container over the container's
authenticated loopback port and are written into the sandbox the person already
granted; they never enter the tape, the model's input, or a log. What the
agent gains is a browser already signed in to sites the person picked — the
same exposure as the person signing in there by hand inside the computer, and
the accepted risk of giving an agent a logged-in browser at all. Expired
cookies never leave the host: the browser would drop them on its next look,
and they would only pad the list. What was brought over is recorded — browser,
profile, time, domains and counts, never a value — one record per teammate on
the room stream, and listed on the teammate's pane, where each site and each
browser's whole import can be taken back. `computer.cookies.forget` names the
exact domains to the computer's bearer-guarded `DELETE /logins/{name}`, whose
browser drops those cookies at once and whose saved login loses them, so what
the agent's browser is signed in to is what the pane shows.

### Image readback is a copy of the person's transfer

The desk and paired phone may read user image attachments through `file.read`,
by living teammate, message id and original attachment index. This adds no agent
tool or grant and does not itself accept a source path. `session.prompt`
is available to owners and the local desk; `mobile.prompt` still resolves only completed device-scoped
uploads. The shared session funnel captures recognized PNG/JPEG/GIF/WebP bytes
at send time, without transcoding, into a per-message, per-index copy. Readback
checks the user event and image kind and only opens that copy, with the same
25 MiB file / 512 KiB decoded chunk bounds as teammate readback. Non-image user
attachments are not readable, regardless of their filename or declared MIME.

Old events have no trusted send-time copy and are refused, not validated against
a guessed mtime or their optional size. Missing or failed copies likewise never
fall back to a source path. Original paths and agent input retain their existing
semantics; this protects phone readback, not the bytes an agent later opens.
As with teammate kept files, the data directory is trusted: an actor able to
rewrite retained files or their parent directories is not confined by this
feature. Source size/mtime checks detect ordinary concurrent edits during
capture, not malicious edits which restore timestamps. There is no total
retained-image budget or cleanup, so repeated messages consume additional disk.

## Secrets a teammate uses without seeing

The operator keeps three kinds of secret in the vault, each under a name.
A variable — a GitHub PAT, an npm token, a Stripe test key — is a value
that becomes an environment variable: `secrets.set {name, value}`. A login
is one or more sites, a username, a password and optionally a TOTP seed:
`secrets.login.set`. A passkey is a WebAuthn credential of the teammate's
own, which is never sent in but made (below). Every record is written to
the OS credential store (macOS Keychain, Windows Credential Manager, Linux
Secret Service, the same opaque-reference records as provider keys; no
plaintext fallback), beside a private sidecar that says only what the
record is for — kind, sites, username, whether there is a seed; site and
user name for a passkey — so `secrets.list` answers that without opening
the keychain. No reply, room event or subscription carries a password, seed or private
key: the store is write-only from the window. These management commands
require owner or local desk authority. The bounded
`secrets.capture.answer` flow below also allows the phone seat.

A teammate gets a secret only by the operator ticking its name on that
teammate's computer, `computer.secrets`, a standing choice like a mount,
nothing pre-ticked. `persona.update` changes that set from the desk;
answering a credential card also ticks its one key for its bound requester. At every grant — session start, reattach, and whenever a stored
value is replaced or deleted — the desk reads the granted values from the
vault and hands the computer the whole set through `PUT /secrets` on its
authenticated loopback port, bearer in a header. A stopped computer is
handed nothing until its next start, which hands it the current set. The
container keeps the set in memory and puts it in the environment of every
job the agent starts through `shell` or `files run` — above the workspace's
saved environment, under the agent's own explicit `env` — and of no
preparation job, because what preparation captures is written into the
workspace. It redacts every value from every tool result at the one point
all its tools pass through, as `[redacted NAME]`, in the spelling JSON
gives a value too. `/secrets` is PUT-only; there is no route that returns
a value, because the bearer is the container's door key and not a secret:
a job no longer inherits it, but anything running in the container can
still read its service's environment, so holding the bearer must never be
worth a value. The agent is told the names and kinds, in the preamble and
by `state info`, and told it will never see a value.

A login goes to the computer as a record with its sites, and is typed only
there. The agent asks `browser fill` for `NAME.username`, `NAME.password`
or `NAME.code` — the six TOTP digits of this moment, computed in the
container from the seed — by name, in place of text; the computer types it
into the referenced field only when the page's origin is one of the login's
sites, same scheme and port, same host or a subdomain of one, and a site is
`https://` unless it is localhost. Any other page is refused with a
sentence, and the preamble tells the teammate that refusal is right. The
password and the digits are redacted from every answer as `[redacted
NAME.password]` and `[redacted NAME.totp]`; the username and the sites are
not secrets and are not. What the origin rule buys is that a page the
teammate was steered to — a phishing copy, an `http://` downgrade, a look-
alike host — cannot have the password typed into it; what it does not buy
is anything after a genuine sign-in, which is the same exposure as a
person signing in there by hand inside the computer, the accepted risk of
giving an agent a signed-in browser at all.

A passkey is the teammate's own, because a person's never leaves their
authenticator. The computer keeps every granted passkey in a WebAuthn
virtual authenticator on each browser tab, from the delivered set, so a
site's `navigator.credentials.get()` is answered inside Chromium and the
private key is never typed, never in a tool answer (`[redacted
NAME.privateKey]` if it ever were), and never in the model's input. Making
one is the operator's act, twice over. First the arming:
`secrets.passkey.register`, from the teammate's pane, arms that teammate's
computer for one site for ten minutes (`PUT /passkeys/registration`,
bearer-only, the computer started if it was stopped). Then the approval:
the operator signs in to the site through the teammate's screen and adds a
passkey in the site's security settings — or asks the teammate to — and
the site's `navigator.credentials.create()` is not answered by the browser
on its own. A guard script the computer installs on every document, keyed
by a token the page cannot read, rejects the call outright when nothing is
armed or another site is; under the arming it parks the request with what
the site asked for — the site, the origin, the account name and display
name — and the computer's next look records it and answers it on `GET
/passkeys/registration` as `asked`. The room, which polls that door every
two seconds while an arming stands, writes a `passkey_ask` card on the
teammate's tape and tells the phones, and the person's answer goes back
through `secrets.passkey.answer` and `POST /passkeys/registration/answer`.
Approved, the page is told to go ahead and the browser's authenticator
mints; denied, the site gets a `NotAllowedError` and the arming ends with
the denial, so neither a site nor a teammate can keep asking. One request
is before the person at a time; a request whose page went away, an arming
that ran out or was cancelled, and a computer that stopped each leave the
card expired rather than answerable; and an answer to a request that is
not waiting is refused by the room and by the computer (409), so a stale
card cannot let one through. The card has the standing of a permission
card — one answer to one request the operator armed for at the desk —
which is why the phone may give it, while arming stays at the desk. The
computer's own check on every look removes from the authenticator any
credential that is neither in the delivered set nor minted under an
approved request of the current arming, so a teammate cannot give itself
a passkey, keep one made for another site, or keep one after the ten
minutes. The look that finds the credential minted stores it in the
vault, ticks the name on `persona.computer.secrets` for that teammate —
the same tick the pane makes for any secret, made for the operator because
they asked for this passkey for this teammate — hands the computer the set
with it, which is what keeps it in the authenticator, `DELETE`s the
arming, and says so on the tape. That answer is the one time a private key
leaves the container: over the bearer-guarded loopback door, in the
direction the cookie import already trusts, into the vault, and into no
tape, room event or log. Revocation is any of three: untick it on the
teammate or remove it under Secrets, both of which hand the computer a set
without it and the authenticator drops it on the next look; or delete the
passkey in the site's own security settings, after which the credential
the computer holds signs nothing. The desk's floor for a new computer is
0.9.0, the first release that asks before it mints; a 0.8.x computer mints
under the arming without asking and the room stores what it minted as
before, an older one refuses a login record with 400 and has no passkey
door, and the pane says so and points at Update.

What this enforces: no agent tool reads, sets or grants a secret; a value
passes host keyring → desk memory → container memory → job environment and
is written to no tape, no log, no room event and no model input; and the
computer scrubs the value as it is from what the model reads. What it does
not enforce, stated plainly: a model that deliberately encodes a value —
`base64`, splitting it, printing it on the desktop and taking a screenshot
— can still read it, because the sandbox has to be able to use the value
with arbitrary programs. Redaction is a guard against incidental exposure
(`curl -v`, `env` in a debug dump, a tool printing its configuration), not a
wall against a determined model; the wall is the grant, and the guidance is
to grant a teammate only the secrets its job needs. Jobs already running
keep what they were started with when a secret is revoked, the accepted
"dispatched side effects survive revocation" rule; a running computer that
cannot be told of a change keeps its set until it is stopped, and the
teammate's tape says so. A release from before `/secrets` answers 404: the
teammate starts, the tape names what is not in its shell, and the pane's
Update is the fix. Network-layer injection, where a proxy adds the header
and the value never exists in the sandbox, is the stronger follow-up for
HTTP APIs and is not built.

## The method for a new capability

Follow this when adding a tool, a tool origin, a driver behaviour, or a
path by which one teammate's work reaches another's. Each step is a
question the change must answer in its PR description.

1. **Inventory the paths.** List every way the new thing executes or
   reads: Hotline Agent tool closures, the in-process MCP server, ACP file
   callbacks, the scheduler's queued turns, cached peer sessions, and the
   wire. If a path is missed, the boundary has a hole in exactly that
   place; the collaboration and background-work steps each found one in
   the built-in tools because those are always registered rather than
   granted.
2. **Name whose authority a request uses.** The caller's? The recipient's?
   The person's (only through the authenticated desk wire)? A request that
   arrives through a peer session carries the caller's lease as a
   dependency, so revoking the caller kills work done on its behalf. Never
   let a request borrow the authority of the process it happens to run in.
3. **Choose the default and the migration.** The default grants nothing
   beyond the workspace. Decide explicitly what an existing record without
   the field means, and write it down beside the field in `contract`;
   "missing means off" is the house answer, and old implicit access is not
   translated into an explicit grant.
4. **Enforce outside the model, at every entry.** The check is a
   `CapabilityLease::check()` (or the policy read that it guards) in the
   handler, not a sentence in the preamble. Both drivers reach the same
   handler; a check that exists only on one driver's path is a hole on the
   other.
5. **Make revocation reach it.** A new handle is cloned from the session's
   lease or scoped from it; it is never a fresh lease of its own. Confirm
   the change is covered by the lifecycle below, including queued work and
   sessions cached for a peer.
6. **Tell the truth in three places.** The ledger row, the Reach or Tools
   card, and the preamble say what the code enforces, and distinguish what
   Hotline enforces from what a grantee enforces for itself.
7. **Prove it both ways.** A test that the allowed action works and a test
   that the denied action is refused, through the real handler (see the
   matrix). A denied path that only a model's good manners keeps closed
   does not count.

Do not add an ACL editor, a per-call approval prompt, or a new policy field
to solve a problem the existing axes already express; if a new axis is
genuinely needed, that is a design decision recorded in
[design.md](design.md) first.

## Grant lifecycle

Authority is a `CapabilityLease` (`capability.rs` in `hotline-tools`), a
snapshot of a teammate's `CapabilityEpoch` generation. Every handle a session
hands a driver — tool closures, the workspace directory, cloned MCP tools
(`McpTool`, in `hotline-mcp`, which takes the lease from `hotline-tools`),
ACP callbacks, the OAuth proxy — clones the same lease and calls `check()`
before acting. Their fields are private to that module, so another crate can
capture, scope, tie and revoke a lease but never forge or revive one.

- **Creation.** `Room::start` takes a lease and checks it before and after
  reading the record, so a start racing a policy update cannot capture the
  old policy in a usable lease.
- **Policy change.** `invalidate` advances the generation and closes the
  epoch *before* the new record is appended; `activate` reopens it after
  the append is durable. In between, every lease — existing or newly
  captured — is dead, and a failed append leaves the teammate visibly
  stopped and refusing work until a retry succeeds. The wire holds one
  room-wide `policy_update_lock` across invalidate, append and reattach so
  a second socket cannot reactivate a generation between the steps.
- **Stop.** Advances the generation and leaves the epoch open: old handles
  are permanently dead, a later explicit start uses the current policy. A
  stop also invalidates a replacement session captured before its startup
  began.
- **Delegation.** A peer session holds a lease `scoped()` from the caller's
  and dependent on the target's. Peer teardown revokes only the scoped
  lease; a stop or policy change on either side revokes the peer through
  the shared epoch. Third-party sessions delegated from a peer follow the
  same chain and stop when the original request's authority goes, while
  each teammate's independent main session stays usable.
- **Caching.** Cached peer sessions are revoked whether the changed
  teammate was caller or recipient, including peer-only sessions whose
  main session is stopped.
- **Active and queued work.** Revocation cancels the activity's shell jobs
  and settles their results, clears queued turns, and releases collaboration
  and human waits so a late answer cannot establish a grant. Stale turns
  cannot publish state or checkpoints. Unix shell cancellation, deadlines and
  task drops send `SIGSTOP` to the owned process group before `SIGKILL`, so
  killing a waited child cannot wake its shell into the next statement while
  the kernel is still delivering the group kill. Cancellation still observes
  exit before reporting success. Already performed effects remain, and a
  process that deliberately leaves the owned group is outside this guarantee.
- **Running processes.** A shell command still running when a Hotline Agent
  turn replies without waiting is handed to the teammate's DM session
  (`session/jobs.rs` `Processes`), so it outlives the turn. Default: nothing
  new is granted; the command was launched under the shell grant and the
  turn's lease, and only the DM session, the one that subscribes to its
  wakes, takes processes (a subagent run, side thread or peer session still
  waits on its own commands). Enforcement: the driver holds them and
  `invalidate` kills every one, so a stop, a removal, a grant change
  (which rebuilds the session, by design) or a revoked lease kills them with
  their whole process group; dropping the driver or quitting the desk drops
  their tasks and the shell's group guard kills them, as for any job. The
  launch refuses a fifth command, counting the processes earlier turns left,
  and names the running ones. A wake from a process that ended on its own is
  queued only while the session is current and its lease is. The person
  sees each process on the session's `processes` and reaches it through
  the driver (`Driver::processes`): `process.output` reads its whole command
  and the last 16 KiB of its output, and `process.stop` cancels it without
  waking the teammate. Both take the teammate's id and find the process
  only in that teammate's current session, and both are the desk's and the
  owner phone's, not a companion phone's, as `session.stop` is. Residual
  risk: a model that neither waits nor means to leave a command running
  gets its result a turn later, at one model request per exit; a process's
  output is held in memory until it exits, as a job's is; and the tail is
  shown to whoever holds the desk, as the transcript is.
- **Schedules.** The grant is read when a job is created, again before a
  wake, and again immediately before a queued firing reaches the driver.
  Each queued firing carries its own provenance because a one-shot may be
  tombstoned by then. Revoking background work pauses agent jobs without
  deleting them.
- **Subagents.** A Hotline Agent teammate's own session may hand a task to
  a subagent: a clone of the teammate (see
  [Clones of the home agent](#clones-of-the-home-agent-adr-0008-0)), started
  as a managed job of the turn that asked. The run holds a lease `scoped()`
  from that session's, so a stop, a policy change or a revoked session
  revokes every run it started, and the run's end revokes the run's lease
  alone. A run is the teammate's permission set, the computer included,
  less what the home agent left out with `without`, and never more: no
  subagents of its own, and of Hotline's tools only the reading ones
  (`search_thread`, `list_chapters`, `list_teammates`, `list_schedules`,
  `computer_status`, `web_search`) — nothing that asks the person, posts,
  reacts, messages a colleague, schedules, or moves a chapter. It writes to
  its own stream and one line on the tape; its report reaches the teammate
  as execution data, which the teammate is told to verify rather than
  obey. The parent turn cancels and settles its runs however it ends, and a
  run left running by a dead process is settled as cancelled at startup.
  There is no per-teammate subagent configuration to widen any of this.

## The regression matrix

Each row is one thing that must stay true, and the test that proves it
through the real handler. `make check` runs everything not marked
otherwise. When a change touches a row, the named test is the one to
extend; when a change adds a boundary, it adds a row.

| Must hold | Proof | Needs |
| --- | --- | --- |
| Finalized device text uses the existing owner/local desk call authority; companions cannot start, commit or subscribe; duplicate commits and mixed input modes are refused; disconnect cancels further input; direct replies stay on the call thread and retain their spoken version, with no new grants or remote STT spend | `wire/tests/owner_handler.rs` `voice_is_owner_only_through_the_real_handler`; `tests/voice/direct.rs` `device_text_uses_the_call_thread_and_preserves_its_spoken_reply`; `voice/room/tests/text.rs` `finalized_text_skips_stt_and_cannot_replay_or_overtake_a_pending_turn`, `text_and_audio_calls_enforce_the_negotiated_mode_and_transcript_bounds`, `canceled_text_is_not_dispatched_and_disconnect_ends_its_bound_call`; `voice/room/tests/budgets.rs` `text_input_keeps_budget_gating_before_dispatch` | — |
| The desk's speech models are installed only when an owner or the desk asks: a companion can neither list, fetch, cancel, remove nor transcribe; an archive that is not the pinned one is deleted before it is unpacked; only a model's own files are taken, whatever paths and links the archive holds; a cancelled download leaves nothing; an installed model is offered and chosen for hearing, a removed one is not; a damaged model is refused before the engine loads it | `wire/tests/owner_handler.rs` `the_desks_speech_models_are_the_owners_through_the_real_handler`; `tests/local_speech.rs` `a_model_is_downloaded_verified_unpacked_offered_and_removed_only_when_the_owner_asks`; `hotline-voice/src/speech/local/tests.rs` `a_damaged_model_is_refused_before_the_engine_could_throw_on_it`, `a_model_is_installed_only_whole_and_under_its_own_name` | — |
| Running processes: a command still running when the teammate replies ends the turn at once and outlives it; a later turn is told of it and stops it; one that ends on its own wakes the teammate once, and on a live call the front says that turn's reply; a session stop and a grant change kill it; a fifth command is refused naming the four; a process is never waited on and a stop asked for wakes nobody, taking its whole process group; a call goes listening at the reply and is given no progress line while a process runs | `tests/processes.rs` `running_process_a_server_left_running_ends_the_turn_and_a_later_turn_stops_it`, `running_process_that_ends_on_its_own_wakes_the_teammate_once`, `running_process_dies_with_its_stopped_session`, `running_process_dies_when_its_grant_is_revoked`; `running_process_serve_open_and_stop_end_to_end`; `tests/voice/processes.rs` `running_process_that_ends_during_a_call_is_said_on_the_call`, `running_process_a_server_started_on_a_call_leaves_the_call_listening`; `session/jobs.rs` `running_process_a_fifth_command_is_refused_naming_the_four`, `running_process_is_inspected_and_cancelled_later_but_never_waited_on`, `running_process_that_ends_before_its_route_moves_still_wakes_once`; `tests/processes.rs` `process_chip_a_process_is_on_the_session_until_the_person_stops_it`, `process_chip_stop_is_refused_for_a_teammate_that_does_not_hold_it`, `process_chip_output_is_the_latest_of_what_it_wrote`; `wire/tests/seats.rs` `process_chip_only_the_desk_and_the_owner_reach_a_process` | Unix |
| ChatGPT and Copilot sign-ins migrate to the vault without signing out; failed writes preserve plaintext; deletion removes chunks; concurrent refresh spends a token once; missing sign-ins never start device flow | `providers/rig_login/tests.rs` `rig_login_a_0_42_copilot_dir_migrates_to_copilot_json_and_stays_signed_in`, `rig_login_a_failed_copilot_write_keeps_the_0_42_files`, `rig_login_a_0_42_chatgpt_auth_json_keeps_its_tokens`, `rig_login_two_clients_refreshing_at_once_spend_the_refresh_token_once`; `providers/rig_login/tests/vault.rs` `rig_vault_plaintext_auth_becomes_a_reference_on_first_read`, `rig_vault_keychain_failure_keeps_plaintext_and_asks_to_unlock`, `rig_vault_delete_removes_every_chunk`, `rig_vault_abandon_removes_copilot_chunks_and_reference`, `rig_vault_0_42_copilot_migrates_straight_into_the_keychain`, `rig_vault_s1_plaintext_copilot_migrates_into_the_keychain`, `rig_vault_failed_legacy_copilot_migration_keeps_both_files`; `tests/rig_upgrade.rs` `rig_upgrade_a_042_profile_keeps_both_signins_and_leaves_no_plaintext_tokens`; `tests/desk/logins.rs` `rig_vault_deleting_a_login_over_the_wire_removes_its_chunks`; `vault.rs` `a_copilot_login_begins_with_no_record`; `driver/rig/tests.rs` `a_chatgpt_turn_without_a_login_refuses_instead_of_starting_one`, `a_copilot_turn_without_a_login_refuses_instead_of_starting_one` | file-permission failure: Unix; native macOS keychain: manual before release |
| Workspace reach keeps another project's `.env` out of the read tool and the shell, over the wire, and the ledger says what is offered | `tests/desk/workspace.rs` `workspace_reach_keeps_another_projects_env_out_of_the_tools` | Linux with bubblewrap |
| Extra folders: granted and checked over the real wire, every refusal leaving the record as it was; in a real Hotline Agent turn a read-only folder is read and not changed, a writable one is changed, a path outside every grant and a link out of a folder are refused, and the confined shell is held the same way; narrowing or removing them over the wire is in force at the next tool call; a teammate without the field reaches none; a folder swapped for a link after it was granted opens nothing; a revoked handle refuses its folders; Seatbelt and bubblewrap grant each folder with its mode; Hotline's ACP callbacks honour the grants and a harness that takes `additionalDirectories` is handed them; a computer mounts them `:ro` unless writable, cannot have them covered by a declared mount, and is made again when the set changes; the preamble names them and who enforces them | `tests/folders.rs` `granted_folders_hold_the_real_tools_and_a_change_takes_them_back`, `a_folder_that_cannot_be_granted_is_refused_and_changes_nothing`; `hotline-tools/src/workspace/tests.rs` `a_granted_folder_is_read_by_absolute_path_and_changed_only_when_writable`, `a_path_outside_every_grant_or_out_through_a_link_is_refused`, `a_folder_swapped_for_a_link_or_never_granted_opens_nothing`, `a_revoked_handle_refuses_its_folders_too`, `whole_machine_reach_keeps_no_folder_wall`; `hotline-tools/src/shell/macos.rs` `granted_folders_are_in_the_profile_with_their_modes`, `granted_folders_are_open_to_the_shell_as_granted`; `hotline-tools/src/shell/linux.rs` `granted_folders_are_bound_with_their_modes_before_the_workspace`, `granted_folders_are_open_to_the_shell_as_granted`; `driver/acp/tests/session_new.rs` `acp_callbacks_honour_granted_folders`, `session_new_hands_the_granted_folders_to_a_harness_that_takes_them`; `computer/tests.rs` `granted_folders_are_mounted_read_only_unless_writable_and_cannot_be_covered`, `a_computer_made_with_other_folders_is_made_again`; `session/tests/preamble.rs` `the_preamble_names_the_granted_folders_and_who_enforces_them` | the shell rows: a Mac, or Linux with bubblewrap |
| Workspace tools refuse parent paths and a path outside the wall; machine reach resolves them; the overflow directory is the one read outside | `hotline-tools/src/workspace/tests.rs` `parent_paths_are_rejected`, `reaching_the_machine_resolves_absolute_paths_and_parents`, `workspace_reach_can_read_the_teammates_overflow_directory`, `machine_reach_ignores_the_overflow_root` | — |
| A revoked workspace handle refuses reads and writes | `hotline-tools/src/workspace/tests.rs` `a_revoked_workspace_handle_refuses_reads_and_writes` | — |
| Confined shell: writes inside, refuses writes and reads outside including from child processes, private `/tmp`, private persistent home, no host environment, a home symlink cannot escape, synthetic parents read-only | `hotline-tools/src/shell/tests.rs` `workspace_reach_*`, `workspace_shell_*`, `a_private_home_symlink_cannot_create_files_outside`, `synthetic_parent_directories_are_read_only` | Linux with bubblewrap |
| Linux runtime exceptions expose executables, not neighbours, credentials or a redirected project | `hotline-tools/src/shell/linux.rs` `arbitrary_path_directories_do_not_grant_read_access`, `a_path_program_does_not_expose_its_neighbors_or_symlink_target_directory`, `home_toolchains_are_read_only_and_do_not_expose_credentials`, `redirected_installations_do_not_expose_other_projects` | Linux |
| macOS: availability requires real enforcement; aliases, symlinks and children cannot reach host data; no host environment or signals; hostile setup symlinks; helper services and control sockets denied; runtime exceptions exclude credentials and neighbours | `hotline-tools/src/shell/macos.rs` `availability_requires_real_enforcement`, `aliases_symlinks_and_children_cannot_access_host_data`, `host_process_environment_and_signals_are_not_available`, `private_scratch_and_hostile_setup_symlinks`, `host_helper_services_and_unix_control_sockets_are_denied`, `home_runtime_exceptions_do_not_include_credentials_or_neighbors`, `redirected_installations_never_grant_the_target` | a real Mac; these fail, not skip, without Seatbelt |
| macOS network stays available and package downloads land in private caches | `hotline-tools/src/shell/macos.rs` `ip_networking_still_works`, `public_dns_and_https_remain_available`, `package_downloads_use_private_caches` | a Mac and public registries; `--ignored` |
| Cancellation reaches the sandbox and everything it started; a deadline takes the whole tree; cancelling during sleep cannot run the next statement (50 repetitions) | `hotline-tools/src/shell/tests.rs` `cancellation_reaches_a_started_sandbox_and_its_children`, `a_deadline_takes_the_command_and_everything_it_started`, `cancellation_during_sleep_never_runs_the_next_statement`; `tests/steering.rs` `an_operator_cancels_a_running_shell_before_its_ninety_second_wait_finishes` | macOS and Linux; sandbox test on Linux needs bubblewrap |
| A new teammate gets no gateway server even with machine reach; grant changes rebuild the live tools; a revoked MCP tool refuses before calling its server | `tests/mcp.rs` `a_new_teammate_gets_no_gateway_tools_even_with_machine_reach`, `gateway_access_is_explicit_and_changes_rebuild_the_live_tools`; `hotline-mcp/src/lib.rs` `a_revoked_mcp_tool_refuses_before_calling_its_server` | — |
| Secrets never enter settings: OAuth client secrets and non-string env values are refused before the write; legacy plaintext is hidden and repaired | `tests/mcp.rs` `oauth_client_secrets_are_removed_before_mcp_settings_are_written`, `a_non_string_env_value_is_rejected_before_settings_are_written`; `tests/desk/logins.rs` `legacy_mcp_secrets_are_hidden_and_repair_removes_the_plaintext_history` | — |
| The ACP OAuth proxy requires its own bearer and a live lease | `driver/acp/tests/oauth_proxy.rs` `oauth_proxy_requires_its_own_bearer_and_a_live_grant` | — |
| Revocation evicts cached logins and retained Copilot transports refuse the next request; ChatGPT delete revokes each stored token once and deletes locally even if the provider fails | `providers/rig_login/tests/revoke.rs` `rig_revoke_copilot_retained_transport_stops_on_its_next_request`, `rig_revoke_chatgpt_forgets_retained_tokens_and_refuses_a_new_lookup`, `rig_revoke_a_refresh_in_flight_cannot_restore_the_login`; `tests/desk/logins.rs` `rig_revoke_delete_calls_provider_once_per_token_even_when_it_fails` | Provider revocation is best effort; already dispatched requests can finish |
| The MCP OAuth callback page is the one thing Hotline serves a browser from its loopback listener, and runs nothing: one escaped card under `default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'` and `Cache-Control: no-store`, with no script, whether sign-in succeeded or failed, and an error it is told is escaped before it is shown | `mcp/tests.rs` `the_callback_pages_say_what_happened_under_a_strict_policy`; `hotline-mcp/src/oauth.rs` `callback_page_escapes_what_it_is_told` | — |
| A stdio server's children die with the connection | `tests/mcp.rs` `a_stdio_servers_own_children_die_with_the_connection` | Unix |
| A reach update reattaches the session and a name patch does not; an MCP settings update reattaches every live session; an unfinished policy update refuses work; stop cannot revive the old generation | `wire/tests/personas.rs` `persona_update_of_reach_reattaches_and_a_name_patch_does_not`; `wire/tests/settings.rs` `settings_update_of_mcp_servers_reattaches_every_live_session`; `session/tests/chapters.rs` `an_unfinished_policy_update_refuses_work_until_reattached`, `stop_revokes_a_replacement_before_its_startup_begins`, `stop_during_policy_quarantine_cannot_revive_the_old_generation`, `reattach_during_a_turn_cancels_the_old_queue_before_rebuilding` | — |
| Collaboration: a machine caller needs no card; reach is re-read after discovery; session consent is directional and expires with either side; a standing grant survives restart and uses the stable id; removing it revokes cached work; a dropped wait cannot be answered later; revocation reaches delegated third parties but not their main sessions | `session/peers/tests.rs` `explicit_whole_machine_hotline_agent_can_collaborate_without_a_card`, `collaboration_rechecks_reach_after_discovery`, `session_consent_is_directional_and_expires_when_a_side_stops`, `permanent_consent_survives_peer_restart_and_uses_stable_sender_id`, `removing_a_permanent_grant_revokes_cached_work_and_requires_consent_again`, `a_dropped_collaboration_wait_is_expired_and_cannot_be_answered_later`, `invalidating_either_side_revokes_cached_peer_tools_without_a_main_session`, `peer_teardown_does_not_revoke_the_callers_main_tools_but_main_stop_does`, `nested_peer_leases_follow_the_outer_target_but_revoke_independently`, `revocation_reaches_a_third_teammates_delegated_tools_but_not_its_main_session` | — |
| The collaboration card is answered over the real wire before the peer starts; the phone seat can answer for the person — a permission card, a `request_human` card, a passkey card — but a companion can never grant a standing one | `wire/tests/door.rs` `the_wire_answers_a_core_owned_collaboration_card_before_peer_start`; `wire/tests/seats.rs` `the_phone_seat_answers_for_the_person_but_never_grants_a_standing_one` | — |
| A linked desk's channel runs a ping and the `federation.*` commands for the desk its key proved, and every other command and subscription is forbidden before it runs; the desk and owner seats cannot speak for a linked desk; only the linked desk's own key opens `/v2/link`, not a stranger's, not the owner device's, not under another desk's id; its frames are at most 1 MiB and read no faster than its budget; a frame racing the link's end never writes its record back | `wire/tests/linked.rs` `a_linked_desk_may_only_publish_its_teammates_and_deliver_messages_through_the_real_handler`; `remote/tests/link.rs` `only_the_linked_desks_own_key_opens_its_link`, `a_linked_desks_frames_are_at_most_a_mebibyte_and_read_no_faster_than_its_budget`; `hotline-room/src/linked.rs` `a_record_written_back_as_its_link_ends_stays_ended` | — |
| A linked sender is never implicitly trusted: the recipient's `allowedSenders` by qualified id or the person's card on the recipient's desk decide it, the sender's desk cannot answer the card, a denial runs nothing, and ending the link fails waiting work with a sentence and withdraws the card; only roster-row fields cross | `session/links/tests.rs` `an_ask_crosses_the_link_waits_for_the_recipients_person_and_its_answer_comes_back`, `a_standing_grant_is_kept_by_the_qualified_id_and_a_handoff_runs_in_the_recipients_work_thread`, `a_denied_linked_sender_hears_so_and_nothing_ran`, `ending_the_link_withdraws_the_card_a_linked_sender_was_waiting_on`, `an_ended_link_fails_what_waited_on_it_and_its_teammates_are_gone`, `a_linked_desk_is_sent_roster_rows_and_nothing_else` | — |
| A colleague sees a teammate's roster state, goal, agent, model and effort, and capabilities as switches, words and granted MCP servers' names; never its workspace path, `mcpPolicy`, a server's address, login header or environment, an ungranted server, `node` or `backendId`; no computer reads `none`; a linked desk's row is cut to the same fields and words | `mcp/server.rs` `a_colleague_sees_a_teammates_goal_agent_model_and_capabilities_and_no_more`, `a_teammate_without_a_computer_or_grants_shows_none_of_them`, `list_teammates_never_carries_a_colleagues_message_text`; `hotline-room/src/linked.rs` `a_published_roster_is_cut_down_to_what_a_roster_row_holds`; `session/links/tests.rs` `a_linked_desk_is_sent_roster_rows_and_nothing_else` | — |
| A teammate withdraws only its own `request_human` card: another teammate's id is refused, a withdrawn card stops reading as waiting for the desk and phone seats and delivers nothing, and an answer to it is refused; a card expires at its lifetime, two hours unless `wait_hours` asks for up to a day, and tells the teammate once; a handoff's waiting result is never released by a withdrawal | `wire/tests/asks.rs` `a_withdrawn_card_is_gone_for_the_desk_and_the_phone`, `a_card_past_its_lifetime_expires_and_tells_the_teammate_once`, `a_longer_wait_is_honoured_up_to_a_day`; `session/exchanges/tests.rs` `a_withdrawn_card_lets_a_running_handoff_finish_and_a_waiting_one_keeps_its_card` | — |
| Background work: scheduling requires the grant, operator jobs do not; a due agent job waits for the grant then fires once; a queued firing is dropped on revocation; an old job on the wire requires the grant; `list_schedules` and `cancel_schedule` stay own-teammate | `mcp/server.rs` `scheduling_requires_background_work_but_operator_jobs_do_not`, `list_schedules_lists_the_callers_jobs`, `cancel_schedule_refuses_another_teammates_job`; `session/tests/scheduled.rs` `a_due_agent_job_waits_for_a_grant_then_fires_once`, `a_due_operator_job_runs_without_a_background_grant`, `a_queued_firing_is_dropped_when_background_work_is_revoked`; `session/schedule.rs` `scheduled_run_authority_reads_the_live_grant_and_trusted_source`; `tests/schedule.rs` `an_old_job_on_the_wire_requires_the_background_grant` | — |
| ACP callbacks stay in the workspace for legacy `machine` personas; a symlinked root alias is accepted; `AGENTS.md` leaves external and dangling symlinks alone, writing through neither; runtime mode is separate from effort and other configs stay hidden | `driver/acp/tests/session_new.rs` `acp_callbacks_stay_in_workspace_for_legacy_machine_personas`, `callback_workspace_accepts_the_selected_symlinked_root_alias`, `agents_md_leaves_external_and_dangling_symlinks_alone`; `driver/acp/tests/config.rs` `disposition_separates_runtime_mode_from_effort_and_hides_other_configs` | — |
| An agent from the ACP registry is unpacked only whole: a link, an entry out of its folder, or a set-id or world-writable mode installs nothing and leaves no scratch, in zip and tar; a checksum mismatch is never unpacked; ordinary archives and bare binaries still install | `driver/acp/install.rs` `a_symlink_out_of_the_scratch_folder_is_refused`, `a_write_through_a_symlink_lands_nowhere`, `an_entry_that_climbs_out_of_its_folder_is_refused`, `a_set_id_or_world_writable_mode_is_refused`, `a_checksum_mismatch_installs_nothing`, `a_zip_without_modes_still_starts`, `a_tarball_is_unpacked_into_place_and_its_command_made_runnable` | — |
| Subagents: a run's lease depends on its teammate's session, so stopping the teammate revokes the run's tools; a run is offered only the reading tools, starts no runs of its own and never reopens the teammate's session; a teammate whose authority is gone starts no run; a cancelled run stops its agent; a run left running by a dead process is settled at startup | `session/tests/runs.rs` `a_runs_tools_read_the_conversation_and_die_with_the_teammates_session`, `a_run_is_on_its_teammates_live_model_with_no_session_to_reopen`, `a_run_for_a_teammate_whose_authority_is_gone_never_starts`, `a_cancelled_run_stops_its_agent_and_its_line_says_so`, `a_run_the_last_process_left_running_is_cancelled_when_the_room_opens`; `session/jobs.rs` `a_cancelled_subagent_ends_cancelled_and_a_fifth_is_refused` | — |
| Clones: a subagent and a job each reach back into the home thread with `search_thread` and `list_chapters` and read the granted folder, and one the home agent made `without` folders cannot read it; a run is not offered the doors to the person; a job's card is raised on its own thread, not the DM, and the answer comes back to it as a turn; a job's thread is found by `search_thread` after the index is rebuilt; a clone is built with the computer unless it was left out, on the model the home session is on, and on its teammate's own harness, ACP included; a run whose home lease is revoked never starts; a job is offered the home thread's tools but `JOB_WITHOUT`, and `schedule` writes what was left out on the job | `tests/schedule/clones.rs` `a_subagent_is_a_clone_that_reaches_back_and_is_narrowed_when_asked`, `a_job_is_a_clone_on_its_own_thread_and_reaches_back_after_a_restart`; `session/tests/runs.rs` `a_run_has_the_computer_unless_the_home_agent_leaves_it_out`, `a_run_is_a_clone_on_its_teammates_own_harness`, `a_run_refuses_a_brief_that_is_a_jobs`; `session/tests/scheduled.rs` `a_jobs_card_is_raised_and_answered_on_its_own_thread`, `a_job_has_the_computer_unless_it_was_made_without_it`; `session/agent.rs` `a_clone_is_narrowed_by_what_the_home_agent_left_out`, `a_clone_is_on_the_model_and_effort_its_home_session_is_on`; `session/tests/posting.rs` `a_jobs_thread_posts_where_a_run_may_not`; `hotline-store/src/search.rs` `a_rebuild_keeps_what_the_teammates_jobs_said`; `tests/threads/together.rs` `every_thread_reaches_the_chat_through_one_door_on_both_drivers`, `a_recurring_job_and_what_came_back_to_jobs_carry_across_a_restart` | Unix for the computer rows |
| `post_to_chat`: a quiet job posts once from its own thread, stamped with the job and from the thread, while a DM turn, a second post and a `to` are refused and land nowhere; `wake` from the DM is refused; a work thread posts once a turn, stamped from the thread, wakes the DM once and is clipped at 2,000 characters; a pair posts to its target's chat, never the asker's; each on Hotline Agent and on an ACP child. A file from a work thread lands in the chat stamped from it and leaves the thread's post; a job's file is its turn's one post, stamped with the job, and a post after it is refused; a file sent on a call is stamped from the call. A run is refused and not offered it, nor `send_file` or `generate_image`, while a job's thread is offered and let through; a revoked lease refuses in process and over the served endpoint; nothing posts between turns | `tests/mcp/post_to_chat.rs` `hotline_agent_dm_may_not_post_and_a_quiet_job_posts_once`, `an_acp_quiet_job_posts_and_a_loud_turn_may_not`, `hotline_agent_work_thread_posts_once_a_turn_and_wakes_its_home`, `an_acp_work_thread_posts_once_a_turn_and_wakes_its_home`, `a_hotline_agent_pair_posts_to_its_own_chat`, `an_acp_pair_posts_to_its_own_chat`, `hotline_agent_work_threads_file_lands_in_the_chat`, `an_acp_work_threads_file_lands_in_the_chat`, `hotline_agent_quiet_jobs_file_is_its_one_post`, `an_acp_quiet_jobs_file_is_its_one_post`; `session/tests/sent_files.rs` `a_jobs_file_is_its_one_post`, `a_file_sent_on_a_call_is_posted_from_the_call`; `session/tests/posting.rs` `a_subagent_run_may_not_post`, `a_jobs_thread_posts_where_a_run_may_not`, `the_dms_own_turn_may_not_post`, `a_revoked_lease_refuses_a_post_in_process_and_over_the_endpoint`, `a_post_between_turns_is_refused`; `tests/schedule.rs` `a_quiet_job_is_heard_only_when_it_posts` | — |
| A permission left open in a peer turn expires with the turn; a receipt cannot move machinery | `session/peers/tests.rs` `a_permission_left_open_in_a_peer_turn_is_expired_when_the_turn_ends`, `a_receipt_cannot_move_machinery` | — |
| The desk restart lease refuses new wire work and keeps saved data | `tests/desk/teammates.rs` `the_desktop_restart_lease_refuses_new_wire_work_and_keeps_saved_data` | — |
| A phone reaches a running computer's viewer only through the desk, with the desk's bearer and never its own copy; the sealed handshake binds the persona and refuses an unpaired device; revoking the device drops the socket | `remote/tests/sealed_network.rs` `sealed_computer_handshake_binds_purpose_and_target_before_upstream`, `sealed_computer_envelopes_hide_the_bearer_and_revoke_with_the_grant`; `remote/tests/computer.rs` `the_computer_target_is_read_off_the_desk_s_own_viewer_and_only_while_running` | — |
| What was brought over is listed from the room's record and taken back by site or whole: the computer is told the exact domains, the record follows, a site never brought over is refused, a release from before the door is named with Update; the record is one entry per teammate and the latest whole list; a companion can neither list nor take back; expired cookies are left on the host | `session/tests/computer.rs` `brought_over_cookies_are_listed_and_taken_back_by_site_or_whole`; `hotline-room/src/room.rs` `the_record_is_the_latest_whole_list_per_teammate`, `an_import_from_the_same_browser_and_profile_merges_and_another_is_listed_beside_it`; `wire/tests/seats.rs` `only_the_desk_seat_may_import_host_cookies`; `computer/cookies.rs` `expired_cookies_are_left_behind_and_session_cookies_stay` | Unix for the first |
| Secret management is operator-only: a companion cannot list, set, delete or arm arbitrary keys, nor grant one through `persona.update`; bounded capture answers are described below | `wire/tests/seats.rs` `only_the_desk_seat_may_touch_stored_secrets` | — |
| A companion reads one living teammate's schedules and nothing it could change them with: no job made, cancelled or quieted, no whole-room list, no room stream (a thread between two teammates and a subagent's run are read like a tape); the entry leaves out who made a job; an unknown teammate or an unreadable room is a refusal, never an empty list; a revoked phone's socket closes and cannot reopen | `wire/tests/seats.rs` `the_phone_seat_reads_a_teammates_schedules_but_changes_none_of_them`; `wire/tests/schedules.rs` `a_teammate_the_room_does_not_hold_or_cannot_read_is_refused_rather_than_empty`; `remote/tests/phone.rs` `a_phone_reads_a_teammates_schedules_and_catches_up_on_what_changed_while_away` | — |
| A companion adds a teammate only through `mobile.persona_create`, never the full `persona.create`: the draft core builds always has workspace reach, this desk's default workspace, no computer and no background work, whatever the phone's JSON names for those fields; `requestId` must parse as a uuid and becomes the id, and a repeated one answers the teammate already made rather than appending a second; a blank name and a `backendId` that is unknown or `backends.list` reports `unavailable` are each refused; `backends.list` itself carries only `id`, `name`, `description` and `unavailable` | `wire/tests/seats.rs` `the_phone_seat_creates_a_teammate_narrowly_but_not_with_persona_create`; `wire/tests/personas.rs` `mobile_persona_create_builds_a_confined_draft_the_phone_could_not_express`, `a_repeated_request_id_does_not_duplicate_the_teammate`, `mobile_persona_create_refuses_a_bad_uuid_a_blank_name_and_a_backend_that_is_not_ready`; `remote/tests/phone.rs` `a_phone_creates_a_confined_teammate_but_not_through_persona_create` | — |
| A companion renames a teammate or changes its goal only through `mobile.persona_update`, never `persona.update`: the patch core builds holds `name` and `goal` and nothing else, whatever the phone's JSON names for reach, path, computer or background work; a blank name, an edit naming neither field and an unknown teammate are refused; the phone may `persona.delete` | `wire/tests/seats.rs` `the_phone_seat_renames_and_deletes_a_teammate_but_never_patches_one`, `mobile_persona_update_changes_only_the_name_and_goal`; `remote/tests/phone.rs` `a_phone_renames_and_deletes_a_teammate_but_not_through_persona_update` | — |
| A relay grant stands in on a served desk's relay and accepts its visits, and opens no other door: `/v2`, `/v2/computer/…` and the server's own `/v2/link` answer it sealed `role_not_permitted`, never `device_not_authorized`, on a server that runs agents (a relay-only one has no route there); a phone's claim on a relay invitation and a relay claim on an owner or companion invitation make no grant, keep the window open and count toward the throttle; relay grants are saved in `relayGrants`, never `grants`, so a reader from before the role loads the file and authenticates none of them; only a served desk invites a relay | `remote/tests/relay.rs` `a_relay_grant_stands_in_and_carries_a_visit_and_opens_no_desk_door`, `a_claim_whose_purpose_does_not_match_the_window_makes_no_grant`, `a_reader_from_before_the_relay_role_loads_relay_grants_and_trusts_none`, `only_a_served_desk_invites_a_relay`; `tests/pair.rs` `a_relay_only_server_refuses_a_phones_claim_and_keeps_its_window` | — |
| An owner device moves its own grant to a relay grant in place (same id, key, name, pairing time) and the move ignores any device it names; the device is then refused `/v2` with `role_not_permitted`, cannot move again, and stands in again so a phone still reaches its desk; a desktop pairs with a relay's invitation only as a relay, kept beside the desks and not listed as one | `remote/tests/relay.rs` `an_owner_moves_its_own_grant_to_relay_in_place_and_keeps_standing_in`, `a_relay_grant_stands_in_and_carries_a_visit_and_opens_no_desk_door`; `desks.rs` `a_relay_is_kept_beside_the_desks_and_never_listed_as_one`; `wire/tests/seats.rs` `remote_controls_are_denied_to_companions`; `ui/tests/relay-picker.test.tsx` `a relay-only server is never a desk in the window` and the offer's `never shows when …` cases; `ui/tests/add-desk.test.ts` `a relay link says it added a relay, chooses it when none is chosen, and never lists a desk` | — |
| A server answers a door only when it runs that door's capability: relay-only has no `/v2`, `/v2/computer/…` or `/v2/link` (404 before Noise), pairs relay grants only, opens no room and still carries a visit; without relay, `/v2/relay`, `/v2/relay/accept` and visitor routes are 404 and no relay is invited; switching modes keeps every grant and its role, and a relay grant made on one boot stands in on the next | `remote/tests/relay.rs` `a_relay_only_server_stands_a_desktop_in_and_opens_no_desk_door`, `a_server_that_runs_no_relay_has_no_relay_door`; `tests/serve.rs` `a_relay_only_server_opens_no_room_and_carries_a_desktops_visitors`, `serving_in_another_mode_keeps_every_pairing_and_its_role` | — |
| End to end with real processes: a relay-only server opens no room and starts no process; a desktop pairs from `hotline pair --link` as a relay and stands in, and a phone paired with the desktop reaches it only through the relay; a phone-shaped claim makes no grant; served again with agents, the same grant stands in without re-pairing and its key, knocking with a hand-written handshake, is answered `role_not_permitted` at `/v2`, `/v2/computer/…` and `/v2/link` | `tests/relay.rs` `a_relay_only_server_relays_for_a_desktop_that_can_do_nothing_else_there` | — |
| A teammate sends the person only what its own read reach opens and what its own computer holds, under the session's lease; nothing is sent past the caps or by a subagent run, a job's file takes the turn's one post, and a refusal leaves no message and no kept copy; `file.read` names a message and never a path, serves only that message's one kept file for teammate messages, and refuses a path-like id, an unknown teammate and an offset outside the file; the phone may read it; the shell opens only a kept copy, and only a PDF or a picture whose name and first bytes agree, so a script that starts like a PDF is never run | `session/tests/sent_files.rs` `a_picture_reaches_the_person_as_a_jpeg_with_its_caption`, `what_cannot_be_sent_is_refused_and_leaves_nothing_behind`, `a_jobs_file_is_its_one_post`, `a_file_and_the_screen_come_off_the_computer`; `sent.rs` `a_kept_file_reads_back_a_part_at_a_time_and_only_by_its_message`; `wire/tests/files.rs` `a_sent_file_is_read_by_its_message_a_part_at_a_time`, `the_phone_seat_reads_a_sent_file_by_its_message`; `sent.rs` `anything_else_is_kept_as_it_is_and_told_by_its_bytes_before_its_name`; `hotline-app` `files.rs` `only_a_kept_copy_is_touched`, `only_a_pdf_or_a_picture_opens_and_only_when_name_and_bytes_agree` | Unix for the computer row |
| User image readback uses a send-time copy at the original attachment index; default zero, mixed attachments, chunk continuation, real MIME and no transcoding; source replacement/deletion cannot change it, missing/legacy copies never reopen paths; non-image, invalid index/offset, wrong teammate/message, planted file link and over-cap copy are refused; a paired phone can refetch its image but not a user text file | `tests/desk/teammates.rs` `user_images_read_back_by_original_index_without_reopening_the_source`; `sent.rs` `user_image_copies_keep_their_bytes_and_cannot_be_replaced`; `remote/tests/phone.rs` `a_phone_sees_the_real_roster_and_tape_but_cannot_administer_the_desk`; `wire/tests/files.rs` `a_sent_file_is_read_by_its_message_a_part_at_a_time` | Unix for planted link |
| A secret's name is an environment variable, never Hotline's own or the shell's; a value is at least eight characters; the disk holds a reference and the room stream nothing; the directory and its records are private and a planted link is refused | `vault/shared/tests.rs` `a_name_is_an_environment_variable_and_hotlines_own_are_refused`, `a_value_is_at_least_eight_characters`, `a_shared_secret_is_listed_by_name_and_never_by_value`, `the_shared_directory_and_its_records_are_private_and_a_planted_link_is_not_a_secret` | Unix for the last |
| A login needs an `https://` site of its own (or `http://` on localhost) and a password of eight characters; a passkey needs a host name and a key; a login and a passkey are listed by what they are for and never by password, seed or key, and the sidecar carries none either | `vault/shared/tests.rs` `a_login_needs_a_site_of_its_own_and_a_passkey_a_key`, `a_login_and_a_passkey_are_listed_by_what_they_are_for_and_never_by_value` | — |
| A computer is handed only what its teammate is granted and what is stored; the tape names what is not; the preamble names what the computer has, by kind; no tape, room event or preamble carries a value; a replaced or deleted value reaches every running computer and no stopped one; a release from before secrets is told apart, and the conversation says nothing; a login travels as one record beside a variable's bare value | `session/tests/computer.rs` `a_computers_granted_secrets_are_handed_to_it_at_start_by_name_and_never_seen`, `a_changed_secret_is_handed_again_to_every_running_computer`, `a_computer_from_before_secrets_leaves_the_conversation_alone`, `a_login_is_handed_to_the_computer_as_a_record_and_named_by_its_sites` | Unix |
| A passkey is made only under an arming for one teammate and one site, and only once the person approves the site's request: a bad site or name is refused before the computer is touched, arming starts the computer, the site's request raises a `passkey_ask` card on the tape with the site, origin and account, nothing is stored while the card waits, an answer must name the request, one answer per request, an approval lets the browser mint and the look that finds it made stores it, ticks it for the teammate, hands the computer the set and ends the arming; a denial ends the arming with nothing stored and the card says so; a request that left with its page expires its card and the next request is a new card; a cancel stores nothing and expires a waiting card; the room's own watch does all of this with no pane polling, and the pane is told once when it next asks; the private key is on no tape and in no room event; a release from before passkeys is named | `session/tests/computer.rs` `a_passkey_is_made_under_an_arming_stored_and_ticked_for_the_teammate`, `a_passkey_made_while_no_pane_is_looking_is_stored_by_the_room`, `a_passkey_request_denied_on_the_tape_ends_the_arming_and_a_lost_one_expires` | Unix |
| Delivery puts the bearer in a header, replaces the whole set, and tells an old release apart from a refusal; the arming is put, polled, answered and ended over the bearer door, the request's site, origin and account are read as the computer answers them, an answer to a request that is not waiting is a refusal that says so, a bad site is a 400, a release before the answer door mints without asking and refuses an answer as too old, and an old release has no door | `computer/secrets.rs` `the_set_is_put_whole_with_the_bearer_in_a_header`, `a_release_from_before_secrets_is_told_apart_from_a_failure`; `computer/passkeys.rs` `an_arming_is_put_polled_answered_and_ended_over_the_bearer_door` | — |
| Inside the computer: `/secrets` wants the bearer and has no GET, every job sees the variables under the agent's own `env`, a login is typed only on its own sites and refused elsewhere, the TOTP digits are computed from the seed, every tool answer has the values redacted; a passkey is minted only while armed, only for the armed site, and only under a request the person approved — the site's request is parked and answered as `asked` with what the site asked for, a denial rejects the site's call and ends the arming, an answer to a request that is not waiting is a 409, one request is before the person at a time — a credential outside an approved request is removed on the next look, a granted one signs a site's challenge, and a revoked one is gone | Hotline Computer's `src/secrets.rs`, `src/passkeys.rs` and `src/browser.rs` tests and `tests/contract.rs`, run by that repository's `make check` and `make contract` | the computer repository |

Where real macOS execution is required: every `hotline-tools/src/shell/macos.rs` row.
`make check` in macOS CI on both architectures runs the isolation and
toolchain tests; the network rows are `--ignored` and run by hand. BRO-14
recorded macOS 26.4.1 on Apple Silicon locally and macOS 15 in CI; macOS
13 and 14 are untested, and the app's minimum version is not evidence
about this policy. A release that changes the Seatbelt profile or the
bubblewrap launcher reruns the platform rows on each supported version.

## The review rule

A pull request that adds or changes a capability states, in its
description, in this order: the **default** for a new record and the
meaning of a missing field on an old one; the **grant source** (which
field, set by whom, over which wire command); the **enforcement points**
(the handlers that call `check()` or read the policy, on both drivers);
the **tests** added to the matrix above; and the **residual risk** it
leaves, in a sentence a reader without context can act on.

The UI copy, the preamble, and the ledger reason are part of the change,
not follow-ups: if the code confines a thing, they say so; if a grantee
enforces its own permissions, they say that instead. Planned protection is
described as planned, in the board or in Linear, never in a settings card
or in this document's present tense. When a row in the matrix stops being
true, this document is wrong, and fixing it is part of the change that
made it so.

## Ask and handoff collaboration (BRO-123)

- **Default and old records:** `intent` defaults to Ask. No standing sender
  grant is the default. Missing informed-consent records on legacy grants do
  not authorize Handoff. Old `link` records grant nothing and are ignored.
- **Grant source:** the existing directional collaboration permission card,
  answered by the person over `permission.answer`, explains main-context
  handoffs as well as workspace/tool delegation. Session consent is bound to
  both live leases; a standing approval records the sender's stable id and an
  informed-consent room record. Phone seats may answer session consent but
  not standing consent. Keep going and Stop exchange are person-seat commands,
  not teammate tools, and do not grant collaboration authority.
- **Enforcement:** `session/exchanges.rs` persists acceptance and checks live
  leases and `authorize_collaboration` before starting either intent. Handoff
  uses the recipient's existing main session and permissions; Ask uses its
  isolated peer session. `invalidate`, stop and grant revocation settle queued
  work; turn dispatch rejects stopped handoffs/results. A cap counts requests
  and automatic replies together, never receipts, and survives restart.
- **Tests:** `session/exchanges/tests.rs` covers default Ask isolation, main
  context Handoff and automatic request correlation, cross-intent counting,
  a held twelfth-message reply, paused restart, started-work crash settlement,
  Stop/revocation and legacy informed approval. `wire/tests.rs` covers both
  person seats' resume/stop authority and existing permission-card handling.
- **Residual risk:** a recipient may use every permission it already has and
  disclose context in its answer. A crash after work starts but before its
  result is saved produces an explicit uncertain failure, not replayed work;
  inspect side effects before retrying. Stopping a handoff cancels its active
  turn but cannot undo completed effects.


### Human-gated handoff continuation (BRO-128)

- **Default / old records:** a request's missing `human_actions` means no linked
  human gate. Existing tape/card shapes are unchanged. Only a successfully
  completed turn with linked unanswered actions enters `waitinghuman`; a card
  alone never makes an interrupted turn safe to replay.
- **Grant source:** unchanged directional collaboration consent. A human answer
  is not a grant; session consent does not survive restart.
- **Enforcement:** `ask_human` durably links the card to the active handoff;
  `resume_handoff_answer` checks scoped leases and `authorize_collaboration`
  before recording continuation start. Turn admission refuses stopped,
  revoked, already-consumed or uncertain continuations. No tools or reach are
  added, and both drivers use the same session funnel.
- **Tests:** `session/exchanges/tests.rs`
  `a_human_gated_handoff_routes_its_final_result_after_restart_for_done_and_declined`,
  `a_saved_human_answer_recovers_the_gap_before_delivery`,
  `stopped_or_revoked_human_handoffs_cannot_resume_after_restart`,
  `a_human_answer_does_not_replace_collaboration_consent_after_restart`, and
  `restart_does_not_replay_an_interrupted_human_answer_turn` exercise the real
  tool/answer handlers and scripted drivers.
- **Residual risk:** starting a continuation before a crash can have effects
  that cannot be undone; the caller receives an uncertain failure and must
  inspect those effects before retrying. Pre-fix requests have no saved gate
  association and cannot have one safely reconstructed.

## What a colleague sees

`list_teammates` tells a teammate who its colleagues are and what each can
do, so it can ask the right one. This is disclosure, not reach: nothing in
it lets a colleague use another's tools.

- **Default and old records:** every teammate's row carries its goal, its
  agent by name, its model and effort when set, and its capabilities —
  computer (`none`, `attached`, `in use`), web search, image generation,
  background work, `messageTeammates` (`directly` or `after approval`),
  reach (`workspace` or `whole machine`) and the names of its granted MCP
  servers. A linked desk from before the profile sends none of it, and its
  rows read as the narrowest words.
- **Grant source:** none to give: the person's own choices for a teammate
  are what the row reports, read from its record, the room's settings and
  its live session at each call.
- **Enforcement points:** the row is the type `contract::LinkedTeammate`,
  built only by `mcp::server::roster_row` and `profile`, which take names,
  switches and words and never a path, a server's transport or a policy's
  ids; `linked::plausible` cuts what a linked desk publishes to the same
  fields, known words and bounded plain text.
- **Tests:** the [regression matrix](#the-regression-matrix) row for a
  colleague's view.
- **Residual risk:** a secret the person types into a goal, or names an MCP
  server after, reaches every colleague, and through a link every teammate
  on the linked desk; so does the model id. A compromised linked desk can
  claim any capability for its teammates; it gains nothing by it, because
  the words decide no grant here.

## Linked desks (BRO-177, ADR 0003)

- **Default and old records:** no desk is linked. A `desk_link` record exists
  only once the person links two desks; nothing older (the previous edition's
  fleet, an owner pairing, an old `link` record) is read as one. A teammate on
  a linked desk starts with no standing grant on any teammate here.
- **Grant source:** `desks.link` from Settings › Desks, on the desk that makes
  the link, which asks the other desk's `desks.link_accept` over the owner
  seat the person already holds there. Both are owner or local desk commands.
  The grant is the other desk's own Noise key, kept on the dialled desk's
  record; it is not a device grant and an owner pairing does not open it.
  What a linked sender may then ask of a teammate here is the ordinary
  collaboration grant: `allowedSenders` by its qualified id
  (`deskId/personaId`), or the person's first-contact card.
- **Enforcement points:** `remote/link.rs` `answer` admits `/v2/link` only for
  a key a link record names, under the desk id bound in the authenticated
  Noise payload, and under the existing admission limits; any other key gets
  the authenticated refusal. Every frame on the channel is answered by
  `wire::federation` under `Seat::Link`, through `Seat::permits`: a ping and
  `federation.roster`, `federation.deliver` and `federation.unlink`, for the
  desk the channel proved; every other command and every subscription is
  `forbidden`, and the desk and owner seats may not send a federation
  command. A published roster is cut to `LinkedTeammate`'s fields
  (`linked::plausible`). A delivered request is checked for size and a known
  recipient (`session/links.rs`), queued in `exchange_pair`, and decided by
  `authorize_collaboration` like a local one — except that a linked caller
  is never implicitly trusted (`caller.node.is_none()`), and its card is on
  the recipient's tape. The sender's desk neither holds nor answers it.
  Both drivers reach the same queue. The channel reads and sends frames of
  at most 1 MiB, as a phone's wire does, and reads a linked desk's frames no
  faster than its budget (`linked::Links::frame_wait`). A link's record is
  read and written back (`linked::update`) under the lock its removal takes,
  so a roster frame that lands as the link ends never writes it back.
- **Revocation:** `desks.unlink` from either side, or the other desk's
  authenticated refusal at the next dial, runs `Room::unlinked`: the record
  goes, the channel and dialing stop, waiting requests to or from that desk
  fail with a sentence, every capability epoch of that desk's teammates is
  invalidated — which withdraws their pending cards, ends session grants and
  peer sessions and stops their handoffs — and they leave `list_teammates`.
  The standing grants the person gave them here end too: their qualified ids
  leave every `allowedSenders` and their informed consent is withdrawn, so a
  link made again starts from nothing.
- **Tests:** `wire/tests.rs`
  `a_linked_desk_may_only_publish_its_teammates_and_deliver_messages_through_the_real_handler`;
  `remote/tests/link.rs` `only_the_linked_desks_own_key_opens_its_link`,
  `a_link_made_over_the_owner_seat_carries_rosters_and_messages_both_ways`,
  `a_link_outlasts_either_desk_going_away_and_ends_on_both_from_either_side`,
  `a_desk_reachable_only_through_its_relay_is_linked_through_it`,
  `a_linked_desks_frames_are_at_most_a_mebibyte_and_read_no_faster_than_its_budget`;
  `session/links/tests.rs` (cards on the recipient's desk, standing grants
  by qualified id, a denied linked sender, offline queueing both ways, an
  ended link failing and withdrawing what waited, ambiguous names, and the
  roster carrying no field but a roster row's); the tests of `linked.rs` in
  `hotline-room`, among them `a_record_written_back_as_its_link_ends_stays_ended` and
  `a_linked_desk_may_send_a_burst_and_then_only_so_many_frames_a_second`.
- **Residual risk:** a compromised linked desk can publish any roster and
  send any message under any of its teammates' names; each still meets the
  recipient's grants and the person's card, and is quoted as a colleague's
  data; its names and roster text are made plain lines first
  (`linked::plain`), because a name stands outside the quote. It can fill
  the person's view with first-contact cards, but only from the at most 200
  teammates it publishes, with at most 48 of its messages waiting here at
  once, and this desk writes its names down at most twice a minute whatever
  it sends. Each of its frames is at most 1 MiB; it may send 200 at once and
  20 a second after that, and past that it is read more slowly rather than
  refused, the budget kept per link so dialing again does not refill it. A
  roster whose names have not moved reads nothing from disk, but each
  delivered message still reads the room stream, so the frame budget is
  what bounds that work. Ending the link stops all of it. A recipient answering it uses its own reach and tools and may
  disclose what it would to a local colleague. Ending a link from the
  dialing desk while the other cannot be reached leaves the other holding a
  link that never dials again, shown offline, until it is ended there too.
  Stopping an exchange on the sender's desk does not stop work already
  running for it on the other.

## Add a teammate from the phone (BRO-127)

- **Default and old records:** a persona `mobile.persona_create` writes carries
  no `reach` (workspace), `cwd` under this desk's own workspaces directory, no
  `computer`, and `backgroundWork: false` — the same defaults `persona.create`
  writes for a draft that names none of them, because the phone's params
  cannot name them at all. There is no legacy record: this is a new command,
  not a new field.
- **Grant source:** the phone seat's allowlist in `Seat::permits`
  (`wire/seat.rs`) now names `Command::MobilePersonaCreate` and
  `Command::BackendsList`. `mobile.persona_create` is confined in core —
  `wire/commands.rs`'s `mobile_persona_create` builds a `PersonaDraft` that
  never sets `reach`, `cwd` or `computer` — rather than by the phone's form
  leaving fields out, per BRO-117 §7. `backends.list` is unchanged and
  read-only; it was already desk-only by omission, and is now on the
  allowlist too.
- **Enforcement:** `mobile_persona_create` parses `requestId` as a uuid before
  anything else, refusing an unparseable one; looks it up in `room::roster`
  and returns the living persona unchanged when it already exists, so a retry
  never appends twice; refuses a blank name; and resolves `backendId` (or the
  room's default) against `room.backends().await`, refusing an id absent from
  that list or carrying `unavailable`. The write itself is the same
  `build_persona` helper `persona.create` calls, given the phone's id instead
  of a fresh one.
- **Tests:** `wire/tests/seats.rs` `the_phone_seat_creates_a_teammate_narrowly_but_not_with_persona_create`
  proves the allowlist; `mobile_persona_create_builds_a_confined_draft_the_phone_could_not_express`
  proves the written persona has no reach, no computer, no background work and
  the default workspace even when the params carry `reach`, `cwd` and
  `computer`; `a_repeated_request_id_does_not_duplicate_the_teammate` and
  `mobile_persona_create_refuses_a_bad_uuid_a_blank_name_and_a_backend_that_is_not_ready`
  prove idempotency and the four refusals. `remote/tests/phone.rs`
  `a_phone_creates_a_confined_teammate_but_not_through_persona_create` proves
  the same through a real paired phone over TLS, that `backends.list` carries
  only `id`, `name`, `description` and `unavailable`, and that `persona.create`
  still answers `forbidden` for that seat.
- **Residual risk:** any paired phone can create a teammate today, the same
  posture retained by the companion seat. A created teammate's workspace is a fresh empty
  directory the desk made, so this adds no reach beyond what an operator
  clicking "New teammate" already grants; it only lets the phone ask for it.

## Try a failed turn again, from the desk or a phone

- **Default and old records:** nothing new is stored. `session.retry`
  queues the line already on the tape as a retry and writes no new message.
- **Grant source:** the local desk and owners, as every command; the phone
  seat's allowlist in `Seat::permits` (`wire/seat.rs`) names
  `Command::SessionRetry`, so a companion may retry too, and the hello lists
  `turnRetry`. It can only resend a line the tape already holds, which any
  paired phone could have sent.
- **Enforcement:** `Room::retry` refuses unless the tape's last turn failed
  and no message came after it, so an old failure cannot be replayed, and it
  holds the same work lease and chapter as a prompt. The driver runs the turn
  with the reach the teammate has now, not the reach it had when it failed.
- **Tests:** `session/tests/cards.rs`
  `a_failed_turn_is_retried_on_the_same_line_without_writing_it_again`,
  `driver/rig/tests.rs`
  `a_failed_turn_keeps_the_users_line_so_a_retry_still_has_the_question`, and
  `wire/tests/seats.rs` `the_phone_seat_may_try_a_failed_turn_again`.
- **Residual risk:** a failed turn may already have run tools before it
  failed; a retry can run them again. The card says what failed and the
  person chooses to retry.

## Edit or remove a teammate from the phone (BRO-117)

- **Default and old records:** nothing new is stored. `mobile.persona_update`
  writes the same persona record `persona.update` does, with only `name` and
  `goal` changed; `persona.delete` writes the same tombstone.
- **Grant source:** the phone seat's allowlist in `Seat::permits`
  (`wire/seat.rs`) now names `Command::MobilePersonaUpdate` and
  `Command::PersonaDelete`, and the hello lists `personaEdit`.
  `mobile.persona_update` is confined by its shape: the command has fields
  for a name and a goal and nowhere to carry anything else, and
  `wire/commands.rs`'s `mobile_persona_patch` builds the patch from those two
  alone. `persona.update` is available to owners and the local desk, never companions.
- **Enforcement:** the patch goes through the same `apply_persona_update`
  path as `persona.update`, under the policy gate; a changed goal restarts a
  live session exactly as it does from the desk. A blank name and an edit
  naming neither field are refused before anything is written.
- **Tests:** `wire/tests/seats.rs` `the_phone_seat_renames_and_deletes_a_teammate_but_never_patches_one`
  proves the allowlist; `mobile_persona_update_changes_only_the_name_and_goal`
  proves reach, path, computer and background work are untouched even when
  the params carry them, and the refusals. `remote/tests/phone.rs`
  `a_phone_renames_and_deletes_a_teammate_but_not_through_persona_update`
  proves the same over a real paired phone, including the delete.
- **Residual risk:** any paired phone, including a companion, can remove a teammate. Removal takes no reach and grants nothing,
  and the tape is kept, but it is destructive; the phone asks the person to
  confirm first.

### Harness sign-in is a private operator operation (BRO-132)

- **Default and grant source:** choosing an ACP harness already trusts that
  external program with its host permissions. Sign-in adds no agent tool or
  standing grant. An authenticated owner or local desk seat may invoke
  `agent.auth.*`; companions remain excluded.
  Poll, input, and cancel also require the exact desktop socket that started
  the attempt; knowing its id does not let another desktop read or drive it.
- **Enforcement:** the driver resolves a method id against the current
  initialized harness, not the failure card's saved descriptor. Terminal
  authentication is offered on Linux and macOS only and uses the captured
  executable with its configured arguments
  plus the advertised suffix and environment; no client-supplied executable,
  arguments, environment or shell string is accepted. Agent authentication
  calls ACP `authenticate`. Login does not advertise ACP terminal tool access.
- **Private data:** the card records only harness/method labels and ids.
  Terminal input and bounded, drained output use dedicated commands, not a
  draft, transcript event, subscription, model prompt, or provider vault.
  Authentication errors use fixed messages instead of raw harness responses.
  The harness owns its credential storage; Hotline does not extract or relay
  Claude session tokens. The terminal only follows HTTP(S) links after a click.
- **Lifetime:** one attempt per teammate. Its session lease, explicit cancel,
  owning desktop disconnect, and five-minute timeout end the attempt. A
  successful sign-in opens a fresh session but never resends the failed turn;
  the person may review its message in an empty composer and choose to send.
- **Tests:** `driver::acp::auth::tests` exercises signed-out startup, safe
  descriptors, success without replay, sanitized failures, cancellation,
  disconnect, revocation, and cancellable replacement initialization.
  `driver::auth::tests` exercises bounded output and real PTY success/failure
  and cleanup, including descendants that keep a slave open in another group.
  `wire::tests::auth_wire_allows_only_desktop_and_disconnect_revokes_its_owner`
  proves companion refusal, socket ownership, no tape writes, and disconnect.
  UI tests cover failure-card parsing and explicit retry selection; browser
  fixtures additionally exercise terminal lifecycle and private input.
- **Residual risk:** a trusted external harness can print sensitive material,
  open its own browser, write its own logs, spawn processes, and retain its
  own credentials. This is not a sandbox around the harness. The authenticated
  desktop and local process memory are trusted; clearing transient buffers is
  not a cryptographic memory-erasure guarantee. Terminal sign-in can leave an
  account authorized even if a later session restart fails. The phone flow is
  deferred because Claude's remote method is a TUI, not a structured device-code
  interface; no unsupported terminal parser or alternate token path is used.


### An agent installed from the ACP registry

- **Default:** an agent the ACP registry ships as a prebuilt archive is
  downloaded only when a session starts it and its command is not on disk
  yet (`driver::acp::registry::install`). It is unpacked into a scratch
  folder beside its install folder and renamed into place, so an install
  that exists is whole.
- **Grant source:** the person's choice of that ACP backend for a teammate.
  The archive's URL and optional SHA-256 come from the registry; when the
  registry publishes a digest, a download that does not match it is never
  unpacked.
- **Enforcement:** in `driver::acp::install::unpack`, outside the model, for
  zip and tar alike. Every entry is admitted before it is written: a link of
  any kind, an entry whose path leaves the folder, and a set-id or
  world-writable mode are each refused, and the refusal drops the scratch
  folder with whatever was already written into it. The zip reader is
  version 9, the first with RUSTSEC-2026-0367..0369 fixed, and it clears
  set-id bits itself. The command must then be a regular file inside the
  install folder.
- **Tests:** `driver::acp::install::tests` refuses
  `a_symlink_out_of_the_scratch_folder_is_refused`,
  `a_write_through_a_symlink_lands_nowhere`,
  `an_entry_that_climbs_out_of_its_folder_is_refused` and
  `a_set_id_or_world_writable_mode_is_refused`, each installing nothing and
  leaving no scratch, and still installs
  `a_tarball_is_unpacked_into_place_and_its_command_made_runnable`,
  `a_zip_without_modes_still_starts`,
  `a_bare_binary_is_put_where_the_command_says` and
  `an_archive_is_known_by_its_extension_in_any_case`.
  `a_checksum_mismatch_installs_nothing` proves the digest.
- **Revocability:** an install is a file on disk, not a lease. Choosing
  another backend removes the reason to run it, and a new release removes the
  older ones beside it.
- **Residual risk:** with no published digest the archive is trusted as
  served. An archive that needs a link inside itself does not install. The
  installed agent runs with the host's permissions, the accepted risk of any
  ACP harness. The updater's and the speech engine build's copies of ZIP are
  older versions, behind their own signature and pinned hash.

## The owner phone changes a teammate's access (BRO-140, part)

- **Default and old records:** nothing new is stored. `mobile.persona_access`
  writes the same `reach`, `backgroundWork` and `modeId` fields the desk
  writes, through the same persona record. `session.set_mode` now also keeps
  the chosen `modeId`, so a harness teammate comes back in it after a restart
  instead of on the harness default.
- **Grant source:** `Seat::permits` (`wire/seat.rs`) lets `Seat::Owner`, and
  not `Seat::Phone`, send `Command::MobilePersonaAccess` .
  The command has fields for a reach, a mode and background work and nowhere
  to carry a path, a server, a skill or a computer; `persona.update` and
  `session.set_mode` are also available to owners; companions cannot call them. The owner's
  hello adds `personaAccess`; a companion's does not.
- **Enforcement:** `wire/commands.rs` `mobile_persona_access` refuses a reach
  for a harness teammate and a mode for a Hotline Agent teammate, and an
  empty change. Reach and background work go through `apply_persona_update`
  under the policy gate, so a live session is invalidated and reattached and
  its tool handles are revoked exactly as from the desk. A mode is checked
  against the modes a running teammate advertises and switched live; a
  resting teammate's choice is kept and offered to the harness at its next
  start, which refuses one it does not have.
- **Tests:** `wire/tests/seats.rs` `only_the_owner_phone_changes_a_teammates_access`
  proves the owner/companion split, companion refusal of
  `persona.update` and `session.set_mode`, and the hello difference;
  `mobile_persona_access_sets_reach_mode_and_background_work` proves the
  writes and every refusal. `remote/tests/phone.rs`
  `a_phone_reads_a_teammates_schedules_and_catches_up_on_what_changed_while_away`
  proves a companion grant's hello over a real phone socket.
- **Residual risk:** an owner phone can now give a Hotline Agent teammate the
  whole machine, or put a harness in its most permissive mode (for example
  Claude's `bypassPermissions`), from anywhere the phone reaches the desk.
  That is the owner's call by design; the phone explains each choice before
  it is made, and a companion phone cannot make it.


## The owner phone changes a teammate's computer (BRO-140)

- **Default and old records:** `computer.cpus` is optional. Absent means no
  CPU cap, as before; the memory default remains `4g`. Docker and Podman
  accept fractional CPU limits; Apple container rounds a requested cap up
  to a whole CPU, with a minimum of one.
- **Grant source:** `mobile.persona_computer {id, enabled?, memory?, cpus?}`
  is owner-phone only, advertised by the owner's `personaComputer` hello
  capability. A companion cannot send it. Owners also have general
  `persona.update`, including image, process-limit, mount and secret controls;
  companions do not.
  Omitted fields stay unchanged; `cpus: null` clears the CPU cap. Enabling
  a teammate with no computer record creates one with the normal defaults.
- **Enforcement:** CPU caps are positive half-CPU increments no larger than
  the reported capacity. Memory is at least 512 MiB, in 512 MiB increments,
  no larger than the reported memory; the phone sends whole megabytes such
  as `4608m`. An empty change is refused. Updates use the desk's existing
  persona-update path and preserve image, process limit, mounts and secrets.
  Enabling or disabling follows the same session reattachment rules as the
  desk. Limit changes do not interrupt a turn: a container records the
  limits it was made with (`HOTLINE_LIMITS`) and the next start makes it
  again when they differ. `computer.restart` brings that forward from the
  desk, between turns; the phone can stop a computer but not ask for it.
- **Read-only capacity:** `computer.capacity {}` is available to the desk
  and both phone seats. It returns `runtime`, `cpus`, `memoryBytes` and
  `source`, cached for about a minute. Docker and Podman report their own
  capacity; Apple container or a failed runtime query falls back to host
  capacity, then to four CPUs and 8 GiB if host capacity is unavailable.
  `runtime: null` means no installed runtime was found. This reports totals,
  not currently free resources or a reservation for the teammate.
- **Residual risk:** an owner can enable a teammate's computer remotely and
  remove its CPU cap. Limits are per computer, not a shared budget: several
  teammates can collectively exhaust the runtime's capacity. Host/default
  fallback is an estimate when the runtime cannot report its own limits.
  These container controls impose no resource limit on host shell tools or
  an external harness.

## Sealed connections and explicit pairing (BRO-114, BRO-218)

- **Default and grant source:** desktop Remote remains opt-in. Both desktop
  and served listeners accept only sealed v2 pairing and sessions. Old grants
  without a device public key stay listed and revocable, but cannot
  authenticate or receive push notifications; the window says **Needs re-pair**.
  Phones paired with the six-digit code before 0.33 scan the new QR once.
  `serve` requires an explicit non-wildcard fixed
  listen address and public HTTPS URL; it starts that listener and never
  widens or chooses an ephemeral fallback. An owner or local desk operator opens
  a two-minute single-use QR invitation. Empty grants never bootstrap an
  owner. Old grants without roles become owner. Owners have the desk command
  set; companions retain the limited phone allowlist.
- **Enforcement:** `remote/v2.rs` persists a separate X25519 identity in the
  selected SecretStore and atomically consumes the invitation with grant
  creation. `remote/server.rs`, `sealed.rs` and `channel.rs` authenticate
  Noise IK before any hello or application frame. A valid handshake with an
  unknown or revoked device key receives an encrypted refusal; invalid
  handshakes close silently. Viewer purpose and persona are authenticated handshake content,
  checked against the HTTP target, so a TLS proxy cannot redirect controls.
  Only the desk resolves computer addresses and presents viewer bearers.
  `Seat::permits` refuses remote administration to companions.
- **Lifetime:** explicit revoke or Remote disable invalidates active sockets,
  even while a remote command is waiting on a driver or computer. Pending
  remote calls are dropped and subscriptions/writers are cleaned up; this is
  not rollback of already dispatched effects or persisted choices. Local
  desk mutations still finish when their socket disconnects.
  The wire cancels only its own invitation on disconnect; CLI Ctrl-C, process
  death and SSH disconnect close that window. There is no six-digit flow or
  legacy bearer pairing, room or computer route on either listener.
- **Tests:** `remote::sealed::tests` and `tests/fixtures/noise_v2.json` cover
  framing and fixed-key interoperability. `remote::tests::sealed_network`
  covers real TLS/Noise, replay/tamper/plaintext rejection, atomic pairing,
  roles, certificate rotation, viewer binding and revocation. Other Remote
  tests cover supplied TLS, exact late binding, absent legacy routes and
  handshake budgets.
  `remote::tests::legacy::desktop_legacy_routes_are_absent_and_old_grants_only_remain_for_revocation`
  proves the desktop has no v1 routes and token-only records have no auth or
  push authority while remaining revocable.
  `remote::tests::legacy::old_grants_do_not_consume_the_sealed_device_limit` proves
  sixteen inactive grants do not block migration, while the sixteen-device
  cap still applies to sealed grants. `ui/tests/remote-section.test.tsx`
  proves the re-pair label, revocation and sealed QR/link lifecycle.
  `wire::tests` checks owner
  authority and companion refusals through the real handler; `hotline-cli/tests/pair.rs`
  proves SIGINT and SIGKILL close the claim route through the running CLI.
- **Residual risk:** this is not traffic-analysis protection: a proxy sees
  paths, timing and sizes and can deny service. Pending sockets rotate within
  a 16-total, 4-per-TCP-IP budget, with one second before eviction and five
  seconds from accept through authentication. Authenticated sockets use a
  separate 16-total, 4-per-device
  budget. HTTP keepalive cannot renew anonymous capacity; forwarding headers
  have no effect. Saturation evicts only pending sockets and waits for their
  final owner to release them before replacement. Sustained connection floods
  can still disrupt handshakes and require edge traffic controls.
  The local desk seat, service account, secret store and QR display are
  trusted. QR scrollback remains sensitive until consumption or expiry.
  Physical iOS and real certificate/proxy deployment QA remain separate from
  Rust network harnesses and shared deterministic vectors.


## Remote admission behind a tunnel (BRO-163)

- **Default and grant source:** the same admission rules cover desktop and
  served listeners. Device identity comes only from a completed Noise handshake
  against a paired public key. Pairing attempts remain in the pending pool and
  require an explicit, unexpired, single-use invitation.
- **Enforcement:** `remote/admission.rs` owns pending rotation, atomic promotion,
  device budgets and final-owner cleanup. `remote/server.rs` keeps the same
  permit across TLS, HTTP and spawned WebSocket tasks. Revocation retains the
  wire's cleanup path for subscriptions and pending commands. The socket
  reader observes silence and cancellation while global commands wait;
  only a seat that exceeds the write deadline selects zero TCP linger, so
  its queued bytes are discarded on close before admission is released.
  The writer belongs to the seat future, including when that future is dropped;
  abortive close consumes the transport without yielding. Ordinary WebSocket and HTTP
  closes retain normal drain behavior.
  No forwarding
  header can manufacture a separate budget or device identity.
- **Tests:** `remote::admission::tests` covers global and per-device bounds,
  IPv4-mapped peers, eviction/promotion races, final-owner release and expired
  admission. `remote::tests::admission` drives disposable TLS listeners
  through a local TLS-terminating proxy, exercising repeated anonymous pool
  saturation, paired reconnects, surviving sessions, HTTP keepalive, stalled
  Noise, raw TCP and multiple devices sharing one proxy IP. The existing remote
  suite covers pairing, roles, viewer sockets and revocation.
  `wire/tests/concurrency.rs` proves silence expiry during a pending global
  command and desktop work surviving disconnect;
  `remote/tests/phone.rs` `a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound`
  proves the stalled connection ends in a TCP reset;
  `a_stalled_seat_resets_tcp_within_one_second_of_releasing_admission` bounds
  that reset to one second after admission reaches zero.
  `wire/tests/phone_pressure.rs` `dropping_a_seat_drops_its_blocked_writer_before_returning`
  proves cancellation leaves no detached writer sending after its seat ends;
  `a_revoked_seat_drains_tcp_instead_of_resetting_it` in the same file proves
  ordinary revocation drains without a reset.
- **Testing limits:** the proxy harness uses one upstream TLS connection per
  client. It does not reproduce Cloudflare's upstream connection pooling,
  production latency or flood defenses. Controlled Cloudflare staging and
  physical-device validation remain deployment checks; do not saturate the
  production desk. This is bounded admission, not protection against an
  unbounded connection flood.


## A desktop's public address (BRO-265)

- **Default and grant source:** none. An owner or the local desk may keep one
  https origin in Settings › Remote for a tunnel, reverse proxy or forwarded
  port. It grants nothing: it is only advertised, while Remote is on, in the
  hello (after the computer's own addresses, and named `publicEndpoint`) and
  in the pairing QR as `x`, a fallback after the network address `u`. A phone
  away from home can pair through it, but only with the same two-minute,
  single-use invitation it would claim beside the desk.
- **Enforcement:** `Remote::set_public_url` accepts only a bare https origin
  (no path, credentials, query or fragment); a served desk refuses it, since
  its `--public-url` is fixed at start. Changing it restarts nothing and
  disconnects no one. Sessions through it are the same sealed v2 sessions, so a
  TLS-terminating proxy cannot read, forge or redirect them; the desk's
  self-signed TLS stays as it is and the proxy must accept it.
- **Tests:** `remote::tests::addresses::a_public_address_is_advertised_after_this_computers_own_and_never_replaces_the_pairing_address`
  covers the order, the QR, validation, persistence and clearing;
  `remote::tests::addresses::a_phone_away_from_home_pairs_through_the_public_address_and_sees_itself_and_the_desk`
  pairs and connects through a tunnel at the QR's `x` with the own address unreachable;
  `wire::tests::remote_controls_are_denied_to_companions` covers the seat.
- **Residual risk:** everything in "Remote admission behind a tunnel" applies:
  every visitor arrives from the proxy's address and shares its pending
  budget, so a public address needs edge rate rules. The address itself is in
  every paired phone and is not a secret.


## Desktop clients and owner parity (BRO-145)

- **Default and old records:** Remote remains opt-in. Explicit pairing creates
  the requested owner or companion grant; missing roles still mean owner.
  Owners, including existing phone owners, now have the local desk command
  and subscription set. Companions retain their allowlist and transcript limits.
- **Grant source:** a two-minute, single-use invitation from an operator. QR,
  JSON and link carry the same invitation. The client saves a device key and
  pinned desk key in SecretStore before consuming it; the registry contains
  only desk ID, name, URL and public key.
- **Enforcement:** `remote::client` requires HTTPS and completes Noise IK
  against the saved pin before accepting application data. TLS certificates
  can rotate: TLS signatures are verified, while Noise supplies desk identity.
  `Seat::permits` and `permits_sub` admit owners and refuse companion admin
  before dispatch. Revocation still cancels live remote calls and sockets.
  Remote output remains bounded; owner frames may be up to 32 MiB.
- **Tests:** `rust_client_pairs_pins_and_recovers_subscriptions_without_replaying_commands`
  and `rust_client_companion_cannot_change_settings_or_subscribe_to_room`
  exercise real TLS/Noise and the handler. The CLI pairing integration test
  uses a disposable `hotline serve --store file` and verifies JSON, link,
  successful claim, single use and process lifetime.
- **Residual risk:** an owner can administer the service account's files,
  credentials and grants, just like the local desk. Protect owner device keys
  accordingly. Network failure cannot prove whether an interrupted mutation
  completed; the client never automatically resends it. Pairing links are
  secrets until consumed or expired. Physical laptop/phone and proxy QA are
  separate from these disposable network harnesses.

### Authenticated device rejection

- **Default and grant source:** this grants no access. Unknown and revoked keys
  receive the same refusal only after a valid Noise IK message 1 proves knowledge
  of this desk's public key and possession of the device key. Invalid handshakes,
  mismatched viewer bindings and failed pairing claims still close silently.
- **Enforcement:** the refusal is the encrypted message 2 payload
  `{"error":"device_not_authorized"}`. The client classifies it as `Revoked`
  only after Noise verifies it against the pinned desk identity, then stops
  reconnecting until the caller pairs again. TLS failures, timeout, reset,
  unauthenticated closure and unsealed error bytes remain `Unreachable`.
  Live revocation still closes the old session immediately; its reconnect learns
  the authenticated refusal. This also covers a device revoked while offline.
- **Reasoning and residual risk:** the only disclosure is “this desk exists and
  rejects your key,” to a party that already holds the desk's public key. A
  scanner without that key receives nothing new. No grant, role or room content
  is disclosed; unknown and previously revoked keys are indistinguishable.
  Someone on the network path can still cause unreachability, but cannot forge
  revocation without the desk's private key. Existing handshake limits apply.
- **Tests:** `explicit_pairing_is_atomic_and_only_owners_have_desk_authority`
  verifies encrypted unknown/revoked refusals and silence for a wrong desk key.
  `rust_client_reports_revoked_only_after_a_pinned_authenticated_rejection`
  covers live/offline revocation and a wrong pin; `rust_client_does_not_trust_a_close_or_an_unsealed_rejection`
  covers unauthenticated closure and a forged refusal over TLS/WebSocket.

### The relay grant role (ADR 0006)

- **Default and grant source:** no grant changes role, and nothing is a relay
  grant until the person at a served desk's terminal runs `hotline pair
  --relay` (or `remote.pairing` with role `relay` from an owner seat) and a
  claim declaring `purpose:"relay"` takes it. A desktop's own Remote never
  invites a relay, because it never hosts one.
- **Enforcement:** the role table `Purpose::admits` in `remote/server.rs`
  runs after Noise message 1 authenticates the key and before any seat, wire
  or relay registration: `/v2` and `/v2/computer/…` admit owner and
  companion, `/v2/relay` and `/v2/relay/accept` admit owner and relay. The
  relay's `/v2/link` answers a relay key `role_not_permitted` too.
  `Seat::for_phone` takes a `WireRole`, which has no relay, so a seat cannot
  be made for one. `claim_v2` checks the claim's purpose against the
  window's role after the secret, refusing a mismatch without consuming the
  window. Relay grants live in `remote.json`'s `relayGrants`, which an older
  reader ignores; the sixteen-device limit counts both lists, and a key is in
  one list at most. Revocation is the grant's lease, as for any device.
  `Purpose::of` answers a door only when the server runs its capability:
  a relay-only server has no `/v2`, `/v2/computer/…` or `/v2/link` at all,
  and its Remote holds no room (`agents` is `None`), so nothing it answers
  can reach one; its pairing makes relay grants only.
  `remote.become_relay` is the one way a grant changes role over the wire:
  its handler (`Remote::become_relay`) takes the device id from the caller's
  authenticated handshake, never from params, moves only an owner grant to
  `relayGrants` and cancels the device's token as `revoke` does. A desktop
  keeps a server paired as a relay in `relays.json`, never `desks.json`, so
  an older window neither lists it as a desk nor dials its `/v2`; a
  `role_not_permitted` answer is `OpenError::NotPermitted`, which never
  marks a pairing revoked.
- **Tests:** the four relay rows of the regression matrix, from the role
  and its doors through the end-to-end harness `hotline-cli/tests/relay.rs`.
- **Residual risk:** a relay grant may stand in for any desk id, as an owner
  may (ADR 0002): denial of service at worst, since visitors pin the desk's
  key. An older build that saves `remote.json` drops relay grants, which
  fails closed. An old phone shown a relay's code learns only that its claim
  failed.

### Loopback bridge, server files and laptop cookies

- **Default and grant source:** no new teammate capability. The loopback bridge
  has a fresh token and binds only `127.0.0.1`; its caller retains the paired
  device's owner or companion role. File administration and cookie push require
  owner or local desk authority through the same command gate.
- **Enforcement:** the bridge authenticates each local upgrade and forwards over
  Noise. Viewer handshakes bind the persona; bearer credentials stay server-side.
  Each wire socket owns its upload handles. Chunks enforce offsets and size;
  finish publishes without overwriting, and dropping the socket removes staged
  files. A filename upload is confined to a fresh directory below the desk's
  `uploads`; exactly one of filename or server path is accepted. Separators and
  traversal are refused before creation. Cancel/disconnect removes the staging
  directory; finish retains the complete file for attachments.
  Browse/download use server paths under the service account's authority.
  Cookie push validates the selected domains and drops expired cookies before
  starting a computer, bypasses server browser discovery, and records only source
  metadata and site counts. These commands are not teammate tools.
- **Tests:** `bridge_authenticates_locally_and_forwards_files_under_the_remote_seat`
  proves local-token refusal, companion refusal, upload ownership, round-trip
  bytes and disconnect cleanup. `bridge_viewer_preserves_binary_frames_and_takeover_without_exporting_the_bearer`
  proves viewer forwarding and revoke. `cookie_push_is_operator_only_through_the_real_handler`
  proves the seat gate; `laptop_cookies_reach_the_computer_but_never_the_room_or_tape`
  proves computer delivery and absence of cookie values from transcripts.
- **Residual risk:** an owner can read the service account's files and fill its
  disk; these are operator capabilities, not workspace-confined agent tools.
  Local processes that steal the bridge token acquire that device's authority.
  Revocation/disconnect is not rollback of files already published or cookies
  already delivered. An abrupt server process crash may leave staged temporary
  files for the operator to remove.

### Files over the sealed computer viewer

- **Default and grant source:** paired owners and companions that can open a
  teammate's viewer can operate on that computer's home, just as they can through
  its desktop. These are viewer requests, not teammate tools or owner-only
  desk-host `files.*` commands. No viewer-supplied host, port or bearer is used.
- **Enforcement:** the existing sealed handshake binds the device, viewer purpose
  and persona before opening the upstream connection. The desk intercepts only
  the six file operations, resolves the running computer's port and bearer, and
  uses authenticated HTTP with no redirects. The computer's file handlers enforce
  home confinement. Viewer List/Download paths go unchecked on the desk side to
  the computer's `/files` and `/files/download` endpoints; the computer confines
  them to home. Only uploads are checked desk-side by `destination()`.
  Responses omit upstream error bodies and credentials.
  Revocation closes the viewer and aborts its file worker. Upload handles and
  spools belong to that connection; chunks enforce offsets and decoded size.
  Publishing requires the computer's advertised atomic create-only operation:
  a listing preflight alone cannot prevent a competing writer from losing data.
  Computers without that capability refuse uploads rather than overwrite.
- **Tests:** `remote/tests/viewer_file.rs`
  `sealed_viewer_files_are_chunked_private_and_connection_scoped` enters the real
  bridge and sealed handler against a disposable HTTP computer. It proves file
  round trips, a single streamed download, bearer confinement, denied paths,
  foreign handles, conflicting uploads and cleanup on cancel, close and revoke.
  `sealed_viewer_files_reject_legacy_overwrite_and_keep_controls_responsive`
  proves companion computer access, safe refusal of legacy uploads and responsive
  viewer controls. Existing sealed viewer tests cover unpaired devices and
  mismatched purpose/persona bindings.
- **Residual risk:** the computer remains responsible for its filesystem boundary
  and atomic publication. A paired viewer can fill available disk space; queues,
  handles and chunk memory are bounded, total transferred bytes are not. Published
  files and already-dispatched HTTP requests are not rolled back by disconnect or
  revocation. Process crashes can leave desk-side temporary spools behind.

### Single-use loopback viewer tokens (BRO-148)

- **Default and grant source:** the trusted shell calls
  `Bridge::viewer_token(persona_id)` to mint an in-memory viewer capability. It
  expires 30 seconds after minting and grants one WebSocket upgrade for that
  persona, under the bridge's existing paired-device authority.
- **Enforcement:** only `/computer/<that persona>/ws` accepts it, carried as
  `Sec-WebSocket-Protocol: hotline-viewer.<token>`. The bridge echoes the selected
  protocol. The token is never accepted on `/ws`, another persona, another bridge,
  or through the query string. Validation and consumption share one mutex, so
  racing upgrades have one winner. Expired entries are pruned on mint/redemption;
  dropping the bridge cancels sockets and discards its tokens. A reconnect needs
  a fresh capability even if the remote computer was unavailable. Thirty seconds
  limits redemption, not the lifetime of an already-open viewer; the existing
  paired-device revocation still closes that viewer.
- **Shell boundary:** `hotline-app/build.rs` declares every registered app
  command in Tauri's `AppManifest`. Without it, app commands bypass Tauri's
  capability checks even if core and plugin permissions are window-scoped.
  Only the trusted `main` window receives the `main-commands` permission set.
  The bundled `computer-*` viewer windows receive exactly `desk_viewer_token`,
  `transfer_begin`, `transfer_write` and `transfer_end`: reconnect with a fresh
  token and save downloaded bytes. They cannot call `desk_list` (which exposes
  owner bridge tokens), pair or forget desks, read laptop files or cookies, or
  administer updates. They receive no core or plugin permissions and no remote
  origins are granted access. The main window supplies the native viewer title
  at creation; native window-manager close needs no JavaScript IPC permission.
- **Compatibility:** `/computer/<persona>/ws` refuses owner `?token=` credentials;
  only `/ws` accepts the owner token. Shell and bridge ship together, so no
  mixed-version fallback is needed. Mixed query/protocol credentials and
  multiple viewer protocols are refused without falling back to owner authority.
- **Tests:** `remote/tests/viewer_token.rs` enters the real loopback upgrade
  handler and proves route/persona/bridge isolation, single use, concurrent
  redemption, protocol echo, a fresh reconnect, owner-query refusal and expiry
  after 30 seconds.
  `bridge_viewer_preserves_binary_frames_and_takeover_without_exporting_the_bearer`
  covers viewer-token forwarding and remote revocation.
  `hotline-app/tests/window_capabilities.rs` uses Tauri's real IPC dispatcher
  with the shipping generated ACL to prove main-command access, the viewer's
  four-command limit, denied plugin access, that only the main window may
  register, unregister and unregister all global shortcuts (and may not ask
  which are registered), and refusal from unrelated windows and remote
  origins. It also keeps both platform handler lists, the manifest
  and the main permission set synchronized.
- **Residual risk:** a stolen unspent token can win the upgrade race for its one
  persona and access that computer's screen, controls and files. The header can
  appear in handshake logs; the secret is narrower and short-lived, not invisible.
  The viewer's allowed shell calls can request another token for a known desk
  and persona and start downloads; this is a boundary against owner desk and
  shell administration, not isolation between computer viewers. Download handles
  are process-wide random ids, not window-bound. A compromised main window or
  local process that steals its owner token still has owner authority.


### Voice calls

`voice.*` commands and `{"call":"id"}` subscriptions are restricted to the local
desk and owner seats in `wire::Seat`; companions receive `forbidden`. The hidden
historical `voice-dispatcher` tape is also refused as a companion subscription.
New calls never write it. No microphone audio is stored: calls keep only
transcriptions, spoken text and tool events.

`voice.text` is an additive finalized-transcript input on an explicitly
negotiated text call; omitted input mode retains audio. The same socket-bound
call cancellation, monotonically increasing sequence and pending-turn gate
apply. There is no remote transcription request or STT reservation for text,
and it adds no permission, grant or caller-controlled origin.

Without a lead, the fallback assistant executes bounded named operations in
`voice::dispatcher::Context` through typed room methods, never the wire command
table. It cannot answer permission/human/passkey cards, edit policies, read
arbitrary files, or change credentials. Its text handoffs retain the teammate's
existing permissions and approval gates. With a lead, the desk uses that
teammate's own call thread and the lead-call grant described below.
Disconnect/revocation cancels the call's authority to dispatch further actions,
while already accepted teammate work continues. Clip queues are bounded; an
out-of-date subscriber is disconnected instead of losing audio silently.

Only a context created for a new heard utterance can invoke action commands.
Narration has no tools. Conversation tails, history, and narrated replies are
escaped untrusted blocks; this is a prompt boundary, not a guarantee against
model misinterpretation. The core caps handoffs at three per utterance and
rejects attachments and reply targets. Startup is asynchronous, with a bounded
wait, and a queued acknowledgement never establishes successful delivery.

Voice-created schedules are agent-origin jobs (`operatorCreated: false`), require
the live background-work grant under the ordinary schedule mutation lock, and
are checked again before firing. The dispatcher refuses loops, intervals, quiet
jobs, and cancellation of jobs created outside this call. The trusted voice
origin is an internal task scope; wire parameters cannot set it. Reply provenance
is retained in the opaque user event ID so only voice handoffs can rewrite an
out-of-call push: a reply too long to say is summarised for its push only when
a voice handoff asked for it, and a typed prompt's reply is pushed as written.
A held call sends normal pushes.

Proofs: `voice::room::tests::voice_schedules_require_a_grant_and_cannot_escape_the_call_scope`,
`a_heard_turn_can_queue_only_three_handoffs_without_waiting_for_startup`,
`session::tests::voice::a_long_reply_is_summarised_for_its_push_only_when_a_voice_handoff_asked_for_it`,
`short_voice_handoff_replies_keep_their_text_without_an_extra_model`,
`wire::tests::voice_is_owner_only_through_the_real_handler`, the voice
state-machine text/sequence/cancellation/accounting tests,
`voice::room::dispatcher::tests`, and the WebSocket scripted client
in `crates/hotline-core/tests/voice.rs`. The speech/ledger tests cover provider
errors, fallback and persistent budget failures. Live provider latency is a
separate measurement; fake-provider test timings are not a production guarantee.

## Posting to the chat from any thread (ADR 0008)

`post_to_chat(text, wake?)` is one of Hotline's own tools: it writes one
message, as the teammate, into the teammate's own chat, stamped with the
thread it came from.

- **Default and old records:** nothing new is stored and nothing is granted
  per teammate. The tool is on the desk's MCP server and among Hotline
  Agent's in-process tools in every session but a subagent run's, because an
  ACP child's tool list is fixed when its session starts; what may post is
  decided per call. It replaced `tell_person`, which only a quiet run on
  Hotline Agent was handed, for its one turn; a quiet job, on either driver,
  is heard through it.
- **Grant source:** the thread's kind, read by the handler. A work thread, a
  pair (into the target's own chat) and a job's thread (stamped with the
  job) may post. Refused: the DM's own turn ("your reply is already the
  chat"); a subagent run, which reports to the teammate that launched it; a call that lands while no turn of its thread is in flight; a
  second post in a turn, and so a second wake; `wake` from the DM itself; any
  argument but `text` and `wake`, so no other teammate's chat can be named;
  and a revoked lease. A voice front's `post_to_chat` is the call's own
  (`hotline-voice/src/front_tools.rs`), under the teammate's current lease.
- **Enforcement:** `TeammateTools::call` checks the session's capability
  lease first, as for every tool, and refuses a run `post_to_chat`,
  `send_file` and `generate_image` by name. A job's file
  (`session/files.rs`) takes the turn's one post through the same
  `Room::take_post` that `post_to_chat` does, and `Room::may_send` refuses a
  file or image before it is fetched once the job's turn has posted.
  `Room::post_to_chat` (`session/post.rs`) applies the rest against the
  `Posting` the turn opened: `Room::open_posting` is held by the DM's turn,
  by `Threads::turn` for work threads, runs and jobs (with the job, which
  stamps its posts), and by a pair's exchange turn, and is dropped when the
  turn ends. The text is
  clipped to 2,000 characters and written through `Room::post`, the chat's
  one door; a write that fails gives the turn its post back. `wake` brings
  the DM up, posts into its chapter and queues one line on the DM carrying
  the posts, and the DM's reply is what the person hears.
- **Tests:** the `post_to_chat` row of the matrix: every refusal through the
  real handler on both drivers, Hotline Agent in process and ACP through a
  real child over the served endpoint, and the posts that land read off the
  wire.
- **Residual risk:** a teammate on a long work thread can post once a turn,
  every turn, and each post pushes like a reply; a `wake` costs a DM turn. An
  ACP harness's own subagents share their parent's MCP server and the handler
  cannot tell them apart, so a post from one is the parent thread's and
  counts as its one. A run on an ACP child is refused by the same handler,
  but nothing starts one yet (voice's work tool will), so the run refusal is
  proved in process only. An agent learns a job is quiet from the
  word `quiet` on its line, so its model may say it is staying silent: those
  words land on the job's thread, never in the chat.

## Jobs on their own threads (ADR 0008 §7)

A job's firing is a turn on its own thread, by an agent of the teammate's
that the room builds for it, rather than a turn of the DM's session.

- **Default and old records:** nothing is granted per teammate. A schedule
  written before this fires on its job thread under the rules it always had:
  an agent-created job needs Background work at every firing, and an
  operator-created one runs without it. A plain reminder starts no agent.
- **Grant source:** the job's record on the room stream (who created it, and
  the live Background work grant) for whether it fires and whether a queued
  firing's turn begins; the job kind's policy row (`thread/policy.rs`) for
  what its agent is handed.
- **Enforcement:** the clock (`session/schedule.rs`) checks the grant before
  a firing; the job thread checks it again as the firing's turn begins
  (`session/job_thread.rs`, `Occupant::begin`). The agent is a clone of
  the home agent, built by `Room::thread_agent` under
  `lease_of(ThreadKind::Job, …)`, a child of the teammate's authority, made
  when the agent is built and revoked when it is let go of, so a stop, a
  policy change or a deletion refuses its next call and a stale agent is
  rebuilt rather than reused. What it is handed, and its tools, are the
  clone's: see [Clones of the home agent](#clones-of-the-home-agent-adr-0008-0).
  `cancel_schedule` with no `id` ends this job only. Its cards are the
  person's, raised on its thread and answered through `human.answer` or
  `thread.answer` back into it; a companion reads a job's thread as it
  reads a run's, and may not close (cancel) a job.
- **Tests:** `session/tests/posting.rs` `a_jobs_thread_posts_where_a_run_may_not`
  (its tools and the refusals, through the real handler);
  `session/tests/scheduled.rs` `a_queued_firing_is_dropped_when_background_work_is_revoked`;
  `wire/tests/threads.rs` `the_phone_seat_runs_thread_commands_as_it_ran_the_old_ones`;
  and `tests/schedule.rs`, a harness test for each rule of §7.
- **Residual risk:** a job's agent works unattended, between the person's
  visits, with the home thread's permission set and the teammate's computer;
  it can post once a turn, a loud job posts every firing, and a card it
  raises pushes like the DM's.

## Clones of the home agent (ADR 0008 §0)

A subagent run and a job are clones of the teammate's home agent: the same
teammate with one focus, on the home agent's model and workspace, with the
home thread's permission set, starting from its brief rather than the home
thread, and reaching back into it with the home agent's own reading tools.

- **Default and old records:** a clone has everything the teammate's record
  grants the home thread: its reach, its granted folders, its MCP servers,
  its skills and its computer, shared through the computer gate as a work
  thread shares it (`Computer::Lease` on the `Run` and `Job` rows of
  `thread/policy.rs`); and the model and effort the home session is on now,
  else the record's. Before this, a run had no computer and was Hotline
  Agent's only, and a job ran on the record's model with no computer. A job
  written before this has no `without`, so it has the whole set.
- **Grant source:** the teammate's record, read when the clone's agent is
  built, narrowed by what the home agent left out for the task: `without`
  on the `subagent` tool (`session/jobs.rs`) and on the `schedule` and
  `loop` tools (`mcp/server.rs`), each of `computer`, `folders` (granted
  folders, and whole-machine reach, which becomes the working directory's)
  and `mcp_servers`. A job keeps it on its record (`ScheduledJob.without`);
  a list a desk cannot read leaves every ability out. Nothing widens it: the
  person's job has no `without`, and a job cannot schedule more work, so
  what it was narrowed by cannot be shed by a job it made.
- **Enforcement:** `clone_of` (`session/agent.rs`) makes the view the agent
  is built from, which only takes away; `Room::thread_agent` builds the
  agent from that view (the computer is granted only to a view that has it,
  the MCP servers are the view's policy's, Hotline Agent's file tools and
  shell take the view's reach and folders, and ACP file callbacks the view's
  folders), and a job's turns are reached from the view's reach. The lease
  is the home thread's: a run's is `scoped()` from the asking session's, a
  job's from the teammate's authority, so a stop, a policy change or a
  deletion revokes every clone, and `Room::run_to_end` checks the
  session's lease before it builds anything. The tools are gated in
  `TeammateTools::call` (`mcp/server.rs`), outside the model, for both
  drivers: a run has `RUN_TOOLS`, the reading ones, and is refused the doors
  to the person; a job has the teammate's tools less `JOB_WITHOUT` (the
  chapter moves, `react`, `open_link`, `schedule`, `loop`). A job's
  `request_human` card is written on its own thread (`Room::ask_human`), a
  colleague's answer to its `message_teammate` is routed by
  `Request::reply_job`, and both come back through
  `Room::deliver_into_job` while the job lives, else to the DM; one the
  last process wrote and had not handed over is handed over at start the
  same way (`Room::resume_job_deliveries`), so a restart neither drops it
  nor hands it to a job that has ended. A run
  starts only from a brief that runs now and reports to whoever asked
  (`Room::run`); later work is a job's.
- **Tests:** the matrix row below: a subagent and a job each reach back
  into the home thread and read the granted folder, and the narrowed one
  cannot read it, through the real core; a job's card goes to its thread
  and its answer comes back there, after a restart too, or to the DM once
  the job has ended; a clone has the computer unless it was
  left out; a run on an ACP teammate is built on its own harness; a run's
  lease dies with the home session's; and a job's thread is found by
  `search_thread` after the index is rebuilt.
- **Residual risk:** a clone has as much as the home thread, unattended: a
  run works without anyone reading it, and a job between the person's
  visits. Narrowing is the home agent's choice, made by the model, and a
  home agent that does not narrow hands a clone everything it has. Reach is
  Hotline Agent's policy: an ACP clone's own tools are its harness's, so
  `folders` narrows only what Hotline enforces for it (its delegated file
  operations and its computer's mounts). Nothing starts a Hotline run for
  an ACP teammate yet; its harness's own subagents are mirrored as runs and
  share its home session's MCP endpoint, with the home thread's tools.

## Thread commands on the wire (BRO-203)

- **Default and old records:** nothing new is stored, and no seat gains a
  capability: `thread.*` is the older commands under one name, and each seat may
  run a verb on a kind of thread exactly when it could run the old command for
  that kind. A socket reads the newer shapes (`link`, `thread_delta`) only after
  it says so with `client.hello`; one that never does is sent what it always was.
- **Grant source:** `phone_thread_command` and `phone_may_read`
  (`wire/seat.rs`), beside `Seat::permits` and `permits_sub`. The desk and an
  owner run everything. A companion:

  | verb | companion may | because |
  | --- | --- | --- |
  | `thread.prompt` | a work thread only | it speaks to a teammate with `mobile.prompt`, and `session.prompt` is refused it; `side.prompt` is not |
  | `thread.answer` | anything but a pair | `side.answer_permission`, `session.answer_permission` and `human.answer` are allowed, `peers.answer_permission` is the owner's |
  | `thread.cancel`, `thread.park`, `thread.close`, `thread.continue`, `thread.open` | yes, but never `thread.cancel` or `thread.close` on a job | `session.cancel`, `side.*` and the exchange stop and resume are allowed; parking is less than `side.archive`; closing a job cancels it and stopping its firing is the same owner's call, and `schedule.cancel` is refused it |
  | `thread.list` | only what it may read: no pairs, no calls | `peers.list` is refused it, and a call summary's `preview` is what was spoken in it, which a companion may not subscribe to or page |
  | `thread.page`, `{"threadId": …}` | any kind but a call, and never the voice dispatcher's tape | what `tape.page` and the old targets reached; a call is `Target::Call`'s, which a phone is not seated on |
  | `client.hello` | yes | it only chooses the shapes this socket is sent |

- **Enforcement:** the seat check is on the command, in the core, before any
  handler runs. Each handler then applies the kind: a verb with no meaning for
  it is refused in a sentence (`wire/threads.rs`), a card in a kind whose
  `Policy::answer` is `Nobody` (a run, a call) is never answered, and the
  answer is routed to the room method the old command called, which holds the
  same lease and card checks as before. `thread.park` goes through
  `Room::park_side_thread`, which is the sweep's own park.
- **Tests:** `wire/tests.rs`
  `the_phone_seat_runs_thread_commands_as_it_ran_the_old_ones` (the table),
  `a_thread_verb_the_kind_has_no_meaning_for_is_refused`,
  `thread_verbs_reach_the_handlers_the_older_commands_did`,
  `thread_list_reads_every_kind_as_one_summary`,
  `a_companions_thread_list_leaves_out_calls_and_pairs`,
  `a_thread_subscription_by_id_reads_the_stream_its_kind_keeps`,
  `a_threads2_client_is_sent_every_link_as_the_link_itself`, and the delta tests
  that send each shape; `remote/tests/phone.rs` for the hello a phone is sent;
  `tests/threads.rs` `a_work_thread_is_one_conversation_to_old_and_threads2_clients`
  runs one conversation through the real core for an old and a `threads2` client.
- **Residual risk:** `thread.list` with no `personaId` reads every teammate's
  threads, titles and last lines in one answer. A companion already reads every
  tape and work thread by subscribing, so this adds no reach, only one place to
  read it.

## One computer, several threads

A teammate's work threads, its main conversation and a colleague's ask all
reach the same computer, so they take turns. The computer's MCP URL is handed to
each agent through a loopback gate (`computer/gate.rs`) with a token of its own;
the gate replaces it with the real bearer only when it forwards. `tools/call`
takes the teammate's exclusive lease: a second thread waits up to 20 s and is
then told the computer is busy and which thread has it, never queued silently.
The hold ends with the holder's turn, after 30 s without a call (never while a
call is in flight), or when the thread's capability lease is revoked (it
closes, parks, is stopped, or the teammate's authority changes), at which point
the gate closes too.

Three rules keep that exclusion honest. Authority is asked again while a call
waits and once more with the lease in hand, immediately before the call is
forwarded, so a call queued behind another thread when its thread is parked,
closed or stopped is answered `403` and never reaches the computer. A hold is
kept by the agent that took it (`Holder::new` gives each agent its own owner),
not by its thread's name, so a DM's replacement agent is not released by the
old one's late turn end or gate shutdown. And a release (turn end, park, close,
stop) while a call is in flight takes effect when the call ends: revocation
refuses new calls at once, but another thread cannot take the computer while the
upstream operation is still driving it. Tests: `computer/gate.rs`
`a_call_queued_at_the_gate_when_its_thread_is_revoked_never_reaches_the_computer`,
`a_waiting_call_whose_authority_ends_is_cancelled_and_never_given_the_computer`,
`a_retiring_agent_cannot_release_its_replacements_hold`,
`a_release_while_a_call_is_in_flight_keeps_the_computer_until_it_ends`. The gate forwards only to the URL the thread was already
granted, so a thread never has a computer wider than the teammate's grants, and a
teammate's lease never covers another teammate's computer.


## Call-thread agents (S5 and S6)

- **Default and grant source:** a call uses the selected teammate's
  tools and standing grants, without chapter moves. Its independent lease
  survives a home-session chapter change and is revoked by the existing
  work-thread stop, policy-change and removal paths. Voice grants nothing.
- **Enforcement:** `Room::thread_agent` builds both drivers under the Call
  policy; the shared work occupant checks its lease at admission and the
  real tool handlers check it on use. `validate_call_thread` refuses another
  teammate's thread, archived threads and nonexistent attachments. A call's
  `post_to_chat` uses the same turn-scoped posting gate as a work thread.
- **Proof:** `tests/call_threads.rs`
  `a_call_has_only_its_teammates_grants_and_cannot_attach_another_teammates_thread`
  proves denied scheduling without Background work, allowed posting, and
  refused cross-teammate attachment on Hotline Agent and ACP;
  `calling_on_a_thread_under_way_queues_speech_on_that_same_thread` proves
  attachment and queued speech through the desk wire. Existing work-thread
  lease tests continue to cover the shared revocation path.
- **Residual risk:** an ACP harness's own tools remain outside Hotline's
  sandbox, as for its work threads. A failed summary is reported explicitly with the thread
  link, rather than fabricated.


## Reading a teammate's own threads

- **Default and grant source:** `read_thread` is an own-conversation tool on
  Hotline's MCP server for both drivers, including runs, jobs and voice
  fronts. It adds no policy field or migration. A DM's key must be the
  caller's persona; a call, work thread, job or run must have that persona's
  DM as its parent. A pair must include the caller. Being the opener of
  another teammate's work thread does not grant its transcript.
- **Enforcement:** `TeammateTools::call` checks the calling session's lease
  before dispatch, in process and over the ACP endpoint. `thread_reader::read`
  resolves the key with `ThreadStore` and checks ownership before returning
  any events. Pages follow folded event order using event ids, never a
  caller-supplied path; records are fenced as conversation data. A stop,
  policy change or deletion revokes the same handles as the other tools.
- **Proofs:** the matrix row below covers allowed reads, another teammate's
  refused record, paging without losing text or tool updates, and revocation.
- **Residual risk:** the caller can read its entire own record, including
  tool output. Pages bound event count, not bytes; one event can be large.
  The log currently folds the whole stream for each page. Recorded speech
  is the transcript kept by the call engine, not recovered audio.

| Boundary | Tests | Platform |
| --- | --- | --- |
| Own-thread reads use both drivers' shared handler; cross-teammate reads and stale leases fail; pages retain full text and tools in order | `tests/call_threads/reader.rs` `the_home_agent_reads_the_call_transcript_to_recover_a_detail_only_said_there`, `thread_reads_page_in_order_and_refuse_another_teammates_thread_on_both_drivers`, `work_and_job_threads_are_read_by_the_keys_the_home_agent_receives`; `session/tests/reading.rs` `thread_pages_keep_full_text_order_and_one_line_per_tool`; `session/tests/posting.rs` `a_revoked_lease_refuses_a_post_in_process_and_over_the_endpoint`; `session/sides/tests.rs` `stopping_the_teammate_archives_its_side_threads_as_stopped` | — |


| Boundary | Tests | Platform |
| --- | --- | --- |
| Live work uses its own driver/model and call transcript, posts its file, and leaves the home agent running through correction and cancellation | `tests/call_threads/live_work.rs` `live_work_uses_its_own_driver_and_posts_its_file_without_touching_home`, `changing_and_dropping_live_work_keeps_one_call_agent_and_home_keeps_running` | Both drivers; scripted Grok, GPT Realtime and GPT-Live |
| Front reads refuse revoked grants and direct writes; GPT-Live uses the same work admission | `tests/voice/front_tools.rs` `a_front_call_answers_from_a_desk_side_tool_and_a_revoked_grant_is_refused`, `gpt_live_delegation_uses_the_same_call_thread_work_tool` | — |


## Desk calls on the team lead

- **Default and grant source:** the roster's current `teamLead`, plus a live
  call placed to the desk, grant local `handoff` requests without first-contact
  approval. No policy field or permanent collaboration grant is installed.
  A direct call to the lead and its other threads get no new grant. Old
  exchange records carry none.
- **Enforcement:** `Calls::lead_call` checks target, live call and current lead.
  `TeammateTools::call` checks the capability lease and limits the front to
  lookups plus `message_teammate` with `handoff` on that desk call. The exchange
  records the source call only for a local recipient and rechecks both leases
  and the lead/call before starting or resuming the handoff. A queued request
  whose call or lead ended fails explicitly, including after restart. Stop,
  policy change and deletion retain the ordinary lease revocation. No tool
  for stopping or reassigning another teammate is added.
- **Proof:** `tests/voice/lead.rs`
  `the_desk_lead_looks_up_the_roster_and_hands_named_words_straight_on` proves
  roster lookup without a working-model request, unchanged named handoff
  without a first-contact card or dispatcher call, and denied ask, stop and
  former-lead use through the real handlers;
  `a_direct_call_to_the_lead_does_not_get_the_desks_handoff_grant` proves the
  direct-call refusal; `a_no_lead_front_knows_the_person_and_roster` proves
  fallback context; `session/exchanges/tests.rs`
  `a_saved_desk_handoff_cannot_recreate_a_live_calls_grant` proves recovery
  fails the old grant before creating work;
  `voice/room/tests/desk_calls.rs`
  `hanging_up_removes_the_leads_handoff_tool_from_an_existing_handle` proves
  a retained handler loses routing authority on hang-up. Existing front-tools
  tests prove lease revocation.
- **Residual risk:** the model selects the recipient and words. Prompt
  injection can misuse this bounded routing authority during the call; the
  core cannot prove that speech really named a recipient. Accepted work uses
  the recipient's own grants and continues after hang-up once started.
  Linked recipients retain ordinary collaboration consent. The fallback
  assistant remains only for desks with no lead, with its existing bounded
  operations; old dispatcher tapes remain owner-only historical data.


## Resuming a dropped call

- **Default and grant source:** the existing call and subscription seat checks
  remain. A socket drop permits twenty seconds of reattachment by the same
  authenticated device id (local desk sockets share the desk owner).
- **Enforcement:** `wire/socket.rs` binds the call to the socket and separately
  to its pairing's revocation token. `wire/subscribe.rs` reattaches through
  `Calls::bind_connection`, which refuses another device. Revocation bypasses
  the grace period; expiry takes the normal hang-up path.
- **Tests:** `voice/room/tests/text.rs`
  `a_short_transport_drop_resumes_but_twenty_seconds_detaches` proves the
  grace period and wrong-device refusal, and that a desk lead’s routing grant
  survives reattachment but ends on expiry; `pairing_revocation_during_a_drop_ends_without_the_grace_period`
  proves revocation during the hold. `tests/voice/front.rs`
  `a_wire_drop_resumes_the_same_live_call_and_announces_it` drives resumption
  through the real socket subscription handler. `wire/tests/owner_handler.rs`
  `a_call_subscription_cannot_resume_another_devices_transport` proves allowed
  and denied owners through the subscription handler.
- **Residual risk:** a dropped call keeps its provider connection for up to
  twenty seconds and can incur that connection's normal cost. This does not
  restore a revoked grant or cancel work already admitted on its thread.

### Automatic written call results

A live call's own turn posts a written result through `Room::post` when its
text contains code, a table, a link, a list of four or more items, or a write-up
over 120 words. Sent attachments use the existing file-post path. This is on
by default for an attached call and grants no new tool or destination: the
turn's originating persona and thread supply both, and the existing open-turn
and live-call checks guard the write. Detached work retains the bounded
in-flight completion rule. The model cannot choose another chat through this
path. Conversation equal to its speech is not posted again.

`tests/voice/direct.rs` `conversation_keeps_one_written_record_and_never_posts_or_points`,
`a_direct_call_uses_its_thread_agent_without_dispatching`, and
`legacy_speaks_its_own_untagged_opening` exercise the real core over the wire:
conversation stays on the call; tables and code reach the owning chat.
The existing thread-lease and revoked-post tests still cover tool authority.
Residual risk: classification is syntactic; a model can mislabel or omit useful
content, and its spoken wording can ignore an instruction. A missing or failed
narrator leaves the written result available without a new spoken pointer.

## Cards answered on a call

- **Default and grant source:** a card is desk-only. A live core-owned
  collaboration wait can accept yes for this session or no; voice never
  grants standing collaboration. `ask_question` creates an informational
  yes/no card under the requesting session's capability lease. Its answer
  conveys information, not permission for an operation. A core-owned
  `set_person_name` card can also accept yes/no by voice, solely to save its
  bound preferred name (see below). `request_human`,
  passkeys and opaque driver permissions remain desk-only, including requests
  involving credentials, 2FA, screens, payments and deletions. Older cards
  have no question marker; after restart no saved marker recreates a lease.
- **Enforcement:** `session/call_cards.rs` checks the actual card, live call,
  owner, pending status, expiry and question lease (including a name card), or the core's actual
  collaboration wait and both participant leases. The normal card handler
  then settles it once. A `question` or `voice` argument cannot promote an
  action card. The voice front has no answer tool or wire command access:
  only a finalized caller transcript equal to yes or no selects the last
  announced card. Holding or ending the call removes that input authority.
  Local Legacy capture reports its floor through owner-only
  `voice.input_state`; provider onset events do the same for live calls.
  A changed grant or stopped requesting session revokes question authority;
  withdrawal and a desk answer remove the queued/active card. Answers leave
  a record in the teammate's chat identifying the call. Call-thread cards
  are stored in that home chat with a core-stamped `from` origin and mirrored
  by id to the call transcript. Home answers route to the original wait or
  driver; moving the card grants no voice permission. Home-session cleanup
  cannot expire a call driver's pending card.
- **Proof:** `tests/call_threads/cards.rs`
  `legacy_answers_a_question_once_and_records_yes_and_no_from_the_call`,
  `live_cards_wait_for_the_caller_and_desk_actions_refuse_spoken_yes`, and
  `spoken_collaboration_approval_is_only_for_this_session`, and
  `a_home_answer_settles_the_call_question_and_removes_its_voice_notice` drive the real
  wire and handlers, with both drivers and the three live provider seams.
  `session/threads.rs` `a_call_permission_is_pushed_and_answered_in_home_chat`
  proves driver settlement and home notification routing without a live call.
  `voice/room/tests/cards.rs`
  `the_card_handler_refuses_actions_forged_questions_and_stale_authority`
  proves the five desk-action categories, forged flags, wrong owner, held
  and ended calls, and revoked question leases through the real handler;
  `duplicate_cards_are_said_once_and_withdrawn_cards_are_not_said` proves
  deduplication and withdrawal. `push.rs`
  `a_call_replaces_the_card_push_until_it_is_held_or_ended` proves notification
  routing; `wire/tests/owner_handler.rs` `voice_is_owner_only_through_the_real_handler`
  also refuses a companion's input-state signal.
- **Residual risk:** speech recognition may mishear yes/no; there is no
  speaker authentication. An informational answer can influence subsequent
  model work within its existing grants, but cannot settle a separate
  operation card or grant permission. Free-form question prose is not a
  trusted description of an operation, and the core does not classify it as
  one. Driver approvals need trusted operation metadata before their voice
  support can expand. Live providers generate the spoken announcement;
  physical playback/echo and microphone timing still need the device pass.


## Files produced by call work

- **Default and grant source:** successful write/edit records on a call's own
  turn are automatically delivered to that teammate's home chat. Explicit sends
  join the batch. This adds no model tool, filesystem grant or destination.
- **Enforcement:** `session/call_files.rs` reads only the recorded paths through
  `files::from_workspace` and checks the call's scoped capability lease before
  reads, storage and posting. Failed tools supply no automatic result. A stopped
  or revoked call cannot regain authority from a saved record. File readback
  selects the stored copy by message id and index; separate indexed directories
  prevent equal basenames from colliding. Other documents open as inert text
  or binary previews through `file.read`; the native image/PDF-only opener
  remains unchanged and markup is never executed.
- **Proof:** `tests/call_threads/files.rs`
  `call_files_are_delivered_once_and_large_batches_are_one_openable_card` covers
  both real drivers over the wire, one/three/six files, explicit sends, empty
  work, unrelated workspace files and readback of every indexed result.
  `session/tests/sent_files.rs`
  `automatic_call_files_keep_the_call_lease_reach_and_work_boundary`
  exercises the real publishing handler with allowed files, an outside path,
  revoked authority, later corrections and separate calls.
- **Residual risk:** ACP reports its own mutations, as it reports tool results;
  those reports do not widen Hotline's read reach. Tools without explicit file
  metadata (including shell commands) do not produce automatic attachments.
  The file is read when the turn finishes, so another authorized writer can
  change it before capture. Real-provider and physical-device behavior still
  requires the separate call QA pass.


## Saving the person's name

- **Default and grant source:** `personName` is absent until the person saves
  it in Settings → General or approves a `set_person_name` card. The shared
  tool is available to both drivers and the live front, under their session
  lease; subagent runs and colleague threads cannot ask. Each proposal, including a change, asks
  “Save your name as George?” with the proposed name. The person's desk answer
  or finalized yes on the matching active call grants only that write.
- **Enforcement:** `TeammateTools::call` checks the lease and dispatches only
  the name argument. `Room::ask_person_name` validates a nonempty name of at
  most 100 characters, with no control characters, and binds it and the lease
  to a core-generated approval id before publishing the card. `answer_later`
  serializes settlement with withdrawal and expiry; `settle_person_name`
  rechecks the pending operation, lease and expiry and appends only the
  `personName` setting on approval. Decline writes no setting. Duplicate,
  wrong-owner, expired, withdrawn and revoked answers cannot save a name.
  A restart loses the operation authority; the persisted card cannot restore
  it. Ordinary question text and extra tool arguments cannot authorize a
  setting change. Settings uses the same name normalization and may clear it.
- **Proof:** `tests/call_threads/person_name.rs`
  `a_name_is_saved_only_by_its_approved_card_on_both_drivers` drives proposals,
  decline, approve, repeated approval, change and revocation through both real
  drivers and the wire, and checks the current Settings name reaches home.
  `a_live_call_saves_a_name_only_after_spoken_yes` covers yes/no across all
  three live provider seams. `voice/room/tests/cards.rs`
  `a_front_can_only_propose_a_name_and_stale_or_forged_cards_cannot_save_it`
  covers the front handler, held calls, withdrawal, revocation, validation and
  forged operation arguments. `session/tests/cards.rs`
  `a_name_approval_cannot_survive_expiry_or_room_restart` proves expiry,
  wrong-owner refusal and restart refusal. `ui/tests/person-name.test.tsx`
  proves the General field's read, write, clear and room-update round trip.
- **Residual risk:** the model proposes the spelling and speech recognition
  can mishear approval; voice does not authenticate the speaker. The trusted
  card displays the exact bounded name to be stored. Approval is specific to
  that proposal, but two separately approved proposals can overwrite each
  other in settlement order. The saved name is prompt data, not instructions;
  no name proposal can write another setting or create a standing grant.

## Credential capture in the chat (task-266)

- **Default and grant source:** `request_credential` proposes a card under
  the requesting session's lease and enabled computer grant. It cannot read,
  store or grant a value. Only the authenticated owner/desk or phone can
  answer `secrets.capture.answer`. Missing name declines; supplied values
  create a new login; a matching existing name grants that key without
  replacing it. New and old keys default to `use: auto`. Capture persists
  and delivers the Ask me setting; use-time approval is described below.
- **Enforcement:** both agent drivers reach the same MCP handler. The card
  binds origin, teammate and lease before publication; the command accepts
  none of those replacements. A scoped lease, thirty-minute deadline and
  per-request mutex cover submission, withdrawal, expiry and minted passkey
  storage. The policy lock serializes the grant with policy and secret changes. An
  accepted answer survives loss of its phone socket and finishes settlement. A
  captured passkey also requires an operator answer and its exact origin;
  a poll reporting approval alone is insufficient. Restarted cards cannot
  acquire a fresh lease. The phone is refused other secret writes and the
  generic value-bearing thread-answer variant. Voice cannot answer it.
- **Data:** values enter only the capture command and vault/computer path.
  `CredentialValues` and vault records redact Debug; parse errors on capture carry no supplied
  content. Computer secret-delivery errors omit upstream bodies, which may
  echo a submitted value. Card events, pushes and deliveries carry only
  metadata. The form clears its fields before submitting, on dismissal,
  on settlement from either seat and at its local expiry deadline.
- **Residual limits:** a captured login contains only the requested origin,
  but Hotline Computer currently permits subdomains of that host with the
  same scheme and port. Strict origin-only filling needs a computer change;
  this branch cannot prove that acceptance line. Computer-side redaction is
  still protection against incidental disclosure, not a determined program
  that encodes an accessible value. The authenticated operator can still
  change a granted key afterward. A stopped computer receives its set at its
  next start, and a failed delivery uses the existing failure reporting.


| Must hold | Proof | Needs |
| --- | --- | --- |
| Capture stores the requested origin, grants and delivers once, survives phone disconnect, and no typed value reaches tape, agent output, Debug or delivery errors | `session/tests/capture.rs` `capture_stores_grants_and_delivers_without_values_in_any_sink`, `capture_an_accepted_answer_finishes_when_its_phone_disconnects`; `computer/secrets.rs` `capture_delivery_failure_cannot_echo_values_into_the_desk_log` | Unix for the session tests |
| Decline, expiry, withdrawal and lease revocation store and grant nothing; repeated same-turn requests are refused; existing keys cannot be overwritten or granted for another origin; narrowed jobs, runs and voice fronts gain no computer access | `session/tests/capture.rs` `capture_decline_expiry_withdrawal_and_revocation_store_nothing`, `capture_existing_key_is_offered_only_for_its_origin_and_cannot_be_replaced`, `capture_cannot_widen_a_job_without_a_computer_or_a_voice_front_or_run` | Unix |
| A captured passkey requires the specific operator-approved request and origin, preserves the chosen name and Ask me setting, declines with a denied approval, and expires with unused arming | `session/tests/capture.rs` `capture_passkey_keeps_approval_origin_name_and_ask_setting`, `capture_passkey_unused_arming_expires_and_call_card_stays_on_the_desk`, `capture_passkey_refuses_another_origin_and_minting_without_operator_approval`, `capture_passkey_denial_is_declined_and_tells_the_agent_why_it_stopped` | Unix |
| The phone can answer only the bound capture command; both seats see one settlement; malformed input never echoes values; a saved request cannot revive its lease after restart | `wire/tests/capture.rs` `capture_phone_answer_settles_both_seats_once_and_other_value_commands_are_refused`, `capture_malformed_values_and_debug_never_echo_typed_values`; `tests/desk/capture.rs` `capture_restart_refuses_saved_cards_and_never_stores_submitted_values` | — |
| A call announces the card once, accepts neither yes nor done, and announces storage only by the key's name | `voice/room/tests/cards.rs` `credential_cards_are_said_once_on_a_call_and_spoken_yes_or_done_never_answers` | — |
| Save sends one command and clears secure fields; decline, dismissal, remote settlement and local expiry clear without sending values; passkeys have no value fields | `ui/tests/asks-credential.test.tsx` | — |


## Approval for a stored key's use (task-267)

- **Default and grant source:** a login or passkey is Automatic unless the
  person selects Ask me in Settings or the capture card. Old records are
  Automatic. Variables have no use policy on the computer wire. The existing
  per-teammate secret grant remains necessary. The phone may answer a held
  use but cannot change a key's standing policy.
- **Enforcement:** the computer holds uses and owns the ten-minute allowance
  for one key/site on its per-teammate door. The core polls only granted Ask me
  keys, binds each card to the running computer and capability lease, and
  serializes polling, deliveries and answers. The real answer handler checks
  the current grant, deadline, lease and computer under the policy lock.
  Unknown, wrong-teammate, already-settled and revoked requests cannot approve.
  A 409 or disappearance expires the shared card. Voice has no answering
  authority. Saved cards expire at desk startup and cannot reconstruct a watch.
- **Residual risk:** a trusted computer supplies site/path metadata and enforces
  actual browser use; the desk does not independently inspect the page. An
  older computer returning 404 explicitly receives Automatic policy and logs
  that compatibility limitation once. Network errors leave a card pending only
  until its deadline and do not become approvals. Actual browser holds and
  allow-window enforcement are tested by task-268 in the computer repository;
  this tree tests the agreed door with a fake computer.

| Must hold | Proof | Needs |
| --- | --- | --- |
| One shared card and one answer across desk and phone; a page disappearing expires both seats; decline and lapse reach the agent | `wire/tests/secret_uses.rs` `ask_me_use_has_one_card_on_desk_and_phone_and_one_answer_reaches_the_computer`, `denying_a_secret_use_tells_the_agent_the_person_declined`, `a_page_that_stops_waiting_expires_both_seats_and_tells_the_agent_it_lapsed` | Unix |
| Allow is key/site/computer specific and lapses; Automatic raises no card; a policy change reaches a running computer | `wire/tests/secret_uses.rs` `allow_ten_minutes_is_one_key_one_site_one_teammate_and_lapses`, `automatic_keys_are_not_held_and_policy_changes_reach_the_next_use_without_restart` | Unix |
| Wrong, revoked, unknown and expired requests cannot approve; old computers receive Automatic without changing the vault | `wire/tests/secret_uses.rs` `use_answers_refuse_unknown_wrong_teammate_revoked_and_page_expired_requests`, `an_older_computer_gets_automatic_keys_without_changing_the_vault_choice` | Unix |
| Policy persists without rewriting values; old records are Automatic; saved cards cannot answer after restart | `vault/shared/tests.rs` `secret_use_changes_survive_restart_without_rewriting_values`, `keys_stored_before_use_policy_are_automatic`; `tests/desk/secret_uses.rs` `secret_use_policy_survives_a_real_desk_restart_and_saved_cards_cannot_answer` | — |
| Voice cannot answer a secret-use card | `voice/room/tests/cards.rs` `the_card_handler_refuses_actions_forged_questions_and_stale_authority` | — |
| Settings saves each choice; card buttons send the three exact answers once and disappear on settlement or expiry | `ui/tests/secret-use.test.tsx` | — |

## Host-cookie offers

- **Default:** no automatic lending, including old records. An enabled, running
  computer and live teammate session permit host-column discovery only. The
  room's `cookieOffers` defaults to enabled for all registrable sites; an
  explicit list restricts it and an empty list offers none. Off skips even
  host-name discovery. An old computer without `GET /pages` produces no offers.
- **Grant source:** Use on a core-created `cookie_offer`, through the desk,
  owner or phone seat. The source, site and recipient are held in core memory;
  callers supply only the card id and choice. No tool or voice answer grants it.
- **Enforcement:** `computer/cookies/lending.rs` copies SQLite stores and selects
  only hosts before consent. Its value reader runs only in
  `session/cookie_offers.rs::answer_cookie_offer`, under the computer watch and
  policy locks, with a live capability lease and matching running endpoint
  checked before and after the read. PSL registrable-domain equality, including
  private suffixes, limits values to the approved site and its subdomains. Error
  bodies from the browser and computer are replaced by fixed messages.
  Settings writes and automatic discovery share the policy gate with answers.
  Desk and owner may write the setting, list host sites on explicit page open,
  and undo Never. The phone may read only the preference through
  `computer.cookies.settings`; it cannot discover sites or change settings.
- **Revocation:** a stop, another computer or a desk restart expires pending
  cards. A restart the person did not ask for (an agent switch, a policy
  reload) keeps them and their answers, bound to the new session's lease once
  it is up; Use still checks that lease and the running endpoint. Not now is
  kept on the tape and quiets that site for that teammate for a day (only
  fewer offers, never a grant). Each lend
  is separately named and its metadata survives restart; Remove sends exactly
  `domains: [site]` to the authenticated computer's `DELETE /logins/{name}`.
  Both the browser and saved login there lose the cookies. The agent hears the
  outcome without any values. Never is per teammate and Settings → Computer →
  Sites can undo it. Turning offers off or excluding a site expires its pending
  cards before acknowledging the settings write, with a notice. An already
  started discovery finishes before Off is saved; subsequent automatic polls
  cannot read browser data. Explicit Sites page discovery remains available
  while off, without lending values.
- **Residual risk:** approval gives the teammate the person's signed-in powers
  on this site. The computer is already a granted sandbox and can read its own
  saved login. Removing the container copy does not invalidate an exfiltrated
  cookie or revoke the site's token, and cannot undo actions already taken.
  The removal record is saved before delivery, with an Unconfirmed state until
  success. A failed rollback after an interrupted delivery may require Remove;
  container/network failure cannot prove remote deletion. The host browser may
  ask its OS keychain for permission after Use. Physical Brave/Porkbun and the
  native phone UI require device verification outside this tree.

| Boundary | Proof | Platform |
| --- | --- | --- |
| Discovery selects only hosts from Chromium and Firefox fixtures; site bounds include private suffixes | `computer/cookies/lending/tests.rs` `detection_reads_chromium_and_firefox_host_columns_without_launch_or_keychain`, `discovery_never_launches_or_calls_the_browser_keychain_until_use`, `only_the_registrable_site_and_its_subdomains_can_be_lent` | — |
| Refused installed browsers raise one notice each per desk run, never an offer; restored access retries the open page, readable browsers still offer, and Never sites stay quiet | `computer/cookies/lending/tests.rs` `discovery_keeps_refused_installed_browsers_and_finds_their_profiles_after_access_is_granted`; `wire/tests/cookie_offers.rs` `refused_browsers_raise_one_notice_each_per_desk_run_and_no_offer`, `a_refused_site_is_offered_after_access_appears_without_reopening_the_page`, `a_granted_browser_offers_once_even_when_another_browser_is_refused`, `a_refused_browser_does_not_raise_a_notice_for_a_never_site` | Unix, unprivileged runner |
| One offer reaches desk and phone only for a matching site; racing answers lend once, and Remove names the saved login and site and tells the agent | `wire/tests/cookie_offers.rs` `cookie_offer_is_one_card_on_desk_and_phone_only_for_sites_in_chromium_or_firefox`, `use_lends_only_the_site_and_subdomains_and_remove_takes_it_back_and_tells_the_agent` | Unix |
| Not now quiets a site for that teammate for a day, through switches, stops and restarts; persistent per-teammate Never and Settings undo; unknown, wrong and revoked ids cannot read values | `wire/tests/cookie_offers.rs` `not_now_quiets_a_site_for_a_day_but_never_waits_until_settings_undo`, `a_restart_the_person_did_not_ask_for_keeps_the_card_and_its_answer`, `cookie_offer_refuses_unknown_wrong_teammate_and_revoked_cards_before_reading_values` | Unix |
| Values stay out of public sinks; a real desk restart expires pending offers and retains removal and preference records | `wire/tests/cookie_offers.rs` `cookie_values_never_reach_tape_room_agent_or_wire_sinks`, `failed_delivery_keeps_a_removal_handle_and_never_echoes_a_cookie_in_errors`; `tests/desk/cookie_offers.rs` `cookie_offers_expire_on_real_desk_restart_while_lends_and_per_teammate_never_survive` | Unix for the sink |
| Off makes no discovery calls and expires pending cards; selected sites alone are eligible; absent lists allow every site and empty lists none | `wire/tests/cookie_offers.rs` `cookie_offers_off_never_discovers_and_expires_pending_cards_with_an_explanation`, `cookie_offers_selected_sites_only_offer_matches_and_expire_deselected_cards`, `cookie_offers_all_sites_offer_any_discovered_site_and_an_empty_list_offers_none` | Unix |
| Desk and owner write; phone only reads the preference; host-name discovery is explicit, and invalid domains/settings cannot widen offers | `wire/tests/cookie_offers.rs` `cookie_offer_settings_are_owner_written_phone_read_and_site_discovery_is_explicit`, `cookie_offer_settings_refuse_invalid_domains_and_shapes_without_changing_the_setting` | Unix |
| The preference survives real desk restarts; clearing restores enabled/all sites | `tests/desk/cookie_offers.rs` `cookie_offer_settings_survive_real_desk_restarts_and_clearing_restores_all_sites` | — |

## Accounts: more than one of any provider (epic-25)

An account is a provider credential: a pasted key, a provider sign-in, or a
keyless server URL, each with its own id, its own name and its own secret in
the vault (`vault/secrets.json` for keys, `vault/logins/<id>/` for sign-ins,
the base URL for a local server). Any provider can have several. Nothing
about an account's secret changes: what a second account adds is a second
secret in the same places, never a copy of the first.

- **Choosing.** A model id names its account when its provider has two or
  more: `provider#<credential id>/model`. With one, it is the plain
  `provider/model` it always was, which runs on the provider's default
  account (the first created that still works), so a room from before
  accounts behaves exactly as it did. `models.list` gives every choice its
  account's id, name, kind and whether the provider has others; never a
  secret. The phone sees the same list through the seat it already had.
- **Running.** Every turn, retry, chapter note, call on the teammate's model
  and picture the teammate asks for resolves the account its model names, and
  only that account (`session::account_keys`, `Vault::account_auth`). The
  account's auth stands in for its provider's for that turn, so everything
  the turn asks of that provider goes to that account.
- **No fallback.** A teammate whose named account is revoked, deleted, or
  has lost its secret does not run: its turn fails with a notice saying the
  account is no longer connected and that it will not move on its own. It is
  never run on the provider's other account unless the person picks it. An
  account id belonging to a different provider is refused the same way.
- **Switching.** A switch made while a turn runs lands on the next turn: the
  running turn resolved its account when it started. A move to another
  account is a new history origin, so one account's opaque reasoning is not
  handed to another's.
- **Room-level work.** Voice and pictures with no teammate behind them use
  each provider's default account, as before.

| Boundary | Proof |
| --- | --- |
| One account keeps plain ids; two list each model once per account, ids naming their account, in creation order; revoking one returns to plain ids | `vault.rs` `two_accounts_of_a_provider_list_its_models_once_each_by_account` |
| An account answers only by its own id; revoked, deleted and unknown ids are nothing | `vault.rs` `each_account_answers_by_its_own_id_and_never_for_another` |
| A plain id runs on the default account; a named one on that account for the whole turn; a gone or foreign account is refused | `session/tests/accounts.rs` |
| Over the wire: two accounts list apart with no secret in a choice; a teammate on a revoked account does not run and is not moved | `tests/desk/accounts.rs` `two_accounts_list_their_models_apart_and_a_teammate_on_a_revoked_one_does_not_run` |

### Switching a teammate's agent (task-287)

`session.set_backend` moves a teammate to another agent (Hotline Agent or an
installed harness), with a model when one is named. It is an owner action on
the desk and on the owner phone, the same reach as `session.set_model`; a
harness that is not installed or not signed in is refused with what is
missing, and the phone sees only that it is not ready (`backends.list`
already hides the reason from it).

- The record changes at once; a turn in progress is not cut short. The live
  session restarts on the new agent only when the teammate is idle (waiting
  at most six hours, after which the record takes effect on the next start).
- The new agent's saved session is withdrawn before it starts, so a teammate
  that goes A→B→A starts fresh on A, seeded from the chapter like any new
  session, and never reopens a stale session of A or reports it Restored.
- A Hotline Agent model named in the switch must be one the desk lists, and
  runs on the account it names, as in Accounts above.

| Boundary | Proof |
| --- | --- |
| An idle teammate moves at once; the agent's old session is withdrawn; nothing is cut short | `wire/tests/models.rs` `session_set_backend_moves_an_idle_teammate_and_withdraws_the_agents_old_session` |
| A busy teammate restarts on the new agent only once idle | `wire/tests/models.rs` `session_set_backend_waits_for_a_busy_teammate_to_finish_its_turn` |
| An agent not ready, or not there, is refused and nothing changes | `wire/tests/models.rs` `session_set_backend_refuses_an_agent_that_is_not_ready_or_not_there` |

The picker lists each agent's models before a teammate runs on it. The
list is the one the agent last reported here (`cache/acp-models.json`): a
teammate's session records it as it opens, and an agent that has answered on
this machine before, with no list from the last day, gets one quiet look
(`driver/acp/models.rs`). The look starts the agent's own launch in an empty
folder of its own (`cache/acp-look/<id>`, a plain id only), offers no file,
terminal or auth capability, opens a session naming no MCP servers, reads the
model picker and kills the process group. It never sends a prompt, so it
spends nothing and runs no tool; one look at a time, bounded at a minute, and
a failed look waits a day like a good one. The agent still reads its own
configuration and sign-in as any start does.

| Boundary | Proof |
| --- | --- |
| A look offers nothing, names no servers, never prompts, and reads only the model picker | `driver/acp/models.rs` `a_look_offers_nothing_and_reads_the_model_picker_without_a_prompt` |
| Only a plain id names a look folder; an empty report keeps the last list | `driver/acp/models.rs` `only_a_plain_id_names_a_look_folder`, `an_agents_reported_models_are_kept_and_an_empty_report_keeps_them` |
