# hotline code-link-v1 crypto core (WP-CL0)

Status: **crypto core + contract, WP-CL0.** Normative extract of §2 and §4 of
the code-linking design (`design-code-linking-otp-2026-07-14.md`). This is the
CRYPTO CONTRACT for code-based (PAKE OTP) device linking: a balanced PAKE
(CPace over ristretto255) run between the box and a new client **through** an
untrusted relay, delivering the existing pair URI confidentially.

Companion goldens live in [`fixtures/`](fixtures/) and are the gate for every
implementation. The reference implementation + fixture generator is
[`ref/`](ref/) (Go, `github.com/gtank/ristretto255` for the group +
`golang.org/x/crypto` for SHA/HKDF/HMAC/AEAD — no hand-rolled primitives).

This document and its fixtures are **pure crypto**: no I/O, no network, no
persistence. The relay endpoints, box CLI, and client UI that consume these
primitives are WP-CL2 / WP-CL3 / WP-CL4 and are out of scope here. Frozen
`protocol/core-v1/` and `protocol/v2/` are untouched; this dir is a sibling
addendum.

All base64 in this document is **STRICT base64url without padding**
(`base64.RawURLEncoding.Strict()`): non-canonical trailing bits and padding are
rejected, and every decoded wire value is length-checked (msg/confirm/key = 32,
nonce = 24). This closes the malleability gap where several distinct final
characters decode to the same bytes.

The reference implementation and the CPace draft vectors are the normative
tie-breaker over prose in this document. The draft's CPace-Ristretto255 vectors
are hand-transcribed as an **external anchor** in `ref/draft_anchor.go`
(`VerifyDraftVectors`); the implementation is asserted against those literals,
not against its own regenerated output.

### Enforced order (structural, not by convention)

The load-bearing sequencing is enforced by the API's type states, so an
out-of-order call does not compile:

- The raw CPace shared point **K is never exported** — `ScalarMultVfy` returns
  an opaque `*SharedSecret` (produced only after the identity/encoding checks),
  and `ISK` consumes only that. An identity or attacker-chosen K can never reach
  the key schedule.
- **A = box** runs `NewBoxInit → (*BoxInit).Verify`. `Verify` computes K,
  derives the keys, and checks `confirm_b`; it returns a `*BoxAccepted` — the
  only object that can seal the payload — **exclusively** when `confirm_b`
  verifies. Any failure (`ErrPeerEncoding`, `ErrPeerIdentity`,
  `ErrConfirmFailed`) is a **failed attempt** (`IsFailedAttempt`) the box counts
  toward its 3-strike cap; no payload material is produced.
- **B = client** runs `NewClientResponder → (*ClientAwaiting).Finish`. `Finish`
  verifies `confirm_a` and only then decrypts the payload — B never trusts a
  payload before authenticating A.

---

## 1. Roles and the wire, in one paragraph

**A = box** (initiator; already holds the pair secret it wants to deliver).
**B = new client** (web/mobile). The relay is not a PAKE party — it stores and
ferries only opaque group elements, MACs, and one AEAD ciphertext. The short
code is the PAKE password. A sends `msg_a` (its CPace element `Ya`); B replies
with `msg_b` (`Yb`) plus `confirm_b`; A verifies `confirm_b`, then releases
`confirm_a` plus `payload_c` (the pair URI, AEAD-sealed). B verifies
`confirm_a`, decrypts, and joins the existing paste-link flow byte-identically.

---

## 2. The code: format and normalization

Rendered `3YPJ-24B8-K7QM`: **12 symbols, Crockford base32** (`0-9 A-Z` minus
`I L O U`), displayed as three hyphenated groups of 4. **Format is 4-4-4, 60
bits total, 40 secret bits vs. the relay (SETTLED).**

Two roles, split by group:

- **`channel = sid = first group`** (4 symbols, 20 public bits): the relay
  lookup handle and the CPace `sid`.
- **`PRS = the full normalized 12-symbol code`** (ASCII bytes): the CPace
  password.

**Normalization (both endpoints, identically):** strip whitespace and hyphens;
uppercase; apply the Crockford decode folds `I→1`, `L→1`, `O→0`
(case-insensitive); require **exactly 12** alphabet symbols. `U` is excluded
from the alphabet and is **not** folded — a code containing `U` (or any
non-alphabet symbol, or the wrong length) is rejected. The normalized 12-char
ASCII string is the PRS byte string; its first 4 chars are the sid.

Alphabet: `0123456789ABCDEFGHJKMNPQRSTVWXYZ`. Fixtures:
[`fixtures/code-normalize.json`](fixtures/code-normalize.json).

---

## 3. CPace instantiation (CPACE-RISTRETTO255-SHA512)

CPace exactly as in **draft-irtf-cfrg-cpace** (vectors validated against
draft-15 Appendix B.3), suite **CPACE-RISTRETTO255-SHA512**. The suite's own
test vectors reproduce byte-exact from `ref/` — see
[`fixtures/cpace-r255.json`](fixtures/cpace-r255.json) `draft`.

- **Group:** ristretto255 (a prime-order group — no cofactor/small-subgroup
  handling). `field_size_bytes = 32`. Element encodings are 32 bytes; scalars
  are 32-byte little-endian.
- **Hash for the map + ISK:** SHA-512 (`H.s_in_bytes = 128`). This is the draft
  suite hash and is what keeps us vector-compatible. **Downstream KDF is
  HKDF-SHA256** (§4) — the SHA-512/SHA-256 split is deliberate and MUST NOT be
  "unified"; a change would break draft-vector compatibility and must return as
  a spec question, not a silent swap.
- **`DSI = "CPaceRistretto255"`** (draft value). ISK DSI =
  `"CPaceRistretto255_ISK"`.
- **`PRS`** = the normalized 12-char code, ASCII (§2).
- **`sid`** = the ASCII bytes of the normalized **channel** group (4 chars).
- **`CI`** (channel identifier / context) = ASCII
  `"hotline/code-link-v1|" + relay_host`, where `relay_host` is the lowercased
  `host[:port]` of the relay base URL each party actually uses
  (`CanonicalRelayHost` / `BuildCIFromURL`; a default port is neither added nor
  stripped, so both endpoints must configure the same base URL). Binding CI to
  the relay host makes a transcript from one deployment fail to confirm against
  another. Canonicalization goldens: [`fixtures/ci-host.json`](fixtures/ci-host.json).

### 3.1 String encoding (draft §A.1–A.3)

- `prepend_len(x)` = the LEB128 encoding of `len(x)` followed by `x`.
- `lv_cat(a, b, …)` = `prepend_len(a) ‖ prepend_len(b) ‖ …`.
- `o_cat(x, y)` = `"oc" ‖ larger ‖ smaller` under lexicographical ordering.
- `zero_bytes(n)` = n zero octets.

### 3.2 Generator (draft §7.1, §7.3, §A.2)

```
len_zpad = max(0, s_in_bytes - 1 - len(prepend_len(PRS)) - len(prepend_len(DSI)))
generator_string = lv_cat(DSI, PRS, zero_bytes(len_zpad), CI, sid)
_g = ristretto255.element_derivation( SHA-512(generator_string) )   # 64→group
```

`element_derivation` is the ristretto255 one-way map from 64 uniform bytes
(`Element.SetUniformBytes`). `2 * field_size_bytes = 64` = the full SHA-512
output.

### 3.3 Exchange (draft §6.2)

- A: fresh scalar `ya`, `Ya = scalar_mult(ya, _g)` → `msg_a = base64url(Ya)`.
- B: fresh scalar `yb`, `Yb = scalar_mult(yb, _g)` → `msg_b = base64url(Yb)`.
- Shared point `K = scalar_mult_vfy(ya, Yb) = scalar_mult_vfy(yb, Ya)` (32-byte
  encoding).
- **Fresh scalar per session (per create).** A real party samples a scalar by
  reducing 64 uniform random bytes mod the group order
  (`Scalar.SetUniformBytes`), which is statistically indistinguishable from
  uniform over `[0, l)` (bias ~2⁻²⁶²·⁵). The one value that must not be used is
  **0** (probability ~2⁻²⁵²), which would make `Ya`/`Yb` the identity;
  `ScalarFromUniform` returns `ErrZeroScalar` so the caller resamples.

**Abort rules (`scalar_mult_vfy`, load-bearing — SPEC §4.1 of the design).**
Both sides MUST reject:
1. a peer element that is not a canonical ristretto255 encoding
   (`ErrPeerEncoding`);
2. a peer element equal to the group identity (`ErrPeerIdentity`);
3. a resulting shared point `K` equal to the identity.

Any abort counts as a **failed attempt** (the box's 3-strike cap, design §4.6).
Negative vectors: [`fixtures/cpace-r255.json`](fixtures/cpace-r255.json)
`negatives` (identity element; bad encoding).

### 3.4 ISK (draft §6.2, initiator-ordered; A = box)

```
transcript_ir(Ya,ADa,Yb,ADb) = lv_cat(Ya,ADa) ‖ lv_cat(Yb,ADb)
ISK = SHA-512( lv_cat(DSI‖"_ISK", sid, K) ‖ transcript_ir(Ya,ADa,Yb,ADb) )   # 64 bytes
```

hotline uses the **initiator ordering** with **empty AD** (`ADa = ADb = ""`);
A (box) is always the initiator. (`ref/` also implements `transcript_oc` and
the `CPaceSidOutput` derivation solely to reproduce the draft's parallel-exec
and optional-sid vectors; hotline does not use them.)

> Ambiguity resolved: draft-15 §B.3.7 prints the sid-output label as
> `"CPaceSidOut"`, but the normative body (§ "optional output of session id")
> and the vector both require **`"CPaceSidOutput"`** — that label reproduces the
> `sid_output_*` vectors byte-exact. hotline does not use sid-output; the
> fixture pins the correct label for completeness.

### 3.5 Message encoding

`msg_a` / `msg_b` = base64url of the 32-byte ristretto255 encodings of `Ya` /
`Yb`. The draft's optional AD fields ride empty on the wire.

---

## 4. Key schedule, confirmations, payload (hotline, §4.3 of the design)

### 4.1 Key schedule — HKDF-SHA256 over the ISK

salt = ASCII `"hotline-code-link-v1"`; each output is 32 bytes.

| output  | info label       | use                                  |
|---------|------------------|--------------------------------------|
| `K_cb`  | `"hcl-confirm-b"`| client→box confirmation MAC key      |
| `K_ca`  | `"hcl-confirm-a"`| box→client confirmation MAC key      |
| `K_pay` | `"hcl-payload"`  | AEAD key for the pair-URI payload    |

Fixtures: [`fixtures/key-schedule.json`](fixtures/key-schedule.json).

### 4.2 Transcript hash + confirmation MACs

```
th        = SHA-256( "hotline/code-link-v1|" ‖ sid ‖ "|" ‖ msg_a_b64 ‖ "|" ‖ msg_b_b64 )
confirm_b = base64url( HMAC-SHA256(K_cb, th) )
confirm_a = base64url( HMAC-SHA256(K_ca, th) )
```

- The ASCII framing runs over the **base64url** message strings; base64url
  contains no `|`, so no escaping is needed.
- **Verification is constant-time regardless of input shape.**
  `VerifyConfirmMAC` decodes into a fixed 32-byte buffer, ALWAYS computes the
  HMAC, ALWAYS compares two 32-byte values with `subtle.ConstantTimeCompare`,
  and folds the parse-valid bit into the result — a malformed, short, padded, or
  standard-base64 MAC takes the same path and simply returns false.
- **Order is load-bearing.** B proves knowledge first: the box MUST verify
  `confirm_b` before releasing anything. A failed `confirm_b` is *the*
  online-guess detector (feeds the 3-strike cap). The client MUST verify
  `confirm_a` before decrypting/trusting the payload.

Negative vector: a wrong PRS (mistyped code) yields a different generator →
different ISK → different keys → a `confirm_b` that does **not** verify under
the box's real `K_cb` over the real `th`
([`fixtures/key-schedule.json`](fixtures/key-schedule.json) `negatives`).

### 4.3 Payload wrap — XChaCha20-Poly1305 under K_pay

```
nonce     = 24 fresh random bytes
AAD       = "hotline/code-link-v1|payload|" ‖ sid
plaintext = UTF-8 JSON  {"uri":"hotline://pair?…&e=1"}
wire      = {"n": base64url(nonce), "c": base64url(ciphertext‖tag)}
```

The plaintext is exactly the pair URI `PairingURIMode` already builds (room,
secret, name, relay URL, `e=1`) — the PAKE is a delivery channel for the same
credential the QR carries; nothing in the frozen trust chain moves. The client
MUST hard-abort on any AEAD failure. Fixtures:
[`fixtures/payload.json`](fixtures/payload.json) (golden + tampered-ciphertext
and wrong-sid-AAD negatives).

---

## 5. Fixtures (the gate)

| file | contents |
|---|---|
| `ref/draft_anchor.go` (source, not a fixture) | The **external anchor**: draft-15 §B.3 (+ B.3.10/B.3.11) literals, hand-transcribed. `VerifyDraftVectors` recomputes and asserts against them. |
| [`cpace-r255.json`](fixtures/cpace-r255.json) | The draft vectors echoed from the anchor (generator_string, g, Ya, Yb, K both directions, transcript_ir/oc, ISK_ir/oc, sid_output, scalar_mult_vfy valid + invalid); a full `hotline_session` cross-vector; negatives (identity, bad encoding B.3.11 `2b3c…a51c`, short element, zero local scalar) with expected error types. |
| [`key-schedule.json`](fixtures/key-schedule.json) | ISK → K_cb/K_ca/K_pay; th; confirm_b/confirm_a; swapped-key rejection; a **real single-attempt wrong-PRS negative** (box.Verify → failed attempt) with a positive control proving PRS drives the generator. |
| [`payload.json`](fixtures/payload.json) | XChaCha20-Poly1305 payload golden (via the role API); negatives: tampered ciphertext, wrong-sid AAD, bad nonce length, wrong confirm_a. |
| [`code-normalize.json`](fixtures/code-normalize.json) | Crockford normalization: case/space/hyphen, i/l→1 o→0 folds; invalid (short, long, `U`, non-alphabet, and **non-ASCII look-alikes** `Ł`/`Ⓜ`). |
| [`ci-host.json`](fixtures/ci-host.json) | Relay-host canonicalization + CI byte string per base URL. |

Regenerate with `go run ./protocol/code-link-v1/ref/gen` (which first runs
`VerifyDraftVectors` and the wrong-PRS assertion, and refuses to write on
mismatch). `ref_test.go` re-derives every value, asserts byte-equality, drives
the full role flow, and exercises every negative with exact error types. WP-CL1
(TS twin), WP-CL2 (relay), and WP-CL3 (box CLI) MUST reproduce these bytes
exactly.

---

## 6. Ambiguity log (WP-CL0 resolutions)

1. **sid-output label** = `"CPaceSidOutput"` (draft body + vector), not the
   truncated `"CPaceSidOut"` shown in §B.3.7's inline formula. Reproduces the
   `sid_output_*` vectors byte-exact. (hotline unused; pinned for completeness.)
2. **Scalar sampling** uses `Scalar.SetUniformBytes` (64 uniform bytes reduced
   mod the group order) — the draft's permitted "uniform between 1 and order-1"
   path — rather than the "clear the high bits of 32 bytes" recommendation.
   (Note: clearing bits above 252 cannot make a ristretto255 scalar
   non-canonical, since 2²⁵²−1 < l; the two paths differ only in distribution.)
   `SetUniformBytes` is statistically indistinguishable from uniform over
   `[0, l)` (bias ~2⁻²⁶²·⁵); its one degenerate output, the zero scalar
   (~2⁻²⁵²), is rejected by `ErrZeroScalar`. Fixed test-vector scalars (already
   `< l`) load via `SetCanonicalBytes`. Interop is unaffected: the wire is fully
   determined by the scalar value, not how it was sampled.
3. **`U` is rejected, not folded.** SPEC §2 folds only `I/L→1` and `O→0`; `U`
   is excluded from the alphabet and never appears in a minted code.
4. **SHA-512 (CPace map+ISK) vs HKDF-SHA256 (downstream)** is intentional and
   frozen here — no friction hit with Go/`gtank`; the split stands.
