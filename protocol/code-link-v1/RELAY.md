# hotline code-link-v1 relay contract (WP-CL2)

Status: **relay control-plane contract, WP-CL2.** Normative extract of §5 of the
code-linking design (`design-code-linking-otp-2026-07-14.md`). This is the
HTTP contract for the **key-blind relay** endpoints that ferry the CPace PAKE
OTP device-linking flow. The crypto that produces every wire value lives in the
box and client (WP-CL0/CL1, [`SPEC.md`](SPEC.md)); the relay never runs any of
it.

Companion transcript goldens: [`fixtures/http-flows.json`](fixtures/http-flows.json)
(mirrored into `hotline-core/test/fixtures/` for the vitest conformance suite).
Reference implementation: `hotline-core/src/codes.ts` (`CodesDO`, edge dispatch
in `src/index.ts`). Frozen `protocol/core-v1/` is untouched; this is a sibling
addendum.

## Key-blindness (the reviewable property)

The relay stores and ferries ONLY:

- `channel` — the public 4-symbol first code group (20-bit lookup handle),
- `box_pub` — the box P-256 public key (already public; TOFU-bound at create),
- `msg_a` / `msg_b` — uniform ristretto255 encodings (32 B), opaque,
- `confirm_a` / `confirm_b` — HMAC tags under CDH-derived keys (32 B), opaque,
- `payload` — an XChaCha20-Poly1305 ciphertext `{n,c}` the relay cannot decrypt,
- timestamps and counters.

None of it is password-testable offline. The relay code imports **no
cryptographic library** — the only crypto it performs is P-256 **signature
verification** to authenticate box endpoints (the same scheme the `/v1/rooms/*`
control plane uses; not message crypto). This is asserted by
`hotline-core/test/link-codes-never-holds.test.ts` (stored-key allow-list) and a
grep gate over the worker diff.

## Path family & DO

`/v1/link-codes/<channel>[/<action>]`, `channel` matching
`^[0-9A-HJKMNP-TV-Z]{4}$` (Crockford base32, `I L O U` removed). Sessions live in
a dedicated `CODES` Durable Object namespace, id = `idFromName("code:"+channel)`.
Edge behavior (before the DO spins): channel-regex check; box endpoints require
the signing headers and share the per-IP `CONTROL_PLANE` limiter; client
endpoints get the dedicated per-IP `LINK_CODES` limiter.

Kill switch `CORE_CODES_ENABLED` (fail-closed on `"0"`, unset ⇒ enabled) gates
EVERY endpoint: create returns `503 provider_unavailable` (the box falls back to
printing the pair URI); claim / poll / confirm / finish all return the uniform
`404 code_unknown`, so existing sessions are instantly shed too.

## Box endpoints (SIGNED, §2.1 scheme — canonical string, ±300 s, per-DO nonce ring)

- **`POST /v1/link-codes/<channel>`** — create. Body
  `{"box_pub":{…P-256 JWK…},"msg_a":"<b64url 32B>","proto":"code-link-v1"}`.
  TOFU: binds `box_pub` and verifies against it inside a `blockConcurrencyWhile`
  race gate. A live session on the channel ⇒ `409 code_taken` (box regenerates a
  fresh channel and retries). Arms a DO alarm at `created+330 s`. →
  `200 {"expires_in":300}`.
- **`POST /v1/link-codes/<channel>/poll`** — empty JSON body `{}`. Long-poll: if
  a `pending` attempt exists, return
  `200 {"pending":{"msg_b","confirm_b","aid":<int>}}` and atomically mark it
  in-flight (at most one attempt handed out per poll, lowest `aid` first); else
  park up to **20 s** and, on timeout, return `200 {"pending":null}`. **`aid` is a
  relay-assigned opaque monotonic attempt id — the box MUST capture it and echo it
  on the matching finish.**
- **`POST /v1/link-codes/<channel>/finish`** — one of:
  - `{"ok":true,"aid":<int>,"confirm_a":"<b64url 32B>","payload":{"n":"<b64url 24B>","c":"<b64url ≤4096B>"}}`
    → stash the result for the attempt named by `aid`; the owning client's
    retrieval is the single-use consumption point (session deleted then). →
    `200 {"ok":true}`.
  - `{"ok":false,"aid":<int>}` → mismatch verdict for that attempt
    (`401 code_mismatch` to its owner); session survives for a re-typed
    (distinct) attempt. → `200 {"ok":true}`.
  - `{"ok":false,"final":true}` → box abort; **no `aid`** (session-wide); session
    deleted immediately. → `200 {"ok":true,"final":true}`.
  - Strict validation: `ok` MUST be an explicit boolean; `final` (if present)
    MUST be boolean; `final:true` REQUIRES `ok:false`; `confirm_a` canonical
    base64url of exactly 32 B; `payload.n` canonical base64url of exactly **24 B**
    (XChaCha nonce); `payload.c` canonical base64url, 1..4096 B. Anything else ⇒
    `400 bad_request`. An unknown/already-consumed `aid` ⇒ `404 code_unknown`.

## ⚠️ WP-CL2 blocker-fix wire deltas — RECONCILE WITH CL3 (`internal/app/codelink.go`)

The attempt-correlation blocker fix (attempt id threaded through the lifecycle)
changed the box↔relay contract on TWO endpoints. Every other endpoint (create,
claim, confirm) is byte-identical to before.

- **poll — ADDED field.** Response `pending` object now carries `aid` (integer):
  `{"pending":{"msg_b","confirm_b","aid"}}`. Old CL3 `pollLinkCode` unmarshals
  only `msg_b`/`confirm_b` and silently drops `aid`, so it still *reads* a poll,
  but has no aid to echo. **CL3 reconciliation:** add `Aid int` to
  `pendingAttempt` (both the nested `{"pending":…}` and flat forms) and thread it
  to `finishLinkCode`.
- **finish — ADDED required field.** Attempt-specific verdicts now REQUIRE `aid`:
  - `finishLinkCode(ok=true, …)` must send `"aid":<the aid from the poll>`.
  - `finishLinkCode(ok=false, final=false)` (strike/mismatch) must send `"aid"`.
  - `abortSession` → `finishLinkCode(ok=false, final=true)` sends **no `aid`**
    (unchanged; the relay treats final:true as a session-wide abort).
  Without the `aid`, the relay returns `400 bad_request` for ok:true / plain
  ok:false. **CL3 reconciliation:** add an `aid int` parameter to
  `finishLinkCode` and include `m["aid"]=aid` except on the final-abort path;
  plumb the aid from the `*pendingAttempt` returned by `pollLinkCode`.
- **finish payload strictness (NEW rejects).** `payload.n` must decode to exactly
  24 bytes (it already is — `ref.NonceBytes`); `payload.c` must be ≤ 4096 B; both
  strict canonical base64url. CL3 already produces these, so no code change is
  expected — but a future change that pads/re-encodes would now 400.

CL3 already tolerates the nested-vs-flat poll shape; the only *required* CL3 edits
are (1) capture `aid` from poll and (2) send it on the two attempt-specific
finish calls. The abort path and all client (CL4/CL5) surfaces are unchanged.

## Client endpoints (UNSIGNED, per-IP `LINK_CODES` limited at the edge)

- **`POST /v1/link-codes/<channel>/claim`** — empty body.
  Unknown/expired/exhausted ⇒ uniform `404 code_unknown`. Else `claims+1` (cap
  **10** ⇒ session deleted ⇒ 404 thereafter). → `200 {"msg_a","expires_in"}`.
- **`POST /v1/link-codes/<channel>/confirm`** — body
  `{"msg_b":"<b64url 32B>","confirm_b":"<b64url 32B>"}` (both length-validated).
  A distinct attempt ⇒ `confirms+1` (cap **3** ⇒ session deleted ⇒ 404); an
  identical retry is the **same attempt** (no double count). Stores `pending`,
  wakes a parked box poll, then parks up to **25 s** for the box's finish.
  Outcomes: `200 {"confirm_a","payload"}` (session deleted, single-use) |
  `401 code_mismatch` | `504 box_away` (box never polled — the client may retry
  the identical confirm).

## Caps, TTL, single-use, uniform-404 (design §4.5)

`claims ≤ 10`, `confirms ≤ 3` (DISTINCT attempts; an identical confirm retry maps
to the same `aid` and does not re-strike), TTL 300 s **inclusive** (`created+300`
is already expired; a session at/past TTL is treated as absent and self-cleans),
alarm backstop at 330 s, single-use delete on successful consume. Every terminal
path (successful consume, final abort, claim/confirm cap, expiry, alarm) does a
full `deleteAll` + `deleteAlarm`, so no channel leaks the `nonces` ring or any
other key past teardown — storage is bounded to a single self-cleaning session
per channel. Unknown / expired / exhausted / consumed all collapse to
`404 {"code":"code_unknown"}` — no distinguishing wire oracle (a small *timing*
residual remains: expiry/cap paths do a delete where an unknown-channel probe
does not; documented, accepted). Added error codes (additive over frozen §2.6):
`code_unknown`, `code_taken`, `code_mismatch`, `box_away`.

## Long-poll parking decision (design Open-question #1)

Implemented as **in-DO in-memory promise parking** (poll 20 s / confirm 25 s),
NOT the 2 s short-poll fallback. Rationale: parking is a pure latency
optimization layered over durable storage — a parked promise only ever early-
wakes a request that would otherwise return the same storage-derived answer.
Because box poll and client confirm for a channel always hit the *same* DO
instance, the in-memory waiter set coordinates them directly. If the DO
hibernates/evicts and drops the promise, the request times out and the peer
retries idempotently (identical confirm = same attempt, no double count), so
**correctness never depends on the promise surviving**. The hibernation friction
the design flagged is therefore a non-issue and the short-poll fallback was not
needed.

## Ready for CL3/CL4/CL5

The relay is a dumb, key-blind board driven entirely by the box and clients:
CL3 (box CLI) drives create/poll/finish (signed); CL4 (web) and CL5 (Expo)
drive claim/confirm (unsigned). No further relay changes are required for those
workpieces.
