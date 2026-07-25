package app

import (
	"crypto/ecdsa"
	"crypto/sha256"
	"encoding/base64"
	"math/big"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"
)

func newTestSigner(t *testing.T) *pushSigner {
	t.Helper()
	signer, err := newPushSigner(filepath.Join(t.TempDir(), pushSigningKeyFile), "https://push.hotline.dev", nil, nil)
	if err != nil {
		t.Fatalf("newPushSigner: %v", err)
	}
	return signer
}

// TestCanonicalStringByteForByte pins the canonical signing string and body-hash
// exactly to PROTOCOL.md §1.1: five fields joined by single '\n', no trailing
// newline, body_hash = base64url_nopad(SHA-256(body)).
func TestCanonicalStringByteForByte(t *testing.T) {
	// Well-known vector: SHA-256("") base64url (no padding).
	if got := bodyHashBase64URL([]byte("")); got != "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU" {
		t.Fatalf("bodyHashBase64URL(\"\") = %q", got)
	}

	hash := bodyHashBase64URL([]byte(""))
	canon := canonicalString("POST", "/v1/push", "1700000000", "testnonce", hash)
	want := "POST\n/v1/push\n1700000000\ntestnonce\n47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU"
	if canon != want {
		t.Fatalf("canonical string =\n%q\nwant\n%q", canon, want)
	}
	if strings.HasSuffix(canon, "\n") {
		t.Fatal("canonical string must not have a trailing newline")
	}
	if n := strings.Count(canon, "\n"); n != 4 {
		t.Fatalf("canonical string has %d newlines, want 4", n)
	}
	// The field order is METHOD, PATH, ts, nonce, body_hash.
	parts := strings.Split(canon, "\n")
	for i, exp := range []string{"POST", "/v1/push", "1700000000", "testnonce", hash} {
		if parts[i] != exp {
			t.Fatalf("field %d = %q, want %q", i, parts[i], exp)
		}
	}
}

// TestSignatureAlwaysLowSAndVerifies runs the real sign path many times and
// asserts every signature is low-S (s <= n/2) AND round-trips through Go's own
// ecdsa.Verify with the derived public key. A high-S bug is intermittent
// (~50%), so a single iteration would be worthless — PROTOCOL.md §2.
func TestSignatureAlwaysLowSAndVerifies(t *testing.T) {
	signer := newTestSigner(t)
	curve := signer.priv.Params()
	halfN := new(big.Int).Rsh(curve.N, 1)

	const iterations = 100
	for i := 0; i < iterations; i++ {
		body := []byte(`{"platform":"ios","token":"aabbcc","n":` + strconv.Itoa(i) + `}`)
		h, err := signer.signCanonical("POST", "/v1/push", body)
		if err != nil {
			t.Fatalf("iter %d signCanonical: %v", i, err)
		}

		// The signature is exactly 86 base64url chars of a 64-byte r||s.
		if len(h.Signature) != 86 {
			t.Fatalf("iter %d signature len = %d, want 86", i, len(h.Signature))
		}
		sig, err := base64.RawURLEncoding.DecodeString(h.Signature)
		if err != nil {
			t.Fatalf("iter %d signature not base64url: %v", i, err)
		}
		if len(sig) != 64 {
			t.Fatalf("iter %d decoded signature len = %d, want 64", i, len(sig))
		}

		r := new(big.Int).SetBytes(sig[0:32])
		s := new(big.Int).SetBytes(sig[32:64])
		if r.Sign() == 0 || s.Sign() == 0 {
			t.Fatalf("iter %d zero scalar r=%v s=%v", i, r.Sign(), s.Sign())
		}
		// The mandatory low-S invariant: s must never exceed n/2.
		if s.Cmp(halfN) > 0 {
			t.Fatalf("iter %d HIGH-S signature: s > n/2 (this is the ~50%% intermittent gateway-rejection bug)", i)
		}

		// Round-trip: reconstruct the exact canonical string from the returned
		// headers and verify with the derived public key.
		canon := canonicalString("POST", "/v1/push", h.Timestamp, h.Nonce, bodyHashBase64URL(body))
		digest := sha256.Sum256([]byte(canon))
		if !ecdsa.Verify(&signer.priv.PublicKey, digest[:], r, s) {
			t.Fatalf("iter %d ecdsa.Verify failed for our own signature", i)
		}
	}
}

// TestSignedHeaderFormats pins the header shapes the gateway enforces
// (index.ts validSignedFormat / PROTOCOL.md §1.2): decimal unix SECONDS
// timestamp, base64url nonce of 8..64 chars, and an 86-char base64url signature.
func TestSignedHeaderFormats(t *testing.T) {
	signer := newTestSigner(t)
	before := time.Now().Unix()
	h, err := signer.signCanonical("POST", "/v1/registrations/complete", []byte(`{}`))
	if err != nil {
		t.Fatalf("signCanonical: %v", err)
	}
	after := time.Now().Unix()

	// Timestamp: decimal, <= 15 digits, and unix SECONDS (not milliseconds).
	if !regexp.MustCompile(`^\d{1,15}$`).MatchString(h.Timestamp) {
		t.Fatalf("timestamp %q is not decimal <=15 digits", h.Timestamp)
	}
	ts, err := strconv.ParseInt(h.Timestamp, 10, 64)
	if err != nil {
		t.Fatalf("timestamp parse: %v", err)
	}
	if ts < before || ts > after {
		t.Fatalf("timestamp %d not within [%d,%d]", ts, before, after)
	}
	// A milliseconds bug would land ~1.7e12; seconds are ~1.7e9. Guard the scale.
	if ts >= 100_000_000_000 {
		t.Fatalf("timestamp %d looks like milliseconds, want seconds", ts)
	}

	// Nonce: canonical base64url, 8..64 chars.
	if len(h.Nonce) < 8 || len(h.Nonce) > 64 {
		t.Fatalf("nonce len = %d, want 8..64", len(h.Nonce))
	}
	if !regexp.MustCompile(`^[A-Za-z0-9_-]+$`).MatchString(h.Nonce) {
		t.Fatalf("nonce %q is not base64url", h.Nonce)
	}

	// Signature: exactly 86 base64url chars decoding to 64 bytes.
	if len(h.Signature) != 86 {
		t.Fatalf("signature len = %d, want 86", len(h.Signature))
	}
	sig, err := base64.RawURLEncoding.DecodeString(h.Signature)
	if err != nil || len(sig) != 64 {
		t.Fatalf("signature decode = (%d bytes, %v), want 64 bytes", len(sig), err)
	}

	// Two successive signatures use distinct nonces (freshness).
	h2, _ := signer.signCanonical("POST", "/v1/push", []byte(`{}`))
	if h2.Nonce == h.Nonce {
		t.Fatal("nonce is not fresh per request")
	}
}

// TestPublicJWKShape pins the challenge public key to {kty,crv,x,y} with 32-byte
// big-endian base64url (no padding) coordinates the gateway will accept.
func TestPublicJWKShape(t *testing.T) {
	signer := newTestSigner(t)
	jwk := signer.publicJWK()
	if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
		t.Fatalf("jwk kty/crv = %q/%q", jwk["kty"], jwk["crv"])
	}
	for _, k := range []string{"x", "y"} {
		b, err := base64.RawURLEncoding.DecodeString(jwk[k])
		if err != nil {
			t.Fatalf("jwk %s not base64url no-pad: %v", k, err)
		}
		if len(b) != 32 {
			t.Fatalf("jwk %s len = %d, want 32", k, len(b))
		}
	}
}

// TestSigningKeyPersistence pins that the key is generated once and reloaded:
// a second signer on the same path yields the same public key.
func TestSigningKeyPersistence(t *testing.T) {
	path := filepath.Join(t.TempDir(), pushSigningKeyFile)
	a, err := newPushSigner(path, "https://push.hotline.dev", nil, nil)
	if err != nil {
		t.Fatalf("first signer: %v", err)
	}
	b, err := newPushSigner(path, "https://push.hotline.dev", nil, nil)
	if err != nil {
		t.Fatalf("second signer: %v", err)
	}
	if a.priv.X.Cmp(b.priv.X) != 0 || a.priv.Y.Cmp(b.priv.Y) != 0 || a.priv.D.Cmp(b.priv.D) != 0 {
		t.Fatal("reloaded signing key differs from the persisted one")
	}
}

// TestValidAPNsToken pins hex-token acceptance/rejection to the gateway's
// IOS_TOKEN_RE + length rules (index.ts validateToken): lowercase hex, even
// length, 1..200 chars.
func TestValidAPNsToken(t *testing.T) {
	cases := []struct {
		tok  string
		want bool
	}{
		{"aabbccdd", true},
		{strings.Repeat("0a", 32), true},   // 64 hex chars, typical APNs token
		{strings.Repeat("ff", 100), true},  // 200 chars (the cap)
		{strings.Repeat("ff", 101), false}, // 202 chars, over the cap
		{"AABBCC", false},                  // uppercase rejected
		{"aabbc", false},                   // odd length
		{"aabbcg", false},                  // non-hex char
		{"", false},                        // empty
		{"ExponentPushToken[abc]", false},  // an Expo token is not a hex token
	}
	for _, c := range cases {
		if got := validAPNsToken(c.tok); got != c.want {
			t.Fatalf("validAPNsToken(%q) = %v, want %v", c.tok, got, c.want)
		}
	}
}

// TestDropPushTokenAndKeyID pins the 410 token-drop and key-id lifecycle in the
// store: SetPushKeyID surfaces via ActivePushTarget; DropPushToken clears the
// token+key so a subsequent push is suppressed and the state is "dropped".
func TestDropPushTokenAndKeyID(t *testing.T) {
	store, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatalf("OpenRelayStore: %v", err)
	}
	link, err := store.MintLink("ws://fixture.invalid", "pi")
	if err != nil {
		t.Fatalf("MintLink: %v", err)
	}
	const deviceID = "dev-abc123"
	res, linked, err := store.VerifyAndLink(link.Room, deviceID, link.Secret)
	if err != nil || res != VerifyActive || !linked {
		t.Fatalf("VerifyAndLink = (%v,%v,%v)", res, linked, err)
	}

	const token = "aabbccddeeff0011"
	if err := store.SetPush(deviceID, token, "ios"); err != nil {
		t.Fatalf("SetPush: %v", err)
	}
	if err := store.SetPushKeyID(deviceID, "keyid-xyz"); err != nil {
		t.Fatalf("SetPushKeyID: %v", err)
	}
	tok, keyID, room, ok := store.ActivePushTarget(deviceID)
	if !ok || tok != token || keyID != "keyid-xyz" || room != link.Room {
		t.Fatalf("ActivePushTarget = (%q,%q,%q,%v), want (%q,%q,%q,true)", tok, keyID, room, ok, token, "keyid-xyz", link.Room)
	}

	if err := store.DropPushToken(deviceID); err != nil {
		t.Fatalf("DropPushToken: %v", err)
	}
	tok, keyID, _, _ = store.ActivePushTarget(deviceID)
	if tok != "" || keyID != "" {
		t.Fatalf("after drop, token/keyID = (%q,%q), want empty", tok, keyID)
	}
	d, _ := store.Device(deviceID)
	if d.PushRegState != "dropped" {
		t.Fatalf("after drop, PushRegState = %q, want dropped", d.PushRegState)
	}

	// A changed token invalidates a stale key_id (re-registration required).
	if err := store.SetPush(deviceID, "0011223344556677", "ios"); err != nil {
		t.Fatalf("SetPush(new): %v", err)
	}
	if err := store.SetPushKeyID(deviceID, "keyid-1"); err != nil {
		t.Fatalf("SetPushKeyID: %v", err)
	}
	if err := store.SetPush(deviceID, "8899aabbccddeeff", "ios"); err != nil {
		t.Fatalf("SetPush(changed): %v", err)
	}
	if _, keyID, _, _ := store.ActivePushTarget(deviceID); keyID != "" {
		t.Fatalf("changed token kept stale key_id %q", keyID)
	}
}

// TestDeliverChallengeUnblocksRegister pins the nonce relay: a challenge
// delivered for a device reaches a waiter registered under that device id.
func TestDeliverChallengeUnblocksRegister(t *testing.T) {
	signer := newTestSigner(t)
	ch := make(chan challengeRelay, 1)
	signer.waitMu.Lock()
	signer.waiters["dev-abc123"] = ch
	signer.waitMu.Unlock()

	signer.deliverChallenge("dev-abc123", challengeRelay{challengeID: "cid", nonce: "the-nonce"})
	select {
	case got := <-ch:
		if got.nonce != "the-nonce" || got.challengeID != "cid" {
			t.Fatalf("relayed = %+v", got)
		}
	case <-time.After(time.Second):
		t.Fatal("challenge was not relayed to the waiter")
	}

	// Delivering to an unknown device is a harmless no-op.
	signer.deliverChallenge("dev-unknown0", challengeRelay{nonce: "x"})
}
