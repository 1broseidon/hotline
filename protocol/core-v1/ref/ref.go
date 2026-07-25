// Package corev1ref is the reference implementation of the hotline core-v1
// protocol primitives (protocol/core-v1/SPEC.md): the e1 E2E envelope
// (HKDF-SHA256 key derivation + XChaCha20-Poly1305 AEAD) and the signed
// control-plane header scheme (P-256 ECDSA / SHA-256, canonical string,
// mandatory low-S).
//
// This package exists to generate and validate the golden fixtures under
// protocol/core-v1/fixtures/. It is normative for byte layout: WP1 (worker),
// WP2 (box) and WP3 (app) implementations MUST reproduce these bytes exactly.
// No production code imports it.
package corev1ref

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"math/big"
	"strings"

	"golang.org/x/crypto/chacha20poly1305"
	"golang.org/x/crypto/hkdf"
)

// HKDF salt for every core-v1 derivation (SPEC §2.3).
const hkdfSalt = "hotline-e1"

// Info labels (SPEC §2.3). The '|' is a literal byte in the label.
const (
	InfoB2A      = "hotline-e2e-v1|b2a"
	InfoA2B      = "hotline-e2e-v1|a2b"
	InfoRoomAuth = "hotline-room-auth-v1"
)

// Keys holds the three 32-byte HKDF outputs derived from the pairing secret.
type Keys struct {
	KB2A     []byte // box→app content key
	KA2B     []byte // app→box content key
	RoomAuth []byte // future device-direct auth (§8.1); only its hash leaves the pair
}

// DecodeSecret base64url-decodes the 43-char pairing secret into its 32 raw bytes.
func DecodeSecret(s string) ([]byte, error) {
	raw, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("decode pairing secret: %w", err)
	}
	if len(raw) != 32 {
		return nil, fmt.Errorf("pairing secret is %d bytes, want 32", len(raw))
	}
	return raw, nil
}

// DeriveKeys runs the three HKDF-SHA256 derivations of SPEC §2.3 over the raw
// 32-byte secret. salt = ASCII "hotline-e1" for all three; only info differs.
func DeriveKeys(secret []byte) (Keys, error) {
	var k Keys
	for _, d := range []struct {
		out  *[]byte
		info string
	}{
		{&k.KB2A, InfoB2A},
		{&k.KA2B, InfoA2B},
		{&k.RoomAuth, InfoRoomAuth},
	} {
		buf := make([]byte, 32)
		r := hkdf.New(sha256.New, secret, []byte(hkdfSalt), []byte(d.info))
		if _, err := io.ReadFull(r, buf); err != nil {
			return Keys{}, err
		}
		*d.out = buf
	}
	return k, nil
}

// AuthHash is base64url(SHA-256(room_auth)) — the only derivation-related
// value the core ever sees (SPEC §2.3, §3.2).
func AuthHash(roomAuth []byte) string {
	sum := sha256.Sum256(roomAuth)
	return base64.RawURLEncoding.EncodeToString(sum[:])
}

// AAD builds the additional authenticated data string for an e1 frame:
// ASCII "hotline/e1|<room>|<dir>", dir ∈ {"b2a","a2b"} (SPEC §2.2).
func AAD(room, dir string) ([]byte, error) {
	if dir != "b2a" && dir != "a2b" {
		return nil, fmt.Errorf("bad envelope direction %q", dir)
	}
	return []byte("hotline/e1|" + room + "|" + dir), nil
}

// Seal encrypts one inner v2 frame (its exact UTF-8 JSON bytes) into an e1
// frame body using XChaCha20-Poly1305 with the given 24-byte nonce and AAD.
// It returns the JSON values for "n" and "c" (base64url, no padding).
func Seal(key, nonce, aad, plaintext []byte) (n, c string, err error) {
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return "", "", err
	}
	if len(nonce) != chacha20poly1305.NonceSizeX {
		return "", "", fmt.Errorf("nonce is %d bytes, want %d", len(nonce), chacha20poly1305.NonceSizeX)
	}
	ct := aead.Seal(nil, nonce, plaintext, aad)
	return base64.RawURLEncoding.EncodeToString(nonce),
		base64.RawURLEncoding.EncodeToString(ct), nil
}

// Open decrypts an e1 frame body (base64url n and c) back to the inner frame
// bytes. Any failure (bad encoding, wrong key, wrong AAD, tampered bytes)
// returns an error; per SPEC §2.2 the receiver drops the frame.
func Open(key []byte, n, c string, aad []byte) ([]byte, error) {
	nonce, err := base64.RawURLEncoding.DecodeString(n)
	if err != nil {
		return nil, fmt.Errorf("decode nonce: %w", err)
	}
	if len(nonce) != chacha20poly1305.NonceSizeX {
		return nil, fmt.Errorf("nonce is %d bytes, want %d", len(nonce), chacha20poly1305.NonceSizeX)
	}
	ct, err := base64.RawURLEncoding.DecodeString(c)
	if err != nil {
		return nil, fmt.Errorf("decode ciphertext: %w", err)
	}
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return nil, err
	}
	return aead.Open(nil, nonce, ct, aad)
}

// --- signing (verbatim hotline-push-gateway PROTOCOL.md §1–2 / pushsign.go) ---

// CanonicalString joins the five signed fields with single '\n', no trailing
// newline: METHOD, path, timestamp (unix s decimal), nonce,
// base64url(SHA-256(exact body bytes)).
func CanonicalString(method, path, timestamp, nonce string, body []byte) string {
	sum := sha256.Sum256(body)
	bodyHash := base64.RawURLEncoding.EncodeToString(sum[:])
	return strings.Join([]string{method, path, timestamp, nonce, bodyHash}, "\n")
}

// SignLowS signs SHA-256(data) with ECDSA P-256 and returns base64url (no
// padding) of the 64-byte r||s signature with s normalized to low-S
// (s <= n/2), exactly as internal/app/pushsign.go signLowS.
func SignLowS(priv *ecdsa.PrivateKey, data []byte) (string, error) {
	digest := sha256.Sum256(data)
	r, s, err := ecdsa.Sign(rand.Reader, priv, digest[:])
	if err != nil {
		return "", err
	}
	n := priv.Params().N
	halfN := new(big.Int).Rsh(n, 1)
	if s.Cmp(halfN) > 0 {
		s.Sub(n, s)
	}
	sig := make([]byte, 64)
	r.FillBytes(sig[0:32])
	s.FillBytes(sig[32:64])
	return base64.RawURLEncoding.EncodeToString(sig), nil
}

// VerifyLowS verifies a base64url 64-byte r||s signature over SHA-256(data),
// rejecting high-S signatures (the core's verify rule: WebCrypto verify plus
// an explicit low-S check).
func VerifyLowS(pub *ecdsa.PublicKey, data []byte, sigB64 string) error {
	sig, err := base64.RawURLEncoding.DecodeString(sigB64)
	if err != nil {
		return fmt.Errorf("decode signature: %w", err)
	}
	if len(sig) != 64 {
		return fmt.Errorf("signature is %d bytes, want 64", len(sig))
	}
	r := new(big.Int).SetBytes(sig[0:32])
	s := new(big.Int).SetBytes(sig[32:64])
	halfN := new(big.Int).Rsh(pub.Params().N, 1)
	if s.Cmp(halfN) > 0 {
		return errors.New("high-S signature rejected")
	}
	digest := sha256.Sum256(data)
	if !ecdsa.Verify(pub, digest[:], r, s) {
		return errors.New("bad signature")
	}
	return nil
}

// HighSTwin returns the malleable high-S twin of a valid low-S signature
// (s' = n - s), used by fixtures to assert the reject path.
func HighSTwin(sigB64 string) (string, error) {
	sig, err := base64.RawURLEncoding.DecodeString(sigB64)
	if err != nil || len(sig) != 64 {
		return "", errors.New("bad signature input")
	}
	n := elliptic.P256().Params().N
	s := new(big.Int).SetBytes(sig[32:64])
	s.Sub(n, s)
	out := make([]byte, 64)
	copy(out[0:32], sig[0:32])
	s.FillBytes(out[32:64])
	return base64.RawURLEncoding.EncodeToString(out), nil
}

// KeyFromScalar rebuilds a P-256 private key from a base64url 32-byte scalar
// (the fixture key), recomputing the public point.
func KeyFromScalar(dB64 string) (*ecdsa.PrivateKey, error) {
	dBytes, err := base64.RawURLEncoding.DecodeString(dB64)
	if err != nil {
		return nil, err
	}
	curve := elliptic.P256()
	d := new(big.Int).SetBytes(dBytes)
	if d.Sign() <= 0 || d.Cmp(curve.Params().N) >= 0 {
		return nil, errors.New("scalar out of range")
	}
	priv := &ecdsa.PrivateKey{D: d}
	priv.PublicKey.Curve = curve
	priv.PublicKey.X, priv.PublicKey.Y = curve.ScalarBaseMult(dBytes)
	return priv, nil
}

// ScalarB64 encodes a curve coordinate/scalar as base64url of its fixed
// 32-byte big-endian form (JWK x/y/d encoding).
func ScalarB64(v *big.Int) string {
	b := make([]byte, 32)
	v.FillBytes(b)
	return base64.RawURLEncoding.EncodeToString(b)
}
