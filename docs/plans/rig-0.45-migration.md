# Rig 0.42 → 0.45 migration for Hotline

Landed for Hotline 0.42.0, 2026-10-20. These are Rig version numbers; the
Hotline release is still 0.42.0. The upgrade is **one jump from Rig 0.42 to
`=0.45.0`**, with Rust 1.97.1. There is no intermediate 0.43 or 0.44 install
or profile conversion for the person to run.

## What landed

| Slice | Result |
| --- | --- |
| S0, task-230 | Narrowed the usage, stream-event and request seams on 0.42; pinned provider bodies, cache accounting, context thresholds and failure classification. |
| S1, task-231 | Ported directly to 0.45, including the report's S2 and S3. Native provider clients, completion models, streamed events, request constructors, shared authenticators and Hotline's login-directory `TokenStore` landed together. S2 and S3 were retired. |
| S4, task-234 | Put ChatGPT and Copilot token records behind `CredentialFile`, using the vault's selected backend, with blocking OS-store work off the async workers. |
| S6, task-236 | One voice hook reserves and settles by dispatch `EffectId`, for unary and streamed requests. |
| S7, task-237 | Removed Copilot's chat-content flattening and the whole `compat.rs` transport. Native Rig decoding covers the loose Responses fixtures. |
| S8, task-238 | Added `tests/rig_upgrade.rs`: a legacy profile, paid cached turns and voice calls through a real core and WebSocket, against local provider stubs. |

The generated TypeScript contract, tape event shapes and segment layout do
not change. Hotline still owns the turn loop, history, tools, steering,
chapters and restart policy. Rig constructs provider requests, decodes
responses, signs in and refreshes credentials.

### Existing sign-ins

A ChatGPT login keeps the name `auth.json`, so Rig's new store interface
reads the existing 0.42 record. On first use `CredentialFile` writes its
contents into the selected vault backend, verifies them, then replaces the
plaintext with an opaque reference. Every caller of that login shares an
authenticator: turns, discovery, image generation and voice use the same
refresh state. Per-record store locks serialize independent authenticators
over that directory too.

Copilot starts from 0.42's `access-token` and `api-key.json`. S1 introduced
the lazy conversion to a single `copilot.json` record; S4 changed that
conversion's destination to `CredentialFile`. The final combined path
therefore performs both conversions on first use, writing the combined
record directly into the vault, without an intermediate plaintext file.
It preserves the API token, expiry, endpoints and bootstrap fingerprint,
adds the GitHub token, and removes the two old files only after the write
succeeds. An already-existing plaintext `copilot.json` also migrates on
first read. Migration is lazy, not the eager startup sweep the report
proposed.

A locked or failing backend preserves the plaintext and refuses its use;
`AuthError::Store` survives HTTP error wrapping and offers unlock/retry,
not a new sign-in. Delete or abandoned sign-in cleanup removes both records'
chunks. A desk explicitly using `--store file` keeps plaintext in that
selected backend. Unused or failed legacy migrations still leave their
original plaintext until successfully used.

The downgrade warning is in [the 0.42.0 release notes](../releases/0.42.0.md):
an older Hotline cannot read the reference and requires sign-in again.
Token deletion does not revoke the provider's token, and already cached
authenticator tokens may remain usable until expiry. S5's proposed remote
revocation and cache invalidation were not adopted in this migration; they
remain separate lifecycle work, not a guarantee made by the vault move.

### Requests, usage and voice

The system prompt is prepended to every round, including a continuation
following a tool result or context recovery. Anthropic uses the request's
`.cache(CacheRetention::Short)`. OpenRouter applies that only to
`anthropic/` models and also marks Claude's system prompt in
`Outgoing::mended`; the S0 body pins require both markers. The report's
suggestion that the top-level cache option alone replaces all markers was
insufficient.

Rig 0.45 includes Anthropic's cache tokens in input usage. Hotline deleted
its old Anthropic fold: input includes cached input once, cache reads and
writes remain separate counters for pricing, and context occupancy includes
them. An all-zero usage remains unreported, preserving a reservation as in
0.42, even though Rig considers present zero-valued fields reported.

Every request accepts unknown finish reasons as an ordinary stop
(decision-3), including voice and one-shot requests. Turn-loop unit fixtures
use Rig's `MockScript` wire. The old `compat.rs` decode probe and transport
are deleted; a two-round loose Responses regression proves native decoding
keeps tool replay working. Copilot uses the shared dialect with
`with_route(Responses)` when its discovery record says a model is
Responses-only; the chat assistant-content flattening is gone.

ChatGPT and Copilot use per-request signing in `providers/outgoing.rs`, not
`SignedInModel`. Hotline discovers arbitrary model IDs and does not require
Rig catalog `ModelSpec` rows for them. The transport refreshes before each
request and attaches the current bearer and ChatGPT account header.

Voice reserves in `on_dispatch` and settles in `on_outcome`, indexed by
`EffectId`. This corrects the report's older hook names and the single
reservation slot: each dispatched completion, including a retry, owns its
charge. Settlement removes the matching reservation, so a repeated outcome
cannot settle it again. Failed, missing/all-zero-usage and interrupted
requests retain their estimates. A consumer that drops a stream need not
run an async outcome hook at all; the already-persisted reservation still
covers it. Exhaustion stops the next dispatch and preserves a reply already
paid for. Tool dispatches do not reserve model charges.

## What was not adopted

- Stepwise 0.43/0.44 bumps: they would port usage, assistant messages,
  dynamic tools and auth twice. S0 pins made a single checked jump possible.
- Rig persistence, `AgentRun`, resumable runs and committed-history streaming:
  Hotline injects steering and job results between rounds and owns its
  append-only tape. Rig's at-least-once replay of unanswered tools conflicts
  with Hotline's rule against automatically repeating uncertain side effects
  after restart. Its changing persistence formats are not Hotline's tape.
- Rig's compaction implementation: the upstream stop-during-summary change
  concerns its harness; Hotline's chapters remain product behavior.
- Typed reasoning options: S7 retained `effort_params`. Replacing it would
  change mapping and support gating, including Gemini's `thinkingLevel`
  spelling and lowercase values; the optional cleanup was not equivalent.
- A pairing debug assertion: it removed no code and was not needed by the
  existing pairing and recovery proofs.
- Browser sign-in and remote provider revocation: the existing device flows
  remain; neither is required to preserve current sign-ins.

## Automated proof and review limits

`cargo test -p hotline-core --test integration rig_upgrade::` runs under `make verify`
and `make check`. Its `rig_upgrade_` tests use temporary data directories
passed directly to the desk and a memory vault backend. They do not read or
change the operator's `HOTLINE_DATA_DIR` or keychain. A `test-support` feature,
enabled by the core's dev dependency only, scopes fixed-origin overrides to
unique fixture credentials and loopback addresses. Shipping builds have no
such override; model construction, signing, tools, metering and the wire
remain the real implementations.

The profile proof starts with plaintext ChatGPT `auth.json` and Copilot's
two historical files, runs ChatGPT Responses and both Copilot routes without
a device flow, verifies the signed requests, reads the records back from
the test vault, and scans the temporary profile for leftover tokens.

The Anthropic proof uses S0's exact usage and Haiku's matching prices:
19,459 input, 12 output, 19,471 total, 9,702 cache-read and 9,742 cache-write
tokens across two rounds. The price is $0.0132227, with each token priced
once. The full desk's tool definitions exceed S0's tiny 7,904-token threshold, so
the wire context proof scales its usage, 12,000-token limit and 4,096-token
output allowance tenfold: 97,120 tokens after round one exceed the 79,040
threshold. The next request carries the committed tool result in a fresh
continuation; the original unscaled `rig_pin_` context test remains unchanged. Every round retains the system prompt.

The voice proof drives a text call through the wire and the real provider
dispatcher, checks its settled charge, then checks that a provider stream
closing before final usage retains its reservation. S6's `voice_meter_`
unit proofs also cover retries, consumer-side stream drop, all-zero usage,
provider failures and refusal before the next dispatch. Stubs cannot prove
real provider compatibility or macOS keychain prompts; those checks are below.

## George's checklist before cutting Hotline 0.42.0

Use a disposable copy of a profile signed in on 0.42, preserve an untouched
copy for comparison, and record each result against the release candidate.
The automated harness is not a substitute for these real-provider checks.

- [ ] Anthropic API key: one real turn, then a multi-round tool turn. Round 2
  shows `cache_read > 0`; every request retains the system prompt. Compare
  budget and context counts with the same transcript on Rig 0.42.
- [ ] OpenAI API key: one real turn.
- [ ] Existing ChatGPT 0.42 sign-in: a Responses turn without signing in
  again; image generation; a GPT-Live call. Exercise fresh ChatGPT device
  sign-in separately.
- [ ] Existing Copilot 0.42 sign-in: one chat-route model and one
  Responses-only model without signing in again. Exercise a fresh Copilot
  device sign-in separately.
- [ ] OpenRouter: `anthropic/` with caching, and `openai/`.
- [ ] Gemini: a turn with an effort selected.
- [ ] Ollama local: a turn with effort (`think`) selected.
- [ ] Custom server: one Responses turn and one Chat Completions turn,
  including a tool round on each route.
- [ ] xAI subscription and xAI API key: a real turn on each.
- [ ] z.ai: a real turn with its API key.
- [ ] Voice: compare call-assistant metering with Rig 0.42 numbers; complete
  a call, interrupt a streaming reply, and confirm the next request is
  refused when the budget is exhausted without losing an already paid reply.
- [ ] macOS keychain migration: start the copied signed-in profile and use
  both providers. Confirm `auth.json` and `copilot.json` are references,
  Keychain contains their entries, Copilot's `access-token` and `api-key.json`
  are removed, and neither login needs re-sign-in. Include a profile with
  an existing plaintext `copilot.json` from the intermediate S1 layout.
- [ ] Locked-keychain recovery: refusal asks to unlock/retry, preserves the
  legacy files, and succeeds after unlocking. Delete a disposable login and
  verify its chunks disappear. Remember deletion does not revoke the token
  at the provider and does not promise immediate cached-token invalidation.
- [ ] Confirm the release's downgrade note: after migration an older Hotline
  requires sign-in again; `--store file` remains an explicit plaintext choice.
