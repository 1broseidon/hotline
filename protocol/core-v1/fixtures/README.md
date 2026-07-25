# core-v1 golden fixtures — THE GATE

These fixtures are **FROZEN** and normative for the core-v1 protocol
([`../SPEC.md`](../SPEC.md)).

**Gate rule: WP1 (hotline-core worker), WP2 (box), and WP3 (app)
implementations MUST pass these vectors verbatim — byte-exact — before any
other acceptance criterion counts.** An implementation that disagrees with a
fixture is wrong; the fixture does not move. Changing any fixture requires the
architect reopening the frozen spec.

| fixture | gates | what it pins |
|---|---|---|
| `envelope-e1.json` | WP2 (Go codec), WP3 (TS codec) | HKDF-SHA256 derivations (K_b2a / K_a2b / room_auth / auth_hash) and XChaCha20-Poly1305 `e1` frames: byte-exact encrypt both directions, decrypt both directions, and DROP every `reject[]` case (tamper, wrong AAD direction/room, wrong key) |
| `signing-core.json` | WP1 (verify), WP2 (sign) | fixture P-256 key, canonical strings for register/wake, one precomputed valid low-S signature that must verify, and reject cases: high-S twin, tampered body, wrong path, stale timestamp (±300 s), replayed nonce. Signers are validated by sign→verify roundtrip + low-S assertion (ECDSA is randomized) |
| `room-register.json` | WP1, WP2 | register body/response goldens, TOFU + key_mismatch + bad-request semantics |
| `device-tokens.json` | WP1, WP2 | PUT/DELETE device record semantics, closed platform set, unknown_room |
| `wake-hint.json` | WP1, WP2 | wake body golden, **exact generic Expo POST body**, push-test body, throttle sequence, no_token / token_invalid / bad kind semantics |
| `pair-uri-e.json` | WP2 (mint), WP3 (parse) | `e=1` parse rules and the old-app-ignores case |
| `../../v2/fixtures/*.json` | WP1 (pipe pass-through), WP2/WP3 (inner frames) | unchanged and still normative: enveloped sessions' inner frames must satisfy the v2 goldens byte-exact |

## Provenance

The cryptographic fixtures (`envelope-e1.json`, `signing-core.json`) are
generated — never hand-computed — by the checked-in reference implementation:

```
go run ./protocol/core-v1/ref/gen     # from the repo root
```

`../ref/ref_test.go` validates every vector round-trip against the reference
implementation (x/crypto `chacha20poly1305.NewX` + `hkdf`; ECDSA per
`internal/app/pushsign.go`) and runs with the ordinary `go test ./...`.
The signature scheme is verbatim `hotline-push-gateway/PROTOCOL.md` §1–2.

Do not regenerate casually: the two precomputed ECDSA signatures change on
every run (ECDSA is randomized) even though everything else is deterministic.
