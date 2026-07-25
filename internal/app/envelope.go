package app

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"

	"golang.org/x/crypto/chacha20poly1305"
	"golang.org/x/crypto/hkdf"
)

// This file is the box's production implementation of the core-v1 e1 E2E
// envelope (protocol/core-v1/SPEC.md §1). The crypto is byte-identical to the
// reference implementation in protocol/core-v1/ref/ref.go and is gated on the
// same golden vectors (envelope-e1.json) by envelope_test.go. golang.org/x/crypto
// only — no hand-rolled primitives.
//
// The envelope is a pure transport layer: it wraps/unwraps the UTF-8 JSON bytes
// of one unchanged v2 frame at the connector's socket read/write choke points.
// The inner v2 protocol code never sees it.

// HKDF salt for every core-v1 derivation (SPEC §1.1).
const envelopeHKDFSalt = "hotline-e1"

// Info labels (SPEC §1.1). The '|' is a literal byte in the label.
const (
	envInfoB2A      = "hotline-e2e-v1|b2a"
	envInfoA2B      = "hotline-e2e-v1|a2b"
	envInfoRoomAuth = "hotline-room-auth-v1"
)

// decodePairSecret base64url-decodes the 43-char pairing secret into its 32 raw
// bytes (SPEC §1.1).
func decodePairSecret(s string) ([]byte, error) {
	raw, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("decode pairing secret: %w", err)
	}
	if len(raw) != 32 {
		return nil, fmt.Errorf("pairing secret is %d bytes, want 32", len(raw))
	}
	return raw, nil
}

// hkdfDerive runs one HKDF-SHA256 derivation over the raw secret with the fixed
// core-v1 salt and the given info label, returning a 32-byte key.
func hkdfDerive(secret []byte, info string) ([]byte, error) {
	buf := make([]byte, 32)
	r := hkdf.New(sha256.New, secret, []byte(envelopeHKDFSalt), []byte(info))
	if _, err := io.ReadFull(r, buf); err != nil {
		return nil, err
	}
	return buf, nil
}

// deriveRoomAuthHash derives room_auth from the pairing secret and returns
// base64url(SHA-256(room_auth)) — the only derivation-related value the core
// ever sees (SPEC §1.1, §2.2). Used to build the register auth_hash.
func deriveRoomAuthHash(secret string) (string, error) {
	raw, err := decodePairSecret(secret)
	if err != nil {
		return "", err
	}
	roomAuth, err := hkdfDerive(raw, envInfoRoomAuth)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256(roomAuth)
	return base64.RawURLEncoding.EncodeToString(sum[:]), nil
}

// envelopeCodec wraps/unwraps e1 frames for one room. It serves BOTH legs of the
// pairing, selected by `device`:
//   - BOX leg (device=false, the "c" side): SEALS box→app frames under K_b2a and
//     OPENS app→box frames under K_a2b.
//   - DEVICE leg (device=true, the "a" side, A2A v2 §3.2/F1): SEALS app→box frames
//     under K_a2b and OPENS box→app frames under K_b2a — the mirror image, so the
//     fleet dialer speaks e=1 as the device against a peer's fleet handler.
//
// The keys and AAD labels are byte-identical across the two legs (core-v1 SPEC §1);
// only the send/receive direction is swapped, so the same golden vectors gate both.
type envelopeCodec struct {
	room   string
	kB2A   []byte // box→app
	kA2B   []byte // app→box
	device bool   // false = box/"c" leg; true = device/"a" leg (keys swapped)
}

// newEnvelopeCodec derives the directional content keys for an envelope-mode
// room from its stored pairing secret (the BOX leg).
func newEnvelopeCodec(room RoomRecord) (*envelopeCodec, error) {
	if room.Secret == "" {
		return nil, fmt.Errorf("envelope room %s has no stored secret", room.ID)
	}
	return newEnvelopeCodecFromSecret(room.ID, room.Secret, false)
}

// newDeviceEnvelopeCodec derives the DEVICE-leg codec (A2A v2 §3.2, F1) from a
// dial edge's stored pairing secret: it seals under K_a2b and opens under K_b2a,
// so the fleet dialer is the "a" leg of the same e1 room the serve box answers.
func newDeviceEnvelopeCodec(roomID, secret string) (*envelopeCodec, error) {
	if secret == "" {
		return nil, fmt.Errorf("fleet device room %s has no stored secret", roomID)
	}
	return newEnvelopeCodecFromSecret(roomID, secret, true)
}

// newEnvelopeCodecFromSecret is the shared derivation for either leg.
func newEnvelopeCodecFromSecret(roomID, secret string, device bool) (*envelopeCodec, error) {
	raw, err := decodePairSecret(secret)
	if err != nil {
		return nil, err
	}
	kB2A, err := hkdfDerive(raw, envInfoB2A)
	if err != nil {
		return nil, err
	}
	kA2B, err := hkdfDerive(raw, envInfoA2B)
	if err != nil {
		return nil, err
	}
	return &envelopeCodec{room: roomID, kB2A: kB2A, kA2B: kA2B, device: device}, nil
}

// aad builds the additional authenticated data for an e1 frame:
// ASCII "hotline/e1|<room>|<dir>", dir ∈ {"b2a","a2b"} (SPEC §1). No escaping;
// room ids match [A-Za-z0-9_-]{22} and cannot contain '|'.
func (c *envelopeCodec) aad(dir string) []byte {
	return []byte("hotline/e1|" + c.room + "|" + dir)
}

// sealE1 encrypts inner under key with the given 24-byte nonce and AAD,
// returning the base64url "n" and "c" values of an e1 frame (SPEC §1).
func sealE1(key, nonce, aad, inner []byte) (n, c string, err error) {
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return "", "", err
	}
	if len(nonce) != chacha20poly1305.NonceSizeX {
		return "", "", fmt.Errorf("nonce is %d bytes, want %d", len(nonce), chacha20poly1305.NonceSizeX)
	}
	ct := aead.Seal(nil, nonce, inner, aad)
	return base64.RawURLEncoding.EncodeToString(nonce), base64.RawURLEncoding.EncodeToString(ct), nil
}

// openE1 decrypts the base64url n/c of an e1 frame body under key and AAD. Any
// failure returns an error; per SPEC §1 the receiver drops the frame.
func openE1(key []byte, n, c string, aad []byte) ([]byte, error) {
	nonce, err := base64.RawURLEncoding.DecodeString(n)
	if err != nil {
		return nil, err
	}
	if len(nonce) != chacha20poly1305.NonceSizeX {
		return nil, fmt.Errorf("nonce is %d bytes, want %d", len(nonce), chacha20poly1305.NonceSizeX)
	}
	ct, err := base64.RawURLEncoding.DecodeString(c)
	if err != nil {
		return nil, err
	}
	aead, err := chacha20poly1305.NewX(key)
	if err != nil {
		return nil, err
	}
	return aead.Open(nil, nonce, ct, aad)
}

// wrap seals one inner v2 frame (its exact JSON bytes) into a full e1 frame JSON
// object with a fresh 24-byte random nonce. Box leg: box→app under K_b2a. Device
// leg: app→box under K_a2b.
func (c *envelopeCodec) wrap(inner []byte) ([]byte, error) {
	key, dir := c.kB2A, "b2a"
	if c.device {
		key, dir = c.kA2B, "a2b"
	}
	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	n, ct, err := sealE1(key, nonce, c.aad(dir), inner)
	if err != nil {
		return nil, err
	}
	return json.Marshal(map[string]string{"t": "e1", "n": n, "c": ct})
}

// unwrap opens an e1 frame back to the inner v2 frame bytes. Box leg: app→box
// under K_a2b. Device leg: box→app under K_b2a. Any failure (not an e1 frame,
// bad encoding, wrong key/AAD, tampered bytes) returns an error; per SPEC §1 the
// receiver drops such a frame — so a plaintext frame never reaches the handler.
func (c *envelopeCodec) unwrap(frame []byte) ([]byte, error) {
	var f struct {
		T string `json:"t"`
		N string `json:"n"`
		C string `json:"c"`
	}
	if err := json.Unmarshal(frame, &f); err != nil {
		return nil, err
	}
	if f.T != "e1" {
		return nil, fmt.Errorf("non-e1 frame on envelope room")
	}
	key, dir := c.kA2B, "a2b"
	if c.device {
		key, dir = c.kB2A, "b2a"
	}
	return openE1(key, f.N, f.C, c.aad(dir))
}
