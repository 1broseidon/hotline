# hotline core-v1 protocol (FROZEN)

Status: **FROZEN 2026-07-14.** Normative extract of §2–§4 of the hotline-core
build spec. Companion goldens live in [`fixtures/`](fixtures/) and are the gate
for every implementation (see `fixtures/README.md`). The v2 device protocol
(`../v2/SPEC.md`) is FROZEN and untouched; core-v1 is purely additive: the `e1`
envelope, the `e=1` pair-URI param, and four signed HTTP control endpoints.

Reference implementation + fixture generator: [`ref/`](ref/) (Go,
`golang.org/x/crypto` only — no hand-rolled primitives).

---

## 1. The `e1` E2E envelope

One frame type wraps every v2 frame, both directions, from the first byte after
socket open (`hello` included — the pairing secret never transits readable):

```json
{"t":"e1","n":"<base64url 24-byte nonce>","c":"<base64url ciphertext||tag>"}
```

All base64 in this document is **base64url without padding**.

- **AEAD**: XChaCha20-Poly1305. Nonce is 24 random bytes per frame; random
  nonces are safe at this size and carry no counter state across reconnects.
  Go: `golang.org/x/crypto/chacha20poly1305.NewX`. TS:
  `@noble/ciphers/chacha` `xchacha20poly1305` + `@noble/hashes` for HKDF.
- **Plaintext** = the exact UTF-8 bytes of one v2 frame's JSON. Inner frames
  are the unchanged FROZEN v2 protocol; `../v2/fixtures/*.json` remain
  normative byte-exact on the inner frames.
- **AAD** = ASCII `"hotline/e1|<room>|<dir>"`, `<dir>` ∈ `b2a` | `a2b`. Binds
  ciphertext to room and direction (no reflection). The `|` characters are
  literal bytes; there is no escaping (room ids are `[A-Za-z0-9_-]{22}` and
  cannot contain `|`).
- **Receiver rules**: an `e1` that fails to decrypt is dropped and counted
  toward the v2 §3.8 abuse limit. On an envelope-mode room any non-`e1` frame
  is dropped (a plaintext `hello` gets no reply). Receivers do NOT track
  nonces; replay is absorbed by the v2 layer's cid/id dedup and cursors.
- **Mode is per-room, forever.** A room is `e1` or plaintext for its lifetime;
  switching = mint a new link (rotates room+secret).
- **Sizing**: worst-case blob_chunk stays under the 1 MiB pipe cap; no
  inner-frame changes.

### 1.1 Key derivation

`S` = the 32 raw bytes from base64url-decoding the 43-char pairing secret.
All derivations are HKDF-SHA256 with salt = ASCII `"hotline-e1"`:

| output | info label | length | held by |
|---|---|---|---|
| `K_b2a` (box→app content key) | `"hotline-e2e-v1\|b2a"` | 32 B | box, app |
| `K_a2b` (app→box content key) | `"hotline-e2e-v1\|a2b"` | 32 B | box, app |
| `room_auth` (future §8.1 seam) | `"hotline-room-auth-v1"` | 32 B | box, app |

The core is registered `auth_hash = base64url(SHA-256(room_auth))` and nothing
else — two one-way steps from `S`, on a different info label than the content
keys. Test vectors for all three derivations: `fixtures/envelope-e1.json`.

### 1.2 Pair-URI negotiation

New pair-URI param `e=1` (additive to v2 §1) marks the pairing envelope-mode.
Exactly the value `1` enables it; absent or any other value ⇒ plaintext. Old
parsers ignore unknown params per v2 rules (they pair, send plaintext hello on
the e-room, and get silence — the error surface is "update the app").
Fixtures: `fixtures/pair-uri-e.json`.

---

## 2. Signed control plane

All control paths live under `/v1/` on the relay host; the pipe plane
`/r/<room>/{c,a}` is unversioned and immortal. All four endpoints are
box→core, `POST`/`PUT`/`DELETE`, `Content-Type: application/json` required,
bodies ≤ 16 KiB (413 otherwise).

### 2.1 Request signing (verbatim gateway PROTOCOL.md §1–2)

**P-256 (secp256r1) ECDSA / SHA-256** over a canonical string — the scheme is
verbatim `hotline-push-gateway/PROTOCOL.md` §1–2 and the box's
`internal/app/pushsign.go`, with one difference: there is **no
`X-Hotline-Key-Id`** header. The verifying key is the `box_pub` bound to the
room named in the path (first register is TOFU, §2.2).

Canonical string — five fields joined by a single `\n`, no trailing newline:

```
METHOD                      e.g. POST or PUT or DELETE
<path>                      e.g. /v1/rooms/<room>/wake (path only, no host/query)
<timestamp>                 integer UTC unix SECONDS, as sent in the header
<nonce>                     the per-request nonce, as sent in the header
<body_hash>                 base64url(SHA-256(exact raw body bytes))
```

An empty body (DELETE) hashes zero bytes. Headers:

| header | value |
|---|---|
| `X-Hotline-Timestamp` | UTC unix seconds, decimal; rejected outside ±300 s (`stale_timestamp`) |
| `X-Hotline-Nonce` | fresh per request; base64url, 8–64 chars; replay-checked per room over 300 s (`nonce_replayed`) |
| `X-Hotline-Signature` | base64url (no padding) of the 64-byte P1363 `r‖s`; **low-S mandatory** (s ≤ n/2), high-S rejected `bad_signature`; DER rejected |

The core verifies with WebCrypto ECDSA P-256/SHA-256 plus an explicit low-S
check. Signer keys: box's `<StateDir>/box-key.json`, a P-256 private JWK
distinct from `push-signing-key.json`, generated on first core-mode start.
Vectors: `fixtures/signing-core.json`.

### 2.2 `POST /v1/rooms/<room>/register` — signed, idempotent, TOFU

First call binds `box_pub` to the room (room ids are unguessable 128-bit
values that only transit the QR). Subsequent calls must verify against the
bound key (else `403 key_mismatch`) and update `name`/`auth_hash` (link
rotation re-registers). Body:

```json
{"box_pub":{"kty":"EC","crv":"P-256","x":"<b64url>","y":"<b64url>"},
 "name":"pi","auth_hash":"<base64url sha-256, §1.1>","proto":"core-v1"}
```

→ `200 {"room":"<id>","registered":true}`. Register is the only endpoint
exempt from `404 unknown_room` (it creates). Fixtures:
`fixtures/room-register.json`.

### 2.3 `PUT|DELETE /v1/rooms/<room>/devices/<device_id>` — signed

The box forwards tokens it received on `hello.push` (inside the envelope). No
device→core auth exists in v1. PUT body:

```json
{"platform":"ios","tokens":{"expo":"ExponentPushToken[...]","native":"<hex, optional>"}}
```

`platform` ∈ `ios` | `android` (closed set). PUT replaces the device record.
`DELETE` (signed, empty body) removes it and is idempotent. Fixtures:
`fixtures/device-tokens.json`.

### 2.4 `POST /v1/rooms/<room>/wake` — the wake hint

Sent per push-eligible durable item when the box's presence lease says the
device is away. Carries **no content**:

```json
{"device_id":"dev-af31fd290542","kind":"message","preview":null,"preview_c":null}
```

- `kind`: `"message"` only in v1; closed set, unknown → `400 bad_request`.
- `preview`: **OPTIONAL CLEAR PREVIEW** — the message plaintext, opt-in per box
  via `HOTLINE_PUSH_PREVIEW=clear` (§4). Absent, `null`, or empty ⇒ the generic
  notification below, byte-identical to today. When present and non-empty the
  core uses it **verbatim** as the notification `body` (`title` unchanged). The
  **box** truncates it to **140 runes** (rune-safe, never splitting a multibyte
  character, `…` appended on a cut) *before* sending; the core does not
  re-truncate. `preview` is covered by the request signature (the canonical
  string hashes the exact body bytes). **Privacy**: only this snippet transits
  readable, by explicit box-owner choice; the mailbox E2E envelope is untouched.
- `preview_c`: reserved NSE seam, always null in v1; a non-null value is
  accepted, never logged, and never alters the v1 notification. **`preview` and
  `preview_c` are mutually exclusive**: a request with both non-null ⇒
  `400 bad_request`.
- Core: verify → throttle (max 1 push per device per 10 s AND max 6 per device
  per rolling minute; throttled ⇒ `200 {"pushed":false,"reason":"throttled"}`,
  never an error) → resolve `dev:<device_id>` → build the notification (preview
  body if a `preview` is present, else generic) → provider router →
  `200 {"pushed":true,"provider":"expo","disposition":"none"}`.
- Unknown device / no tokens ⇒ `200 {"pushed":false,"reason":"no_token"}`.
- Generic notification: `title` = registered room `name`, `body` =
  `"New message"`, `data` = `{"url":"hotline://chat","room":"<room>"}`,
  plus `sound:"default"`, `priority:"high"` (matching the box's Expo-direct
  behavior). Exact Expo POST body goldens (generic + preview): `fixtures/wake-hint.json`.
- Dispositions use the gateway's provider-neutral vocabulary
  (`none | drop_token | retry | backoff`; codes `ok | token_invalid | retry |
  backoff`). On token_invalid the core deletes the token record AND returns
  `410 {"code":"token_invalid"}` so the box drops its local copy.

### 2.5 `POST /v1/rooms/<room>/push-test` — signed

Body `{"device_id":"..."}`. Bypasses the wake throttle (own cap: 1 per device
per minute), sends `title` = `name`, `body` = `"Test push"`, same `data`.
Backs `hotline relay push-test`.

### 2.6 Errors (closed set)

`{"code":"<stable>"}` with status: `400 bad_request`, `401 bad_signature |
stale_timestamp | nonce_replayed`, `403 key_mismatch`, `404 unknown_room`
(register exempt), `410 token_invalid`, `413 bad_request` (body > 16 KiB),
`429 rate_limited` (+`Retry-After`), `503 provider_unavailable`. The box
treats unknown codes as retryable-once. Breaking changes go to `/v2/`; `/v1/`
never mutates.

---

## 3. Ambiguity log (boring options chosen, WP0)

Where the frozen build spec left room, WP0 picked the boring reading. These
are now part of the frozen contract:

1. **base64url everywhere means no padding** (`base64.RawURLEncoding` /
   canonical unpadded base64url), matching the gateway and pushsign.go.
2. **Expo POST body includes `sound:"default"` and `priority:"high"`** in
   addition to the spec'd to/title/body/data — matching the box's current
   Expo-direct `sendPush` so core-mode notifications behave identically.
3. **`e` param strictness**: only the exact value `1` enables envelope mode;
   `e=0`, `e=yes`, or absence ⇒ plaintext.
4. **DELETE body hash**: an absent/empty body hashes zero bytes
   (`SHA-256("")`) in the canonical string.
5. **DELETE of an unknown device is `200`** (idempotent), not 404.
6. **`token_invalid` surfaces as HTTP `410 {"code":"token_invalid"}`** on the
   wake/push-test response (spec lists 410 in the error set and requires the
   token to be "returned" — the stable code is the return).
7. **Timestamp window is inclusive**: |skew| ≤ 300 s accepted; 301 s rejected.
8. **Wake throttle is per device** for both limits (10 s spacing, 6/min), and
   the 6/min is a rolling window.
9. **PUT devices is full replace**, not merge, of the device record.
10. **Bad room id in the path is `400 bad_request`** (same
    `[A-Za-z0-9_-]{22}` regex as the pipe), distinct from `404 unknown_room`
    for a well-formed but unregistered room.
11. **AAD has no escaping**; room ids can't contain `|`.
12. **`preview_c` non-null in v1** is accepted and ignored (forward-compatible
    with the NSE fast-follow) rather than rejected.
13. **`preview` truncation is the box's job** (140 runes, rune-safe, `…` on
    cut); the core uses the received `preview` verbatim and never re-truncates
    or re-encodes it.
14. **Empty `preview` (`""`) is treated as absent** ⇒ the generic notification,
    byte-identical to a wake with no `preview` field. Both `preview` and
    `preview_c` non-null ⇒ `400 bad_request` (mutually exclusive). The core
    MUST NOT log or persist `preview` (it is message plaintext).
