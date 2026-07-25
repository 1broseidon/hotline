// Command gen regenerates the cryptographic golden fixtures for core-v1:
// fixtures/envelope-e1.json and fixtures/signing-core.json. Run from the repo
// root:
//
//	go run ./protocol/core-v1/ref/gen
//
// Everything is deterministic except the two precomputed ECDSA signatures
// (ECDSA is randomized); those are generated once, checked in, and validated
// by verify (plus the sign→verify roundtrip in ref_test.go). Do not regenerate
// casually — these fixtures are FROZEN gates for WP1/WP2/WP3.
package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log"
	"os"

	corev1ref "github.com/1broseidon/hotline/protocol/core-v1/ref"
)

const (
	// Fixture pairing, shared with protocol/v2/fixtures/*.json.
	secret = "Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ"
	room   = "Ab3dEf6hIj8lMn0pQr2tUv"

	// Fixture P-256 private scalar: bytes 0x01..0x20 (valid, < n).
	// base64url of 0102...20.
	fixtureD = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA"

	// Fixed timestamp for signing vectors: 2026-07-14T00:00:00Z.
	fixtureTS = "1752451200"
)

// Exact inner v2 frames (compact JSON), lifted verbatim from the v2 goldens.
const (
	innerB2A = `{"t":"mailbox_item","m":"9007199254741009","j":"815","id":"env-j815-daf31fd290542","payload":{"t":"sent","seq":815,"id":"u-815","cid":"01J2ZK8Q0X6WYV9R3T5B7N4E2M","text":"yo from the phone","device":"dev-af31fd290542"}}`
	innerA2B = `{"t":"hello","v":2,"device_id":"dev-af31fd290542","secret":"Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ","resume_from":"9007199254741005","push":{"token":"ExponentPushToken[fixture-v2-drain]","platform":"ios"}}`
)

func fixedNonce(start byte) []byte {
	n := make([]byte, 24)
	for i := range n {
		n[i] = start + byte(i)
	}
	return n
}

func must[T any](v T, err error) T {
	if err != nil {
		log.Fatal(err)
	}
	return v
}

func writeJSON(path string, v any) {
	data := must(json.MarshalIndent(v, "", "  "))
	data = append(data, '\n')
	if err := os.WriteFile(path, data, 0o644); err != nil {
		log.Fatal(err)
	}
	fmt.Println("wrote", path)
}

type e1Frame struct {
	T string `json:"t"`
	N string `json:"n"`
	C string `json:"c"`
}

func main() {
	raw := must(corev1ref.DecodeSecret(secret))
	keys := must(corev1ref.DeriveKeys(raw))

	type encVec struct {
		Name  string  `json:"name"`
		Dir   string  `json:"dir"`
		Key   string  `json:"key"`
		AAD   string  `json:"aad"`
		Inner string  `json:"inner"`
		Frame e1Frame `json:"frame"`
	}
	b64 := corev1ref.ScalarB64

	aadB2A := must(corev1ref.AAD(room, "b2a"))
	aadA2B := must(corev1ref.AAD(room, "a2b"))

	seal := func(key, nonce, aad []byte, inner string) e1Frame {
		n, c, err := corev1ref.Seal(key, nonce, aad, []byte(inner))
		if err != nil {
			log.Fatal(err)
		}
		return e1Frame{T: "e1", N: n, C: c}
	}
	encB2A := seal(keys.KB2A, fixedNonce(0x00), aadB2A, innerB2A)
	encA2B := seal(keys.KA2B, fixedNonce(0xA0), aadA2B, innerA2B)

	// Tamper case: flip the last byte of the b2a ciphertext (inside the tag).
	tampered := encB2A
	cb := []byte(tampered.C)
	if cb[len(cb)-1] == 'A' {
		cb[len(cb)-1] = 'B'
	} else {
		cb[len(cb)-1] = 'A'
	}
	tampered.C = string(cb)

	envelope := map[string]any{
		"intent": "HKDF + XChaCha20-Poly1305 vectors for the e1 envelope (core-v1 SPEC §2.2–2.3). Byte-exact: an implementation must reproduce every derived key and ciphertext exactly, decrypt both directions, and DROP every frame in reject[].",
		"secret": secret,
		"room":   room,
		"hkdf": map[string]any{
			"hash":      "SHA-256",
			"salt":      "hotline-e1",
			"k_b2a":     map[string]string{"info": corev1ref.InfoB2A, "key": b64ify(keys.KB2A)},
			"k_a2b":     map[string]string{"info": corev1ref.InfoA2B, "key": b64ify(keys.KA2B)},
			"room_auth": map[string]string{"info": corev1ref.InfoRoomAuth, "key": b64ify(keys.RoomAuth), "auth_hash": corev1ref.AuthHash(keys.RoomAuth)},
		},
		"encrypt": []encVec{
			{Name: "b2a-mailbox-item", Dir: "b2a", Key: "k_b2a", AAD: string(aadB2A), Inner: innerB2A, Frame: encB2A},
			{Name: "a2b-hello", Dir: "a2b", Key: "k_a2b", AAD: string(aadA2B), Inner: innerA2B, Frame: encA2B},
		},
		"reject": []map[string]any{
			{"name": "tampered-ciphertext", "key": "k_b2a", "aad": string(aadB2A), "frame": tampered, "expect": "drop"},
			{"name": "wrong-aad-direction", "key": "k_b2a", "aad": string(aadA2B), "frame": encB2A, "expect": "drop", "note": "reflection defense: b2a ciphertext must not open under the a2b AAD"},
			{"name": "wrong-aad-room", "key": "k_b2a", "aad": "hotline/e1|Zz9yXx7wVv5uTt3sRr1qPo|b2a", "frame": encB2A, "expect": "drop"},
			{"name": "wrong-key", "key": "k_a2b", "aad": string(aadB2A), "frame": encB2A, "expect": "drop"},
		},
	}
	writeJSON("protocol/core-v1/fixtures/envelope-e1.json", envelope)

	// ---- signing-core.json ----
	priv := must(corev1ref.KeyFromScalar(fixtureD))
	pub := map[string]string{"kty": "EC", "crv": "P-256", "x": b64(priv.PublicKey.X), "y": b64(priv.PublicKey.Y)}

	registerBody := must(json.Marshal(map[string]any{
		"box_pub":   pub,
		"name":      "pi",
		"auth_hash": corev1ref.AuthHash(keys.RoomAuth),
		"proto":     "core-v1",
	}))
	wakeBody := []byte(`{"device_id":"dev-af31fd290542","kind":"message","preview_c":null}`)

	type sigVec struct {
		Name      string `json:"name"`
		Method    string `json:"method"`
		Path      string `json:"path"`
		Timestamp string `json:"timestamp"`
		Nonce     string `json:"nonce"`
		Body      string `json:"body"`
		Canonical string `json:"canonical"`
		Signature string `json:"signature"`
	}
	mkVec := func(name, path, nonce string, body []byte) sigVec {
		canon := corev1ref.CanonicalString("POST", path, fixtureTS, nonce, body)
		sig := must(corev1ref.SignLowS(priv, []byte(canon)))
		return sigVec{Name: name, Method: "POST", Path: path, Timestamp: fixtureTS, Nonce: nonce, Body: string(body), Canonical: canon, Signature: sig}
	}
	regVec := mkVec("register", "/v1/rooms/"+room+"/register", "fixture-nonce-0001", registerBody)
	wakeVec := mkVec("wake", "/v1/rooms/"+room+"/wake", "fixture-nonce-0002", wakeBody)

	highS := must(corev1ref.HighSTwin(regVec.Signature))

	signing := map[string]any{
		"intent": "Signed-header vectors for the core-v1 control plane (SPEC §3.1). Scheme is verbatim hotline-push-gateway PROTOCOL.md §1–2 / internal/app/pushsign.go. Verifiers MUST accept vectors[] and reject every case in reject[] with the named code. Signers are validated by sign→verify roundtrip plus the mandatory low-S assertion (ECDSA is randomized, so signer output is not byte-comparable).",
		"key":    map[string]string{"kty": "EC", "crv": "P-256", "d": fixtureD, "x": pub["x"], "y": pub["y"]},
		"room":   room,
		"headers": map[string]string{
			"X-Hotline-Timestamp": "unix seconds, decimal, within ±300 s",
			"X-Hotline-Nonce":     "fresh per request; base64url, 8–64 chars; replay-checked per room over 300 s",
			"X-Hotline-Signature": "base64url (no padding) of the 64-byte P1363 r||s, low-S MANDATORY",
		},
		"canonical_string": "METHOD\\npath\\ntimestamp\\nnonce\\nbase64url(SHA-256(exact body bytes)) — joined with single \\n, no trailing newline",
		"vectors":          []sigVec{regVec, wakeVec},
		"reject": []map[string]any{
			{"name": "high-s", "vector": "register", "signature": highS, "expect": "bad_signature", "note": "the malleable twin s' = n - s of the valid register signature; verify must enforce low-S explicitly"},
			{"name": "tampered-body", "vector": "register", "body": string(registerBody[:len(registerBody)-1]) + " }", "expect": "bad_signature", "note": "same headers/signature, body bytes changed → body hash mismatch"},
			{"name": "wrong-path", "vector": "register", "path": "/v1/rooms/" + room + "/wake", "expect": "bad_signature", "note": "signature bound to path via canonical string"},
			{"name": "stale-timestamp-past", "vector": "register", "verify_at": 1752451501, "expect": "stale_timestamp", "note": "301 s after the signed timestamp; window is ±300 s inclusive"},
			{"name": "stale-timestamp-future", "vector": "register", "verify_at": 1752450899, "expect": "stale_timestamp"},
			{"name": "replayed-nonce", "vector": "register", "expect": "nonce_replayed", "note": "presenting the exact register vector a second time within 300 s of the first must be rejected"},
		},
		"rules": map[string]any{
			"timestamp_window_seconds": 300,
			"nonce_window_seconds":     300,
			"key_lookup":               "no X-Hotline-Key-Id header: the verifying key is reg.box_pub for the room named in the path; first register is TOFU",
		},
	}
	writeJSON("protocol/core-v1/fixtures/signing-core.json", signing)
}

func b64ify(b []byte) string {
	return base64.RawURLEncoding.EncodeToString(b)
}
