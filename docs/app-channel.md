# The `app` channel

The `app` channel makes hotline host its own transport: a WebSocket server that
a phone client connects to **directly** over your LAN or tailnet. No
Telegram/Signal/Discord platform sits in the middle — the app *is* the client,
and the harness sees the same `<channel source="app">` turns every other
provider produces.

Two clients speak the protocol today: the **built-in `/chat` browser page**
(served by hotline itself — zero install) and the **hotline mobile app**, a
separate closed-source client that ships outside this repository. The channel
itself is an open protocol: anything that speaks it is a first-class client.

## Enable it

The channel is selected via `HOTLINE_PROVIDERS`, like every other provider. It
composes: `telegram,app` runs both at once.

```bash
# In the shared .env (or the real environment):
HOTLINE_PROVIDERS=app
HOTLINE_APP_BIND=172.16.30.90:8990   # a specific LAN/tailnet IP + port
```

Then run the harness as usual (`hotline start`, `hotline up`, …). On boot you'll
see:

```
hotline: provider=app state=…/app bind=172.16.30.90:8990
hotline: app serving WebSocket on ws://172.16.30.90:8990/ws (chat page: http://172.16.30.90:8990/chat)
```

### Bind rules

`HOTLINE_APP_BIND` defaults to `127.0.0.1:8990` (loopback only). A
network-reachable bind must be chosen deliberately:

- A **specific LAN or tailnet IP** (recommended): only that interface is served.
- A **wide-open** bind (`0.0.0.0` / `::`) is **refused** unless you also set
  `HOTLINE_APP_ALLOW_ANY=1`. A network-listening server must be opted into
  explicitly.

The listener itself is the single-consumer guard: a second hotline process on
the same bind gets `EADDRINUSE`.

## Optional outbound relay

The app channel can also dial a transparent WebSocket relay while keeping its
LAN listener available:

```bash
HOTLINE_APP_RELAY=wss://<relay-host>/b/<16-lowercase-hex-slug>
HOTLINE_APP_RELAY_TOKEN=<relay-bot-token>
```

The connector accepts exactly `ws(s)://host/b/<16-lowercase-hex-slug>`. It
sends the relay token as `Authorization: Bearer …`, reconnects forever with
jittered backoff, and stops with the app server's context. Relay failures never
stop LAN serving. `wss://` derives a fixed `https://host` blob origin; `ws://`
is for local development only because the token is plaintext on the wire.
Named app instances use the existing uppercased suffix convention, for example
`HOTLINE_APP_RELAY_WORK` and `HOTLINE_APP_RELAY_TOKEN_WORK`.

The relay token authenticates only the box-to-relay transport. Relayed device
frames still enter the normal `hello` secret, pairing, allowlist, replay, and
handler paths; the relay grants no device trust.

The transparent relay combines all device traffic for a bot onto one box-side
socket, while the current app hub binds one device identity to each socket.
This walking skeleton therefore supports **one relay device at a time**. The
first `hello` owns the relay socket; a later `hello` receives an `unauthorized`
error and the box reconnects rather than attributing another device's frames to
the first. Correct multi-device support requires a relay envelope or per-device
socket identity.

The relay does not tunnel the box's direct HTTP server. Relay photos instead
use the bounded `/blob/<slug>/…` plane: device uploads are materialized into the
box's local attachment store before the harness sees them, and bot images carry
dual refs (`/att/<id>` for direct clients plus a relay `blob` capability).
Relay `publish` uploads one self-contained HTML file and journals one artifact
message. `/history` and `/push-token` remain disabled in relay mode.

## Pair a device

Auth reuses hotline's shared access model. On first launch a client generates a
32-byte secret (stored in the browser's `localStorage`, or the app's storage);
the server derives `device_id = "dev-" + sha256(secret)[:12]` — that id is the
allowlist entry, exactly like an E.164 number is for Signal. **The secret rides
the first WebSocket frame (`hello`), never a URL query.**

1. Open the client. An unknown device is answered with a **pairing code** and
   shown the exact command.
2. On the box, out-of-band, the operator runs:
   ```bash
   hotline pair <code> --provider app
   ```
3. The client reconnects (its backoff is already running) and is welcomed.

Inspect and manage state with the usual commands:

```bash
hotline status --provider app          # bind, auth mode, allowlist, pending
hotline revoke <device-id> --provider app
```

### Token fallback

If wiring the pairing UX end-to-end is inconvenient, set `HOTLINE_APP_TOKEN` to
a shared secret. Any client presenting that exact token is allowed, bypassing
pairing entirely. Leave it unset to use real pairing (the default and
recommended mode).

## Open the chat page

Point a phone (or any) browser at:

```
http://172.16.30.90:8990/chat
```

It's a single self-contained page — bubble UI, typing indicator, connection
status, reconnect-with-backoff — that speaks the same v1 protocol as the Expo
app. It's the fastest way to prove the round-trip end to end from the phone.

## Security posture (tonight)

Transport security for the **direct listener** is the network: bind to a LAN
you trust, or a tailnet / WireGuard interface where the path is already
encrypted and authenticated. The direct listener has no TLS yet — `ws://` is
plaintext, so the hello secret transits in clear off-tailnet. Do not expose the
bind to an untrusted network. Direct-listener `wss://` (a self-signed cert pinned
at pairing time) is the fast-follow; the outbound connector already supports
relay `wss://`. The `0.0.0.0` refusal above is the hard guardrail against an
accidental wide-open listen.

The server never executes anything from the app; it only relays text into the
same gated `<channel>` envelope every other provider uses. Approving a pairing
is strictly an out-of-band operator action (`hotline pair`) — a chat message can
never grant itself access.

## Wire protocol (v1)

Versioned JSON text frames, one object per frame, keyed by `t` (type). Both
sides ignore unknown frame types and unknown fields.

**App → server**

| `t` | fields |
|-----|--------|
| `hello` | `v` (=1), `secret`, `device_name`, `last_seq` — always the first frame |
| `send` | `cid`, `text`, `reply_to?`, `attachments?`, `cids?`; a relay photo attachment adds `mime`, `size`, and `blob` |
| `tap` | `cid`, `msg_id`, `label` |
| `react` | `cid`, `msg_id`, `emoji` |

**Server → app** (every content frame carries a monotonic `seq`, the replay cursor)

| `t` | fields |
|-----|--------|
| `welcome` | `v`, `seq`, `device_id`, `bot_name` |
| `pairing` | `code`, `hint` |
| `error` | `code`, `message` |
| `ack` | `cid`, `seq` |
| `typing` | `seq`, `on` |
| `msg` | `seq`, `id`, `text`, `buttons?`, `reply_to?`, `file?`, `artifact?`; relay-backed files add `file.blob` |
| `sent` | `seq`, `id`, `text`, `cid?`, `device`, `kind?`, `file?`; a materialized relay photo uses the same dual file ref |
| `edit` | `seq`, `id`, `text` |
| `react` | `seq`, `msg_id`, `emoji` |

On reconnect the client sends its highest-seen `seq` as `hello.last_seq`; the
server replays every buffered frame after it.

## Durable history, attachments, and push registration

The server appends every sequenced frame to `<StateDir>/outbox.jsonl`, restores
the sequence and hot replay ring after restart, and keeps the file as the full
thread journal. The in-memory ring remains a roughly 200-frame reconnect window;
clients page older records through the history endpoint.

These HTTP endpoints use the same device secret as `hello`, sent in the
`X-Hotline-Secret` header:

- `GET /history?before=<seq>&limit=<n>` returns an oldest-first page of journal
  frames.
- `POST /upload` stores a multipart attachment, relays it to the harness, and
  journals its `sent` echo.
- `GET /att/<name>` serves a stored attachment; agent reply files use the same
  store and appear as file bubbles.
- `POST` or `PUT /push-token` registers or replaces the calling device's Expo
  token; `DELETE /push-token` unregisters it.

## Elements (protocol v2)

`msg` and `edit` frames may carry an optional `elements: Element[]` — typed
inline UI the app renders as native components under the message text (like the
existing `buttons` row). Old-client compatibility rides `text` (E6): for an
element-only message or edit the box synthesizes `text` = the elements'
`fallback` strings joined by `" · "` — an old client renders that text; a new
client that successfully parses the frame's elements suppresses the text iff it
exactly equals that synthesized join (deterministic on both sides). New clients
on an old box simply never receive elements. Max **4 elements per message**;
each element ≤ **2 KiB** serialized; the **complete serialized msg/edit
payload** (text + elements + all fields) ≤ **16 KiB**, validated before
enqueue (E9). The box validates at the tool layer and returns a precise error
(which element index, which rule) rather than emitting a mangled element.

```jsonc
Element = {
  "el": "chip" | "job" | "decision" | "approval" | "checklist",
  "id": string,          // ^el-[A-Za-z0-9_-]{1,32}$ — unique within the message
  "fallback": string,    // required, ≤ 200 Unicode code points (E5) — feeds the synthesized text
  ...variant fields
}
```

- **chip** — inline status atom, no interaction:
  `{ "el":"chip","id":"el-t1","fallback":"tests 233/233","kind":"ok"|"warn"|"err"|"info","label":"tests","value":"233/233" }`
- **job** — the live work card:
  `{ "el":"job","id":"el-b1","fallback":"header pass: running","title":"header pass","state":"running"|"ok"|"err"|"cancelled","detail":"…","startedAt":1783876000,"progress":0.6 }`
  (`progress` optional 0..1; absent = indeterminate. `startedAt` is unix seconds
  on the box clock; the app ticks elapsed time client-side.)
- **decision** — pick-one with previews:
  `{ "el":"decision","id":"el-icon","fallback":"pick: A or B","prompt":"which icon?","options":[{ "key":"A","label":"Classic","detail":"safe","thumb":{ "xfer":"…","mime":"image/png","size":12345 } }],"chosenKey":null }`
  (`thumb` reuses the existing blob/xfer transfer + media cache, ≤ 256 KiB each,
  ≤ 4 options. Option `key`s match `^[A-Za-z0-9_-]{1,32}$` — E4.)
- **approval** — explicit gate:
  `{ "el":"approval","id":"el-deploy","fallback":"approve: deploy?","title":"deploy to prod?","detail":"wrangler deploy","approveLabel":"deploy","denyLabel":"hold","resolved":null|"approved"|"denied" }`
- **checklist** — tickable list:
  `{ "el":"checklist","id":"el-v","fallback":"verify: pill, morph","title":"verify on device","items":[{ "key":"pill","label":"pill hugs name","done":false }] }`
  (≤ 12 items; item `key`s match `^[A-Za-z0-9_-]{1,32}$` — E4.)

**Updates ride `edit` frames.** An `edit` may carry `elements` with the same
schema. The app applies an **id-matched merge** that never grows a message past
4 elements (E8): a same-id element always replaces, a new id appends only while
the merged count stays under the cap, excess appends are dropped — and the box
also rejects an edit whose merge would exceed the cap. An **element-only edit**
(the tool was given elements and no text) carries the synthesized fallback join
as its `text`; the app applies the merge and keeps its rendered text. The
empty-text-keeps-text rule applies ONLY to edits carrying elements — a plain
text edit keeps legacy replacement semantics (E11).

**Push rule (E10):** `msg` frames are push-eligible, element-only ones included
(the preview is the `text`, which the synthesized join guarantees non-empty);
`edit` frames push ONLY when they carry fresh non-empty user-readable text —
an element-only edit (text == the synthesized join of its elements) never
pushes; `react` unchanged. Element-only terminal job edits therefore remain
silent and generic-push-ineligible.

FB44 adds one explicit successful-completion intent without changing that
generic rule. For an away device, a bare `job done` with `state:"ok"` pushes the
completed card once. A nonblank `notify` suppresses that completion intent and
instead produces exactly one push from the fresh notify bubble sent after the
terminal edit. Bare `err` and `cancelled` completions remain silent; supplying
`notify` gives either state the one fresh-bubble push. Foreground devices do not
receive these pushes.

The `job` MCP tool is sugar over this: `job start` posts a message with a
running job element; `job update` is an element-only edit; `job done` lands the
terminal state and applies the completion behavior above. Direct Expo and
signed-gateway completion pushes use the job title and final detail (`Completed`
when detail is blank); the box trims each field and truncates after 140 runes,
appending `…` when cut. Registered core-v1 rooms keep the registered room name
as the title and include the detail only when the device's effective FB23
clear-preview preference permits it; otherwise the core sends its generic body.

Each device can set `{"t":"set_job_completion_push","enabled":<bool>}` through
the existing nested `device_send` text-control path. The preference defaults
enabled. `false` opts only that device out of the automatic bare-success
completion push; a nonblank `notify` remains a normal fresh-message push.

## Live Activities (protocol v2, FB64)

An iOS client can register the ActivityKit push token for one currently running
job with an authenticated post-hello control:

```json
{"t":"live_activity_token","job_id":"job-1","token":"aabbccdd"}
```

Identity is the authenticated session; the frame has no device or room field.
`job_id` is `^job-[1-9][0-9]*$` (maximum 64 characters). The token is lowercase,
even-length APNs hex up to 200 characters. An empty token idempotently
unregisters that job. Malformed frames receive `bad_frame`; valid unknown or
finished jobs are ignored. Registration immediately enqueues a running-state
hydration, then job edits enqueue updates in lifecycle order. Job completion
atomically clears the registrations and enqueues one final `end` with the final
state. The frame is accepted during mailbox-gap recovery as well as steady
state, and creates no mailbox item, transcript row, ack, or generic push.

Registrations persist in `relay-state.json`, up to 32 distinct jobs per device
(oldest evicted), and are cleared on room change, rotate/unbind, revoke, or
terminal job completion.

Direct ActivityKit delivery is separate from Expo, the signed push gateway, and
core wake hints. Enable it with all four credentials; partial or invalid config
logs one startup warning and disables only Live Activities. With all credentials
absent it is silently disabled:

```bash
HOTLINE_APNS_KEY_FILE=/run/secrets/AuthKey.p8  # PKCS#8 P-256 Apple key
HOTLINE_APNS_KEY_ID=<Apple key id>
HOTLINE_APNS_TEAM_ID=<Apple team id>
HOTLINE_APNS_TOPIC=dev.hotline.mobile          # base app bundle id
HOTLINE_APNS_ENVIRONMENT=production            # default; or sandbox
```

Named instances use the normal uppercased suffix on every variable (for
example, `HOTLINE_APNS_KEY_FILE_WORK`). The sender posts directly to the selected
APNs `/3/device/<activity-token>` endpoint with push type `liveactivity`, topic
`<base-topic>.push-type.liveactivity`, priority 10, expiration 0, and a cached
50-minute ES256 provider JWT. Network work is asynchronous and ordered per
device/job; independent targets can run concurrently. Tokens, JWTs, and job
content are never logged.

The APNs payload has no keys outside `aps`; indeterminate progress is an
explicit JSON `null`:

```json
{"aps":{"timestamp":1800000001,"event":"update","content-state":{"title":"Header pass","state":"running","detail":"Compiling","progress":null,"startedAt":1799999900}}}
```

## Agent state (protocol v2)

A new **transient** frame (typing-class: never durable, never acked, never
replayed) exposes the agent's standing work so the app can render a per-bot
Agent page:

```jsonc
{ "t":"agent_state", "seq":…,
  "state": {
    "name":"Umibozu",                                   // optional, box-owned identity (FB21/FB35)
    "harness":"claude-sdk","model":"claude-opus-4-8","effort":"xhigh",  // optional identity metadata, see below
    "runs":      [ { "id":"job-1","title":"header pass","state":"running","detail":"tests","startedAt":… } ],  // ≤ 8, live jobs
    "schedules": [ { "id":"s1","label":"morning positions","next":1783900000,"recurrence":"daily 07:00" } ],    // ≤ 24
    "loops":     [ { "id":"l1","label":"email sentry","cadence":"15m","state":"active"|"paused" } ] } }          // ≤ 24
```

`harness` / `model` / `effort` are the optional, additive box identity metadata
(the same fields as `welcome` §3.2 in protocol/v2/SPEC.md, golden
`welcome-metadata.json`): the harness kind (`claude` = the TUI, `claude-sdk`,
`pi`, `opencode`), the resolved/configured model id, and the operator effort
knob verbatim; each is omitted when unknown. They ride both carriers because
`welcome` covers idle boxes on every (re)connect while the snapshot gives live
refresh when the claude-sdk harness resolves its model after a device is
already attached (the `notifications/hotline/harness_info` child→box
notification). A snapshot carrying a harness identity is never suppressed as
empty — an idle box still announces who it is.

The app treats each frame as a **full replacement** and caches the last snapshot
in memory only (it re-arrives on reconnect — no persistence, no staleness lies).

Emission:

- **On subscribe** (after welcome + gap + drain), the box sends the current
  snapshot to the freshly caught-up device.
- **On change** — a schedule created/fired/cancelled, a loop toggled, a run
  started/updated/finished — throttled to **≥ 2 s** between sends with a trailing
  send, so a burst ends with one accurate snapshot.

Sources: `runs` come from the in-memory job registry (process-lifetime — a
restart clears them, and the next snapshot reflects that honestly). `schedules`
and `loops` are read fresh from the existing schedule and loop stores; only
active (non-paused, approved) standing work appears.

The on-subscribe snapshot is **unconditional** (E7): an idle box sends
`{ "runs":[], "schedules":[], "loops":[] }`, so a device holding a stale
in-memory snapshot from before a box restart is always corrected. Suppression
of empty snapshots applies only to CHANGE broadcasts before any first non-empty
state was announced.

## Element actions (protocol v2)

A tap on an interactive element rides the **existing user-send path** — no new
inbound frame, no hidden channel. The app sends a normal `device_send` whose
text is a zero-width space (U+200B) + `/el ` + a single-line JSON object. The
size limit is **512 UTF-8 bytes over the COMPLETE canonical line** (marker
included), measured before any slicing; a canonical line contains no CR or LF
anywhere (E1):

```
​/el {"msg":"a-123","el":"el-icon","act":"pick","v":"B"}
```

Actions: `pick` (decision; `v` is the option key), `approve`/`deny` (approval;
no `v`, or `v:null`), `toggle` (checklist; `v` is an object of `{key:bool}`,
batched into one action after ~800 ms idle). The raw serialization is the
**canonical text** — old boxes, transcripts, and push previews see a
readable-enough line — while the app renders its own compact action chip
("chose B", "approved deploy", "ticked 2").

The box parses the marker at the inbound boundary. A recognized action is
delivered to the harness as a structured turn in the `<channel>` block with
`kind="element_action"` and `element_msg` / `element_id` / `element_act` /
`element_value` attributes; the agent receives it as untrusted user input, the
same trust class as any message (an approval element is UX sugar over the same
"yes" trust model — it never bypasses the permission relay or operator gates).
Box-side decoding is **strict** (E2): duplicate JSON keys anywhere, unknown
keys, wrong types, trailing content, or an identifier failing its grammar —
`msg` matches the existing frame-id shapes (`^[au]-[0-9]{1,19}$`), `el` matches
`^el-[A-Za-z0-9_-]{1,32}$`, `act` matches `^[a-z]{1,16}$`, and key values match
`^[A-Za-z0-9_-]{1,32}$` — disqualifies the line. Existence of the referenced
message/element is NOT validated (grammar only); the harness already treats
actions as untrusted input. Decoded string values must be free of control
characters (E3) — the channel formatter additionally strips any control rune
from element-action summaries as a belt.

**Any** parse failure — no marker, over the byte cap, CR/LF, bad or duplicate
JSON, unknown action, a value that does not fit its action's exact shape —
falls open: the whole text is treated as a plain message, visible and harmless.

## Read-state sync (protocol v2)

A new **transient** frame (typing-class: never durable, never acked, never
replayed) syncs a single shared read cursor across every device on the box —
Telegram-style "read on one screen, cleared on all". Additive: it rides inside
the existing per-room e1 envelope, so the relay stays key-blind, and both sides
ignore it as an unknown type when the peer predates it.

```jsonc
{ "t":"read", "j":"<decimal>" }   // identical shape BOTH directions
```

`j` is the highest **journal seq** the user has read on any device. It is in
**global journal-seq space** — the same `j` carried on every `mailbox_item`, NOT
the per-device mailbox cursor `m` (which is device-local and not comparable
across devices). A device therefore never sees another device's read expressed
in its own `m`-space; the cursor is always the shared `j`.

- **Device → box** `{ "t":"read", "j":… }` — sent when the user has the
  conversation visibly at/through journal seq `j` (client policy: scroll-to-
  bottom, foreground with the conversation open, debounced ≥ 1 s, and implicitly
  on own-send). Validated like `mailbox_ack`: `j` must be a canonical decimal and
  `≤` the box's **read watermark W**, NOT the raw outbox cursor. W is hole-free by
  construction: **W = min over PARTICIPATING devices of each device's highest
  *contiguous* durable-delivered journal seq**. A device *participates* only once it
  is both CONNECTED and CAUGHT UP — it has completed its attach reconcile/backfill so
  its contiguous head honestly reflects delivery (every gap recorded as a real hole).
  A disconnected device, or one still reconciling, is EXCLUDED: it neither pins W low
  (a dead paired device can no longer stall read-state forever) nor lets W leap past
  a range it never received. With no participating device, read-state is inert. Each
  device tracks the largest seq N such that it has received every durable frame with
  seq `≤ N`; an interior hole at seq k freezes that device's contiguous head at the
  durable seq below k until k is actually delivered (backfilled) — regardless of
  later seqs succeeding — and the min across participants holds W below any such hole
  even as higher seqs fan. This replaces the earlier monotone-max scalar, which could
  jump a partial-fan hole (seq1 fans, seq2 dropped by a sibling, seq3 fans → the
  scalar leapt to 3 though the sibling never got seq2). Bounding to W means a read is
  never accepted while its item is missing from any participating device (including
  behind an interior hole), and a transient-reserved seq (agent_state/typing/read
  fans — reusable after a restart) is never a valid target. The acceptance bound is
  re-applied atomically at persist time against the current participant set, so a
  device becoming participating mid-validation cannot let a value through that its now
  lower W should reject. The accepted `j` is then snapped DOWN to the highest durable
  seq `≤ j` using the append-only journal (ring-independent), so an interior transient
  seq — even one evicted from the hot replay ring — is never persisted as the read
  cursor and a future durable message can never inherit an already-marked-read seq.
  **Reconcile — one path for boot and reconnect.** At boot, and again on every
  (re)attach, the box reconstructs a device's true delivery state against the
  authoritative append-only `outbox.jsonl`: every durable seq above its persisted
  contiguous head that it has NOT received becomes a recorded hole (so a crash gap —
  a durable seq that reached `outbox.jsonl` but never fanned — can never be silently
  leaped by a later fold), then backfill re-delivers what it can. Only after this
  reconcile does the device participate. A device that falls too far behind (its
  hole/out-of-order tracking exceeds a per-device cap — e.g. a mailbox stuck full
  while emits continue) is collapsed into a bounded FULL RESYNC: its tracking is
  dropped and it re-provisions fresh from the mailbox/outbox on its next connect,
  excluded from W until it catches up — so per-device state stays bounded and a
  hopelessly-behind device never pins or bloats the watermark. Malformed or
  past-the-watermark is a `bad_frame` (counts
  toward the 4002 abuse limit), except during mailbox-gap recovery where a `read`
  is inert (ignored, never counted) since the device has not yet adopted the
  floor. The box **max-merges** `j` into the shared cursor (monotone — a
  lower/equal `j` is a no-op) and persists it only when it advances.
- **Box → devices** `{ "t":"read", "j":… }` — on every advance the box fans the
  new cursor as a transient to **all** active devices, the sender included
  (idempotent). It reserves no outbox seq; durable cursor churn would bloat every
  mailbox. **Ordering guarantee (both post-drain and in steady state): a `read.j`
  frame never reaches a device before that device has been delivered every
  `mailbox_item` with `J ≤ j`.** The box enforces this two ways together: it fans
  `read.j` only after item `j` is already queued on every device's ordered item
  channel (W only advances once the item is delivered to every device), and the per-device writer flushes every
  ready `mailbox_item` before it writes any transient — so a `read` can never
  overtake an item already queued for that device.
- **Box → device on attach** — after the device's mailbox drains (alongside the
  `agent_state` snapshot) the box sends the current cursor once. This is how an
  offline device converges: no replay of intermediate values is needed
  (max-merge), and the post-drain ordering guarantees the snapshot never arrives
  ahead of the messages it refers to.

The cursor persists on the box as one field on `mailboxes.json`
(`"read":"<decimal>"`, omitempty). Read-state never wakes a device (transient,
push-ineligible by construction).

**Kill switch:** `HOTLINE_UNIFIED_CHAT`-independent; gated by
`HOTLINE_READ_SYNC` (default **on**; `=0` makes the box ignore inbound `read`
frames as inert and suppress the fan/snapshot — for soak hygiene).

## Unified chat_id (protocol v2, box→harness)

With multi-device sync the harness perceives **one** conversation across every
device. An app `device_send` is stamped `chat_id="app"` (a stable constant) while
`user` and `user_id` keep the originating `deviceID` for provenance — one user,
many keyboards. Transcript records group under `ChatID:"app"` with
`UserID:deviceID`; catch-up replay round-trips the stamped meta unchanged. This
is a harness-visible semantic change gated by `HOTLINE_UNIFIED_CHAT` (default
**on**; `=0` restores the legacy per-device `chat_id=deviceID`). Purely a
box↔harness meta convention — nothing on the wire or at the relay changes.

## What's deferred (fast-follow)

Inbound coalescing of quick bursts, `wss://`, permission-relay frames, and
multi-*person* instances (`app:karen`).
