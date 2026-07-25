# hotline protocol v2 — FROZEN CONTRACT (P0)

Status: **FROZEN 2026-07-11.** Changes require the architect reopening P0; builders build
against this exactly. It was derived from a rev-2 ground-up design document dated
2026-07-11 that is not published; this spec is the authoritative contract.

**P0 amendment 2026-07-12 (additive, backward-compatible): push-presence.** Adds the app→CLI
`presence` control frame (§3.9) and a 60 s box-side foreground lease so a backgrounded app stops
suppressing push. Old boxes ignore unknown steady-state frames and old apps never send `presence`,
so the frame is additive — **deploy the box before the app.** Push semantics (§4) now key off leased
foreground presence instead of raw subscriber existence.

**FB64 amendment 2026-07-18 (additive, backward-compatible): live-activity registration.** Adds the
authenticated post-hello `live_activity_token` control (§3.10). It is independent of generic push:
old boxes ignore it, and boxes without a complete direct-APNs configuration keep every existing
Expo, gateway, and core path unchanged.

**SDK-settings amendment 2026-07-19 (additive, backward-compatible): remote model/effort for
claude-sdk boxes.** Adds `set_sdk_config` to the nested text-payload control family (§6) and the
transient per-device `sdk_config_result` frame (documented beside the §3.2 identity metadata;
fixture `sdk-config.json`). A claude-sdk box validates with the same caps as the welcome-metadata
fields, persists `HOTLINE_SDK_MODEL`/`HOTLINE_SDK_EFFORT` into the shared state-dir `.env`, and
bounces the harness via the existing supervisor restart control; the post-restart `welcome`/
`agent_state` identity is the apply confirmation (no new confirm frame). Non-sdk harnesses refuse
with `unsupported_harness`. Old apps never send it; on an old box the ride degrades to a literal
JSON text message (the family's standing fail-open). Caveat, by design: a real-environment
`HOTLINE_SDK_MODEL`/`_EFFORT` exported by the shell that launched `hotline up` shadows the `.env`
edit (mergedEnv is real-env-wins) — clients surface the persistent post-restart mismatch.

**SDK hot-model amendment 2026-07-19 (additive, backward-compatible): model changes apply to the
live SDK session — no restart.** On a hot-capable claude-sdk box a MODEL-ONLY `set_sdk_config`
skips the supervisor bounce: the box forwards it to the harness over an internal box↔harness
stdio notification pair (`notifications/hotline/sdk_apply` / `…/sdk_apply_result` — run-child
stdio only, never app wire), the harness validates the id against the CLI's live model list and
applies the SDK's `setModel` control to the running session, and the box persists to `.env` and
answers `ok+restart:false+hot:true` only after that confirmation (persist-after-ok). Confirmation
closes over the next `agent_state` snapshot carrying the restamped model — no welcome, the wire
never drops. `sdk_config_result` gains the optional `hot:true` marker and the failure codes
`unknown_model | apply_failed | no_session | harness_unreachable` (§3.2). Effort stays
Options-bound at `query()` start and keeps the restart path, as does a model-only request on a
box without the harness binding — old boxes answer `restart:true` and new apps support both
shapes; old apps drop the unknown `hot` field. Fixture: `sdk-config.json` SC6–SC9.

**Pi hot-apply amendment 2026-07-20 (additive, backward-compatible; NO frame changes): the pi
harness joins the model/effort path.** `set_sdk_config` / `sdk_config_result` and the internal
`sdk_apply` / `sdk_apply_result` stdio pair are unchanged byte-for-byte; only the set of harnesses
that accept them widens from `claude-sdk` to `claude-sdk | pi`. The frame names are a WART kept
deliberately — they were frozen when claude-sdk was the only such harness, and renaming them would
churn the wire and every fixture. Read "sdk" in these names as "the agent's model/effort knobs".

Per-harness differences a client must know:

- **Model charset.** A pi box accepts `/` (its ids are provider-scoped:
  `openai-codex/gpt-5.6-sol`); a claude-sdk box still refuses it. Both stay within 64 chars of
  `[A-Za-z0-9][A-Za-z0-9._:/-]`.
- **Effort grammar.** A pi box takes a THINKING LEVEL only — `off|minimal|low|medium|high|xhigh|max`
  — and refuses the raw-integer `maxThinkingTokens` form a claude-sdk box accepts (pi has no token
  budget knob). Clients should offer `low|medium|high|xhigh|max` on both; `off|minimal` exist so a
  level set in pi's own TUI can be REPORTED, not so the app can send them.
- **Clear (`""`).** On claude-sdk it reverts to the SDK default. On pi it only UNPINS: pi always has
  exactly one live model and one live thinking level, so the box drops the persisted `.env` line and
  the live value keeps being reported. Clients should not offer a "default" row for pi.
- **Scope knob.** `HOTLINE_PI_MODELS` (comma-separated patterns, globs allowed) scopes pi's
  Ctrl+P model cycling via `--models` and is the SAME list the app's model row offers. Operator-
  owned and never written by an app apply: the scope is the MENU, the model is the SELECTION.
- **Persisted keys.** `HOTLINE_PI_MODEL` / `HOTLINE_PI_THINKING` (and a model write retires
  `HOTLINE_PI_PROVIDER`, since the persisted id carries its own provider).
- **Failure codes.** Adds `no_api_key` to the closed set: the id resolved against pi's live registry
  but the box has no credential for its provider. Distinct from `unknown_model` because the fix is
  logging in, not correcting the id.

Two semantics sharpen for BOTH harnesses (claude-sdk frames are unaffected in practice, because it
echoes verbatim):

- **The `model`/`effort` echo on an ok result is what the harness APPLIED, not what was requested.**
  It is still present only for fields the request carried. pi canonicalizes (`gpt-5.6-sol` →
  `openai-codex/gpt-5.6-sol`) and CLAMPS a thinking level to the model's capability, and the box
  persists and echoes the landed value. A client must confirm its pending apply against the echo,
  not its own request, or a canonicalized/clamped apply never matches the live identity.
- **Identity is bidirectional on pi.** The pi extension restamps `harness_info` when the operator
  changes model or thinking level in pi's OWN TUI, so `agent_state` can move with no app request
  outstanding. Clients already treat `agent_state` identity as authoritative; nothing to change.

Caveat, by design (the pi analogue of the real-env-wins note above): `hotline up -- --model X`
passthrough flags beat the `.env` when the supervisor rebuilds pi's argv, so on such a box a hot
apply survives until the next respawn and then reverts. Boxes that want a durable remote knob must
set `HOTLINE_PI_*` in the `.env` and drop the passthrough flags.

## 0. Shape

```
[app] --wss--> [rendezvous: dumb pipe] <--wss-- [hotline CLI]
```

- Both sides DIAL OUT. Nobody listens on a port. The rendezvous pairs exactly two sockets on a
  room key and pipes frames verbatim. It never parses a frame, holds no state beyond the live
  pair, and buffers nothing: **a frame sent while the peer is absent is dropped silently.**
- Durability lives ONLY in the CLI's per-device mailbox. Delivery correctness never depends on
  the pipe or on the socket's lifetime.
- Everything in §3–§6 is CLI↔app **end-to-end**; the rendezvous is bytes-in bytes-out.

## 1. QR pairing URI

```
hotline://pair?v=1&u=<url>&r=<room>&s=<secret>&n=<name>
```

| param | meaning | format |
|---|---|---|
| `v` | pairing-URI version | literal `1` |
| `u` | rendezvous websocket base URL | percent-encoded; `ws://` (dev) or `wss://` (prod); no trailing slash |
| `r` | room id | 22-char base64url (128-bit), `[A-Za-z0-9_-]{22}` |
| `s` | pairing secret | 43-char base64url (256-bit), `[A-Za-z0-9_-]{43}` |
| `n` | display name for the assistant | percent-encoded UTF-8, ≤ 64 chars after decode |

Rules: unknown params MUST be ignored (forward compat). `v` ≠ `1` → the app MUST refuse with a
"newer QR, update the app" message, never a partial pair. The app persists `{u, r, s, n}` as the
pairing record; `s` goes in secure storage (Keychain via expo-secure-store).

The secret is a bearer capability minted by the operator's own CLI (`hotline relay`), one per
pairing. `hotline relay revoke <device>` creates a permanent device ban; rotating a link does not.
Verification is CLI-side (§3.1) — never at the pipe. Pairing is an operator action; nothing in
chat content can grant or approve access.

## 2. Rendezvous semantics (the dumb pipe)

- Endpoints: `GET /r/<room>/c` (CLI side) and `GET /r/<room>/a` (app side); both are WebSocket
  upgrades. `<room>` must match `[A-Za-z0-9_-]{22}` (400 otherwise).
- Exactly ONE live socket per side per room. A new connection on a side closes the previous one
  with code **4001 "replaced"**. (The newest device/CLI wins; the replaced side redials if alive.)
- Piping: any text or binary frame from one side is forwarded verbatim to the other side if
  connected, else DROPPED. No inspection, no transformation, no buffering, no close propagation,
  no presence signals of any kind.
- Limits: max frame 1 MiB (close 1009 on violation); per-IP new-connection rate limit; per-room
  frame-rate soft cap (generous; protects the worker, not the protocol). Room ids are unguessable
  128-bit values — that plus rate limits is the pipe's entire security story.
- Rooms are implicit: the first socket to name a room key creates it; when both sockets are gone
  the room is garbage (nothing to persist). Idle rooms hibernate (CF WebSocket Hibernation API).
- The pipe MUST work identically under `wrangler dev --local` (dev, bound on LAN) and deployed
  (prod). Zero environment-specific behavior.

## 3. Frame envelope (CLI↔app, both directions)

- JSON text frames. Every frame is an object with a string field `t`.
- **Keys are case-sensitive and lowercase.** A receiver MUST match `t` by exact byte comparison
  (Go decoders MUST NOT rely on encoding/json's case-insensitive field match for `t` — this was
  a real spoof bug in v1). Frames whose exact-case `t` is absent or not a string are dropped.
- Unknown `t` values MUST be ignored (forward compat). Unknown fields on known frames MUST be
  ignored.
- All mailbox cursors (`m`, fields `floor`, `head`, `resume_from`, `ack`) are **decimal strings**
  (BigInt-safe above 2^53). Journal seq `j` likewise.

### 3.1 `hello` — app → CLI, first frame on every connect

```json
{"t":"hello","v":2,"device_id":"dev-af31fd290542","secret":"<43-char>",
 "resume_from":"9007199254740995",
 "push":{"token":"ExponentPushToken[...]","platform":"ios"}}
```

- Sent immediately on socket open; nothing else may precede it. `resume_from` is the app's
  cursor: the highest `m` it has applied AND acked (its durable cursor). `"0"` = fresh install.
- `push` is optional (simulator has no token). Sending it (re)registers the token.
- CLI verifies `secret` (constant-time) against the current room capability.
  - unknown/invalid secret → `error{code:"unauthorized"}` then close 4003.
  - permanently banned device → `error{code:"revoked"}` then close 4003.
  - a known, non-banned device presenting the current room secret is re-linked to the current
    room and becomes active, even if its record names an older room or legacy revoked state.
  - if a different device is already active in the current room, the newcomer is unauthorized
    (the room remains 1:1).
- On success the CLI replies `welcome` and immediately drains (§4).

### 3.2 `welcome` — CLI → app

```json
{"t":"welcome","v":2,"room":"<22-char>","name":"pi","device_id":"dev-af31fd290542",
 "floor":"9007199254740993","head":"9007199254741007",
 "harness":"claude-sdk","model":"claude-opus-4-8","effort":"xhigh"}
```

- `floor` = lowest `m` still retained in this device's mailbox; `head` = highest assigned `m`.
- After `welcome`, the CLI streams `mailbox_item`s from `max(resume_from, floor)` exclusive,
  through `head`, then continues live as new items are enqueued.
- `harness` / `model` / `effort` are **optional, additive** box identity metadata; absent =
  unknown (old boxes). `harness` is the kind driving the box — `"claude"` (the TUI),
  `"claude-sdk"`, `"pi"`, or `"opencode"`; `model` is the resolved or configured model id
  (e.g. `"claude-opus-4-8"`), omitted when unknown; `effort` is the operator effort knob
  verbatim (e.g. `"xhigh"`), omitted when unknown or numeric-custom. The same three fields
  ride the transient `agent_state` snapshot (see `docs/app-channel.md`, Agent state) for live
  refresh after the harness resolves its model; `welcome` guarantees an idle box still
  displays correctly on every (re)connect.
- **`sdk_config_result` — CLI → app (SDK-settings amendment 2026-07-19).** A transient content
  frame in the `agent_state`/`typing` family: seq'd, never durable, never acked, never replayed,
  sent **only to the device** whose `set_sdk_config` control (§6) it answers, correlated by
  `rid`. Success — `{"t":"sdk_config_result","seq":N,"rid":"…","ok":true,"restart":true|false}`
  plus an echo of the accepted post-trim values for exactly the fields the request carried
  (`""` echoes as `""` = cleared; absent stays absent); `restart:true` means persisted and a
  supervisor bounce was requested, `restart:false` the no-op case (nothing written or bounced) —
  or, with the hot-apply amendment's `hot:true` marker (serialized only when true), a model
  and/or effort change applied to the LIVE session and persisted after the harness confirmed (SC6/SC7/SC10):
  no bounce, and the apply confirmation is the next `agent_state` snapshot carrying the
  restamped model, no welcome involved. Error — `ok:false` with `code` from the closed set
  `unsupported_harness | invalid_model | invalid_effort | empty_request | persist_failed |
  restart_failed` plus the hot-path additions `unknown_model | apply_failed | no_session |
  harness_unreachable` (SC8: harness refusals forwarded verbatim — session untouched, nothing
  persisted; `harness_unreachable` = forward failed or no confirmation within the box's 10 s
  pending budget; `persist_failed` after a live apply carries the applied-but-unsaved detail)
  and an optional single-line `detail` ≤ 200 chars, plus `apply_in_progress` when another
  model/effort change is still being applied (applies serialize box-side; the rows disable while
  one is in flight, so this only reaches a second device). An ok hot result may additionally carry
  `unverified:true` (SC11, serialized only when true): the applied model was syntactically valid but
  ABSENT from the harness's catalog, which under-enumerates live-valid ids — the apply landed, the
  API rather than the catalog is the authority, and a rejection would surface on the NEXT turn and is
  recoverable by switching back. A replayed `rid` — an older app parks this control in its message
  outbox, where nothing can settle it, so it re-flushes on every reconnect — is re-answered from the
  box's settled-rid cache and never re-applied. On the restart path apply confirmation is
  NOT a frame of its own: the post-restart `welcome` / `agent_state` identity carrying the new
  values closes the loop. Fixture: `sdk-config.json`.
- **`agent_catalog` — CLI → app (model catalog amendment 2026-07-20).** A transient content
  frame in the `agent_state`/`typing` family: seq'd, never durable, never acked, never replayed.
  Pushed to a single device on subscribe (after drain, alongside the `agent_state` snapshot) and
  broadcast to all active devices when the catalog changes. Shape —
  `{"t":"agent_catalog","seq":N,"harness":"pi","source":"models","models":[…]}` with each entry
  `{"id":"provider/id","label":"Sol","available":true}`; optional `truncated:true` when the list
  was cut at the box's cap. `id` is the CANONICAL id the app sends back in a `set_sdk_config`
  and the same form the box restamps as the live identity, so a selected row shows as selected;
  `available` is ALWAYS serialized (an omitted `false` would be indistinguishable from a frame
  that never carried the field); `source` is `models | settings | available`, naming which
  precedence branch produced the list.

  **Two tiers, deliberately different sets.** The catalog is the operator's FILTERED scope for
  that box — on pi, the `--models` selection, literally the set Ctrl+P cycles — enumerated live
  by the harness through pi's own scope resolver against the live `ModelRegistry`. The app's
  free-text escape is NOT bounded by it: that goes through `set_sdk_config` to the harness,
  which resolves against the FULL registry (`getAll()`, which deliberately includes models
  without pre-configured auth) and key-checks, answering `no_api_key` when a model resolves but
  the box holds no credential. A catalog narrows the menu, never the reach.

  **Scope ownership.** The scope is box-owned (`HOTLINE_PI_MODELS`, or an explicit
  `-- --models` passthrough, which wins). `hotline up` puts the effective value on pi's launch
  line AND re-exports it into the harness env, because an extension cannot read pi's resolved
  cycling list — `AgentSession.scopedModels` is absent from the `ExtensionContext` /
  `ExtensionAPI` surface — so the box is the only place the launch line, Ctrl+P, and the app can
  be made to agree.

  **Compatibility is by absence.** A box that sends no catalog leaves the app on its curated
  fallback list, exactly as before this amendment; an old app drops the unknown `t` like any
  other unrecognized transient. There is no negotiation and no version flag. A harness reporting
  an EMPTY catalog clears the box's catalog and the app falls back — that is how a re-scoped box
  says "stop showing a list that no longer exists". Fixture: `agent-catalog.json`.

### 3.3 `mailbox_item` — CLI → app

```json
{"t":"mailbox_item","m":"9007199254741001","j":"812","id":"env-j812-daf31fd290542",
 "created_at":"2026-07-17T20:15:23.456789Z",
 "payload":{"t":"msg","seq":812,"id":"a-812","text":"hello from the box"}}
```

- `id` (envelope id) is deterministic per (journal seq, device): `env-j<j>-d<device-suffix>`.
  The app dedups by `id`: an already-applied `id` is acked but not re-applied.
- `created_at` is the optional RFC 3339 timestamp captured once when the journal row was
  appended. Replays and per-device backfills preserve that original value. Records written by
  older boxes omit it; clients must accept both forms.
- `payload` is a **content frame** (§6) — the transport never interprets it beyond JSON validity.

### 3.4 `mailbox_ack` — app → CLI, cumulative

```json
{"t":"mailbox_ack","m":"9007199254741001"}
```

- Means: "I have durably applied everything ≤ m." The app persists its cursor BEFORE sending
  the ack (commit-before-ack — the v1 kill/reopen fix). The CLI may trim ≤ m (subject to §5
  retention). Acks are idempotent and may repeat; the CLI takes the max.
- The app acks at most once per drain batch and then per applied live item (coalescing allowed;
  cadence ≤ 1 ack/500ms is fine).

### 3.5 `mailbox_gap` — CLI → app

```json
{"t":"mailbox_gap","floor":"9007199254741050"}
```

- Sent instead of a drain when `resume_from < floor` (the mailbox no longer retains the app's
  next item — retention trim or mailbox reset).
- App handling (mandatory): clear the transcript projection for this room, set cursor to
  `floor`, send `mailbox_ack{m:floor}`, and render from the drain that follows. No partial
  stitching.

### 3.6 `device_send` — app → CLI

```json
{"t":"device_send","cid":"01J2ZK8Q0X6WYV9R3T5B7N4E2M",
 "payload":{"t":"send","text":"yo"}}
```

- `cid` is a client-generated unique id (ULID/UUID), the idempotency key. The CLI keeps a
  bounded per-device set of seen cids (≥ 1024, FIFO); a duplicate `cid` is accepted silently
  and NOT re-applied (no duplicate journal event, no second echo).
- Effect: the CLI applies the turn to the room journal. Journal events fan back into the
  device's own mailbox as an echo item whose payload is `{"t":"sent","cid":"<same>",...}` —
  the app reconciles its optimistic bubble by `cid` and trims its pending outbox on echo.
  The cid key and exact echo are committed in the same durable journal record. If processing
  crashes before the cid index or mailbox is updated, replay recovers that record and
  re-enqueues the same deterministic echo without applying the turn again.
  **There is no separate send-ack frame; the echo is the ack.**
- Sends never require a live socket: the app queues in its pending outbox and flushes on the
  next connect (cid idempotency makes double-flush harmless).

### 3.7 `ping` / `pong` — app → CLI → app

```json
{"t":"ping","n":7}   →   {"t":"pong","n":7}
```

- App sends `ping` every 25 s while a socket is open and foregrounded; CLI echoes `pong` with
  the same `n`. Two consecutive misses → app closes and redials. The CLI never initiates pings.
- The CLI ALSO uses each `ping` as a foreground liveness signal: a valid ping refreshes the
  sending subscriber's presence lease (§4). This is what lets even an old app (which sends no
  `presence` frame) be detected as away within the lease window once it stops pinging.

### 3.8 `error` — CLI → app

```json
{"t":"error","code":"unauthorized","detail":"optional human string"}
```

Codes (closed set): `unauthorized` · `revoked` · `cursor_ahead` · `full` · `bad_frame`.
- `cursor_ahead`: `resume_from > head` (CLI-side state was reset while the app kept old state).
  App handling: treat like §3.5 with `floor` = "0" — clear projection, re-hello with
  `resume_from:"0"`.
- `full`: the device's mailbox hit its retention cap and the device has not acked (§5). The
  connection stays open; new enqueues for this device are dropped oldest-first from the
  unacked tail is NOT allowed — instead the CLI stops enqueuing and the next hello will gap
  (§3.5) from the advanced floor.
- `bad_frame`: schema-invalid inbound frame; the offending frame is ignored; repeated abuse →
  close 4002.

### 3.9 `presence` — app → CLI (push-presence, P0 amendment 2026-07-12)

```json
{"t":"presence","state":"foreground"}   {"t":"presence","state":"background"}
```

- Valid only AFTER `hello`, on the authenticated session. Identity is the session's subscriber —
  there is **no `device_id`** on the frame.
- `state` is exactly `foreground` or `background`. Unknown fields are ignored. A malformed
  `presence` is a `bad_frame` (counted toward the §3.8 abuse limit).
- Idempotent, last-write-wins per subscriber. It has **no** response, cursor, ack, persistence,
  replay, or mailbox item, and never enters content/transcript parsing.
- App behavior: on background, send `presence:background` **before** the final ack and close
  (earliest queue slot); on foreground with an open socket, send `presence:foreground`
  immediately; on a fresh socket, `hello` marks the replacement subscriber foreground and the app
  reinforces `presence:foreground` **after** hydration (so it never lands in an old box's gap
  path). A failed presence send never blocks the close.
- The CLI accepts `presence` both in the steady-state session and during the `mailbox_gap` ack
  wait, without disturbing the gap ack/drain.

### 3.10 `live_activity_token` — app → CLI (FB64 ActivityKit registration)

```json
{"t":"live_activity_token","job_id":"job-1","token":"aabbccdd"}
```

- Valid only after an authenticated `hello`, including during a `mailbox_gap` ack wait. Device and
  room identity come exclusively from that session; the frame carries no `device_id` or `room`.
- `job_id` matches `^job-[1-9][0-9]*$` and is at most 64 characters. `token` is either empty or
  1–200 lowercase hexadecimal characters with even length (the same APNs-token validation used by
  the box's existing iOS gateway path).
- A non-empty token registers/replaces this device's ActivityKit token for the active job and
  immediately hydrates it with the job's current running state. At most 32 distinct jobs are
  registered per device; adding another evicts the oldest registration.
- `token:""` is an idempotent unregister. A schema-invalid frame is `bad_frame`; a valid frame
  naming an unknown or already-finished job is silently ignored for lifecycle-race tolerance.
- Registrations are box-durable but binding-scoped: room change, rotate/unbind, or device revoke
  clears them. Job updates and terminal completion drive direct APNs `update`/`end` events in
  lifecycle order; terminal completion atomically clears the registrations before enqueueing end.
  This control never creates a mailbox item, transcript event, ack, or generic push.

## 4. Connection lifecycles

- **CLI**: dials `u + "/r/" + room + "/c"` on `hotline relay` start; redials forever on close
  with exponential backoff 1 s → 60 s + jitter, reset after 60 s of stable connection. Startup
  mints a link only when no current room exists; an existing room is preserved even with zero
  active devices. Operators rotate it explicitly with `hotline relay new-link`. The CLI socket
  is long-lived but NOT load-bearing: mailbox enqueue and push happen regardless of it.
  - **hello = session (re)start, at any time (CLARIFICATION).** The rendezvous is dumb: it does
    NOT propagate the app's socket close to the CLI. So the CLI's single `/c` socket outlives
    many app sessions, and a `hello` frame is the ONLY signal that the app (re)connected. The
    CLI MUST treat a `hello` arriving at ANY point on the `/c` socket as a clean session
    restart: re-verify, re-provision/subscribe, re-drain from the new `resume_from` — exactly as
    if it were the first hello. "hello is first-frame-only" (§3.1) constrains the APP (it sends
    hello first on each of ITS connections); it does NOT license the CLI to reject a later hello
    on its persistent pipe socket. A second hello is never a `bad_frame`.
- **App**: opens a socket only while UI is active (foreground, room open or app just launched
  from a push tap); closes it on background without ceremony. Every connect is
  `hello(resume_from)` → drain — **reconnect ≡ cold start; there is no session state to
  restore.** "Connected" is a UI hint only; no user action requires it.
- **Push**: leased foreground presence is sampled for the target device at enqueue time. A
  subscriber is *present* while it is foreground AND its foreground lease is fresh
  (`now − lastForeground < 60 s`; the lease is set on subscribe and refreshed by `presence:
  foreground` and by each `ping`). `presence:background` latches the subscriber background
  immediately, and a later ping/ack does NOT reactivate it. The device is *away* when NO
  subscriber is present (all background, lease-expired, or absent) — and only then does every
  `msg`, `edit`, and `react` item create a push intent. A single foreground-and-fresh subscriber
  suppresses push (the item is delivered live). Unread/ack state does not gate later intents;
  `typing` never creates a push. Push carries no message content beyond title/preview and is
  never a data channel; its `data` carries `url:"hotline://chat"` and, when a current room
  exists, `room:<roomId>` so the app can suppress the in-app banner for the chat already on
  screen. A stale subscriber may remain in `m.subs` for delivery/overflow while still counting as
  away for push; durable replay/dedup remains authoritative.

## 5. Mailbox semantics (CLI-local)

- One mailbox per device per room. `j` (journal) is the room's permanent truth; `m` (mailbox)
  is the per-device delivery sequence, assigned locally, strictly ascending, decimal string.
- Provisioning (first pair): the mailbox is seeded from the tail of the room journal
  (most recent ≤ 200 events) so a new device sees recent history; `floor` starts at the seed.
- Retention: items are trimmed once acked. Unacked items are retained ≥ 7 days and ≤ 10 000
  items per mailbox; beyond that the floor advances (next hello gaps per §3.5).
- Dedup: envelope `id` is deterministic; re-enqueue of the same journal event is a no-op.
- Durable payloads are `msg`, `sent`, `edit`, and `react`. `typing` is transient (§6), consumes
  no mailbox cursor or retention capacity, and is never present in a reconnect drain.
- The engine ports from v1 `internal/app/mailbox.go` — same invariants (F-series), minus the
  remote transport: `onEnqueued` becomes a local assignment, no in-flight window, no
  `mailbox_enqueue/enqueued` wire leg.

## 6. Content payloads (inside `mailbox_item.payload` / `device_send.payload`)

- The content-frame family is **carried over from v1 unchanged**: `msg`, `sent`, `edit`,
  `react`, `buttons`/tap, `typing`, pacing — as fixed by the existing fixtures
  (`protocol/fixtures/*.json`: paced-reply, buttons-tap, react-both-ways, edit, error,
  send-echo-ack for shapes). Those fixtures remain normative for payload shapes. `typing` is the
  sole transient exception: the CLI sends it directly only to a live subscriber, never stores
  it in the journal or mailbox, and never replays it after reconnect.
- Exception — **blobs (photos, artifacts) do NOT use relay blob URLs** (there is no relay).
  v2 transfers blobs in-band, chunked, e2e:

```json
{"t":"blob_begin","xfer":"x-01J2...","mime":"image/jpeg","size":2381921,"chunks":10}
{"t":"blob_chunk","xfer":"x-01J2...","i":0,"data":"<base64 ≤ 256 KiB decoded>"}
{"t":"blob_end","xfer":"x-01J2..."}
```

  Both directions. Receiver reassembles by `xfer`, verifies size, then the referencing content
  frame (e.g. `msg` with `attachment:{xfer:"x-01J2..."}` or `device_send` payload
  `{"t":"send.photo","xfer":...,"name":...,"mime":...,"size":...}` / generic file payload
  `{"t":"send.attachment","xfer":...,"name":...,"mime":...,"size":...}`) resolves against the
  completed transfer. Chunk data ≤ 256 KiB
  decoded keeps every frame safely under the 1 MiB pipe cap. Incomplete transfers expire after
  120 s. Blob frames ride inside the same socket and are NOT mailbox items themselves; the
  referencing content frame is the durable item (a mailbox_item whose blob was never fetched is
  re-requestable via `blob_req{xfer}` → CLI re-sends begin/chunk/end).
- Client uploads support photos and one generic file per `device_send`, each ≤ 50 MiB. A
  successful `sent` echo carries the additive `file` descriptor so the sender, sibling devices,
  and replay all resolve the same persisted transfer.
- **Nested text-payload controls (app → CLI).** The app's connection primitive cannot emit
  top-level websocket frames, so box-directed controls ride as a NORMAL `device_send` whose
  `"send"` text payload is a serialized control object recognized by its inner `t`. A box that
  recognizes the `t` consumes the ride SILENTLY at the inbound boundary — never delivered to
  the harness, never a transcript/journal event, no `sent` echo, no error frame (a recognized
  control with malformed fields is still consumed). On an old box the ride degrades to a
  literal JSON text message reaching the harness — harmless, the family's standing fail-open.
  The family: `set_name` (FB21), `set_push_preview` (FB23), `set_job_completion_push` (FB44),
  and `set_sdk_config` (SDK-settings amendment 2026-07-19; claude-sdk model/effort with
  box-authoritative validation, `.env` persistence, and a supervisor bounce — answered on the
  transient `sdk_config_result` frame, §3.2; fixture `sdk-config.json`. Hot-model amendment:
  model-only requests on hot-capable boxes apply to the live SDK session instead of bouncing,
  answering `hot:true` — persist-after-ok, SC6–SC9).

## 7. Cross-cutting rules

- Secrets (`s`, push tokens) never appear in logs on either side.
- The CLI verifies identity from its own device records, never from client-asserted fields
  beyond `device_id` lookup + secret verify.
- One frozen schema: `protocol/v2/schema/frames-v2.schema.json`. Goldens:
  `protocol/v2/fixtures/*.json` (§8). Gate for every builder: goldens pass byte-exact.
- v1 relay frames (`relay_presence`, `mailbox_enqueue`, raw broadcast, hl-ping) DO NOT EXIST
  in v2 code. There is no compatibility mode.

## 8. Goldens (normative fixtures, this directory)

| fixture | proves |
|---|---|
| `pair-uri.json` | QR URI parse: valid, bad version, missing param, unknown-param tolerance |
| `hello-welcome-drain.json` | fresh + resumed connect, welcome floor/head, drain order, cumulative ack, ping/pong |
| `resume-dedup.json` | reconnect overlap: re-delivered ids acked-not-reapplied, cursor advances |
| `gap-reset.json` | resume below floor → gap → reset → ack floor → clean re-drain |
| `send-echo.json` | device_send cid → journal → echo `sent` reconciles outbox; duplicate cid = no second echo |
| `cursor-ahead-and-errors.json` | resume above head → cursor_ahead reset; unauthorized/revoked/bad_frame behavior |
| `presence.json` | app→CLI foreground/background control frame: no response/cursor/ack/persistence; leased-presence push gate |
| `welcome-metadata.json` | optional additive harness/model/effort identity on welcome + agent_state; sanitize-never-reject; old-box/old-app compat |
| `agent-catalog.json` | transient agent_catalog: the box enumerates its selectable models (pi's --models cycling scope) and the app renders THAT instead of a curated list; canonical ids + availability + truncation; free text still reaches the full registry (no_api_key vs unknown_model); absence = curated fallback (old-box/old-app compat) |
| `sdk-config.json` | set_sdk_config nested control ride + transient per-device sdk_config_result; welcome-metadata validation caps; non-sdk refusal; post-restart identity as apply confirmation; hot-apply amendment: model AND effort live-session apply (hot:true, persist-after-ok, agent_state confirmation, closed-set hot failures); restart is the forwarder-unbound/old-box fallback only; two-tier model guard (catalog-absent ids apply carrying unverified:true) |
