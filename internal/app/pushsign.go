package app

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/big"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// pushSigningKeyFile is the persisted P-256 signing key for the push gateway,
// created once under the instance StateDir and reused for every signed request.
const pushSigningKeyFile = "push-signing-key.json"

// challengeTTL bounds how long register() waits for the app to relay the silent
// challenge nonce back over the WebSocket before giving up (gateway challenge
// lifetime is 5 minutes; PROTOCOL.md §3).
const challengeTTL = 5 * time.Minute

// signedHeaders carries the four X-Hotline-* values a signed request needs (the
// Key-Id is added by the caller for /v1/push, which is not part of the signed
// canonical string).
type signedHeaders struct {
	Timestamp string // X-Hotline-Timestamp: unix seconds, decimal
	Nonce     string // X-Hotline-Nonce: fresh base64url, 8..64 chars
	Signature string // X-Hotline-Signature: base64url of 64-byte r||s (86 chars)
}

// challengeRelay is the {challenge_id, nonce} the app relays back after the
// gateway's silent registration push reaches the device.
type challengeRelay struct {
	challengeID string
	nonce       string
}

// pushSigner owns the box's P-256 signing key and the registration lifecycle
// against the push gateway. It exists only in gateway mode (HOTLINE_PUSH_ENDPOINT
// set); the legacy Expo path never constructs one.
type pushSigner struct {
	priv     *ecdsa.PrivateKey
	endpoint string
	client   *http.Client
	store    *RelayStore

	// waiters keys a per-device channel that a relayed challenge nonce is
	// delivered on; inflight guards against launching a second registration for
	// a device while one is already in progress.
	waitMu   sync.Mutex
	waiters  map[string]chan challengeRelay
	inflight map[string]bool
}

// storedKey is the on-disk representation of the signing key: a private EC JWK.
// x/y/d are base64url (no padding) of the 32-byte big-endian scalars.
type storedKey struct {
	Kty string `json:"kty"`
	Crv string `json:"crv"`
	D   string `json:"d"`
	X   string `json:"x"`
	Y   string `json:"y"`
}

// newPushSigner loads the persisted signing key or generates and atomically
// persists a fresh one, then returns a signer bound to the gateway endpoint.
func newPushSigner(path, endpoint string, store *RelayStore, client *http.Client) (*pushSigner, error) {
	priv, err := loadOrCreateSigningKey(path)
	if err != nil {
		return nil, err
	}
	if client == nil {
		client = &http.Client{Timeout: pushTimeout}
	}
	return &pushSigner{
		priv:     priv,
		endpoint: strings.TrimRight(endpoint, "/"),
		client:   client,
		store:    store,
		waiters:  map[string]chan challengeRelay{},
		inflight: map[string]bool{},
	}, nil
}

// loadOrCreateSigningKey reads the P-256 private key from path, or generates a
// new one and persists it (mode 0600, atomic rename) when the file is absent.
func loadOrCreateSigningKey(path string) (*ecdsa.PrivateKey, error) {
	data, err := os.ReadFile(path)
	if err == nil {
		var sk storedKey
		if err := json.Unmarshal(data, &sk); err != nil {
			return nil, fmt.Errorf("decode push signing key: %w", err)
		}
		return privFromStored(sk)
	}
	if !os.IsNotExist(err) {
		return nil, err
	}
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, err
	}
	sk := storedKey{
		Kty: "EC", Crv: "P-256",
		D: scalarBase64URL(priv.D),
		X: scalarBase64URL(priv.X),
		Y: scalarBase64URL(priv.Y),
	}
	body, err := json.MarshalIndent(sk, "", "  ")
	if err != nil {
		return nil, err
	}
	body = append(body, '\n')
	if err := atomicWriteFile(path, body); err != nil {
		return nil, err
	}
	return priv, nil
}

// privFromStored reconstructs the private key from the stored scalar d,
// recomputing the public point rather than trusting the persisted x/y.
func privFromStored(sk storedKey) (*ecdsa.PrivateKey, error) {
	if sk.Kty != "EC" || sk.Crv != "P-256" {
		return nil, errors.New("push signing key is not EC P-256")
	}
	dBytes, err := base64.RawURLEncoding.DecodeString(sk.D)
	if err != nil {
		return nil, fmt.Errorf("decode push signing key scalar: %w", err)
	}
	curve := elliptic.P256()
	priv := &ecdsa.PrivateKey{D: new(big.Int).SetBytes(dBytes)}
	priv.PublicKey.Curve = curve
	priv.PublicKey.X, priv.PublicKey.Y = curve.ScalarBaseMult(dBytes)
	if priv.PublicKey.X == nil {
		return nil, errors.New("invalid push signing key scalar")
	}
	return priv, nil
}

// scalarBase64URL encodes a curve scalar as base64url (no padding) of its
// fixed 32-byte big-endian representation.
func scalarBase64URL(v *big.Int) string {
	b := make([]byte, 32)
	v.FillBytes(b)
	return base64.RawURLEncoding.EncodeToString(b)
}

// publicJWK is the {kty,crv,x,y} public key sent in the registration challenge.
func (k *pushSigner) publicJWK() map[string]string {
	return map[string]string{
		"kty": "EC", "crv": "P-256",
		"x": scalarBase64URL(k.priv.PublicKey.X),
		"y": scalarBase64URL(k.priv.PublicKey.Y),
	}
}

// canonicalString builds the exact byte string the gateway verifies: five
// fields joined by a single '\n', no trailing newline (PROTOCOL.md §1.1).
func canonicalString(method, path, timestamp, nonce, bodyHash string) string {
	return strings.Join([]string{method, path, timestamp, nonce, bodyHash}, "\n")
}

// bodyHashBase64URL is base64url (no padding) of SHA-256 of the exact body bytes.
func bodyHashBase64URL(body []byte) string {
	sum := sha256.Sum256(body)
	return base64.RawURLEncoding.EncodeToString(sum[:])
}

// signCanonical produces the signed headers for (method, path, body): a fresh
// timestamp (unix seconds) and nonce, and a LOW-S normalized ECDSA signature
// over SHA-256 of the canonical string (PROTOCOL.md §1, §2).
func (k *pushSigner) signCanonical(method, path string, body []byte) (signedHeaders, error) {
	return signRequest(k.priv, method, path, body)
}

// signRequest is the shared box→core / box→gateway request signer: it produces a
// fresh timestamp (unix seconds) and nonce and a LOW-S normalized ECDSA signature
// over SHA-256 of the canonical string (PROTOCOL.md §1, §2 / core-v1 SPEC §2.1).
// Both the push gateway signer and the core client use it so the signing scheme
// lives in exactly one place.
func signRequest(priv *ecdsa.PrivateKey, method, path string, body []byte) (signedHeaders, error) {
	ts := strconv.FormatInt(time.Now().Unix(), 10)
	nonce, err := randomBase64URL(16) // 22 chars, within the 8..64 bound
	if err != nil {
		return signedHeaders{}, err
	}
	canon := canonicalString(method, path, ts, nonce, bodyHashBase64URL(body))
	sig, err := signLowS(priv, []byte(canon))
	if err != nil {
		return signedHeaders{}, err
	}
	return signedHeaders{Timestamp: ts, Nonce: nonce, Signature: sig}, nil
}

// signLowS signs SHA-256(data) with ECDSA P-256 and returns base64url (no
// padding) of the 64-byte r||s signature, with s MANDATORY-normalized to the
// low-S form (s <= n/2). This normalization is verbatim per PROTOCOL.md §2.2:
// Go's ecdsa.Sign yields a high-S signature ~50% of the time and the gateway
// rejects those, so it must be applied on every signature.
func signLowS(priv *ecdsa.PrivateKey, data []byte) (string, error) {
	digest := sha256.Sum256(data)
	r, s, err := ecdsa.Sign(rand.Reader, priv, digest[:])
	if err != nil {
		return "", err
	}
	n := priv.Params().N
	halfN := new(big.Int).Rsh(n, 1) // n/2
	if s.Cmp(halfN) > 0 {
		s.Sub(n, s) // s = n - s
	}
	sig := make([]byte, 64)
	r.FillBytes(sig[0:32])
	s.FillBytes(sig[32:64])
	return base64.RawURLEncoding.EncodeToString(sig), nil
}

// register runs the challenge→complete handshake for a device's push token and,
// on success, persists the returned key_id. It is idempotent per device: a
// second call while one is in flight returns immediately. The relayed challenge
// nonce is awaited on a per-device channel (fed by deliverChallenge) with a
// 5-minute TTL.
func (k *pushSigner) register(ctx context.Context, deviceID, token string) error {
	k.waitMu.Lock()
	if k.inflight[deviceID] {
		k.waitMu.Unlock()
		return nil
	}
	k.inflight[deviceID] = true
	ch := make(chan challengeRelay, 1)
	k.waiters[deviceID] = ch
	k.waitMu.Unlock()
	defer func() {
		k.waitMu.Lock()
		delete(k.waiters, deviceID)
		delete(k.inflight, deviceID)
		k.waitMu.Unlock()
	}()

	challengeID, err := k.postChallenge(ctx, token)
	if err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(challengeTTL):
		return errors.New("push registration timed out awaiting challenge nonce")
	case relay := <-ch:
		keyID, err := k.postComplete(ctx, token, challengeID, relay.nonce)
		if err != nil {
			return err
		}
		return k.store.SetPushKeyID(deviceID, keyID)
	}
}

// deliverChallenge hands a relayed {challenge_id, nonce} to a register() call
// waiting on this device, if any. It never blocks.
func (k *pushSigner) deliverChallenge(deviceID string, relay challengeRelay) {
	k.waitMu.Lock()
	ch := k.waiters[deviceID]
	k.waitMu.Unlock()
	if ch == nil {
		return
	}
	select {
	case ch <- relay:
	default:
	}
}

// postChallenge POSTs the unsigned /v1/registrations/challenge with the public
// JWK and returns the challenge_id. It retries on 429/503 with backoff.
func (k *pushSigner) postChallenge(ctx context.Context, token string) (string, error) {
	body, err := json.Marshal(map[string]any{
		"platform":   "ios",
		"token":      token,
		"public_key": k.publicJWK(),
	})
	if err != nil {
		return "", err
	}
	backoff := 2 * time.Second
	for attempt := 0; ; attempt++ {
		id, retry, err := k.tryChallenge(ctx, body)
		if err == nil {
			return id, nil
		}
		if !retry || attempt >= 3 {
			return "", err
		}
		select {
		case <-ctx.Done():
			return "", ctx.Err()
		case <-time.After(backoff):
		}
		backoff *= 2
	}
}

// tryChallenge performs one challenge POST. retry is true for 429/503 (the
// caller should back off and retry).
func (k *pushSigner) tryChallenge(ctx context.Context, body []byte) (id string, retry bool, err error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, k.endpoint+"/v1/registrations/challenge", bytes.NewReader(body))
	if err != nil {
		return "", false, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := k.client.Do(req)
	if err != nil {
		return "", true, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusOK {
		var r struct {
			ChallengeID string `json:"challenge_id"`
		}
		if err := json.NewDecoder(io.LimitReader(resp.Body, 4096)).Decode(&r); err != nil || r.ChallengeID == "" {
			return "", false, errors.New("challenge response missing challenge_id")
		}
		return r.ChallengeID, false, nil
	}
	retry = resp.StatusCode == http.StatusTooManyRequests || resp.StatusCode == http.StatusServiceUnavailable
	return "", retry, fmt.Errorf("challenge rejected (%d): %s", resp.StatusCode, readCode(resp.Body))
}

// postComplete POSTs the signed /v1/registrations/complete and returns key_id.
func (k *pushSigner) postComplete(ctx context.Context, token, challengeID, nonce string) (string, error) {
	body, err := json.Marshal(map[string]any{
		"platform":     "ios",
		"token":        token,
		"challenge_id": challengeID,
		"nonce":        nonce,
	})
	if err != nil {
		return "", err
	}
	const path = "/v1/registrations/complete"
	h, err := k.signCanonical(http.MethodPost, path, body)
	if err != nil {
		return "", err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, k.endpoint+path, bytes.NewReader(body))
	if err != nil {
		return "", err
	}
	req.Header.Set("Content-Type", "application/json")
	applySignedHeaders(req, h)
	resp, err := k.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("complete rejected (%d): %s", resp.StatusCode, readCode(resp.Body))
	}
	var r struct {
		KeyID string `json:"key_id"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 4096)).Decode(&r); err != nil || r.KeyID == "" {
		return "", errors.New("complete response missing key_id")
	}
	return r.KeyID, nil
}

// applySignedHeaders sets the three signed X-Hotline-* headers on a request.
func applySignedHeaders(req *http.Request, h signedHeaders) {
	req.Header.Set("X-Hotline-Timestamp", h.Timestamp)
	req.Header.Set("X-Hotline-Nonce", h.Nonce)
	req.Header.Set("X-Hotline-Signature", h.Signature)
}

// readCode returns the gateway error "code" (if any) for logging, capped.
func readCode(body io.Reader) string {
	var r struct {
		Code string `json:"code"`
	}
	if json.NewDecoder(io.LimitReader(body, 512)).Decode(&r) == nil && r.Code != "" {
		return r.Code
	}
	return "unknown"
}

// atomicWriteFile writes data to path via a temp file + rename, mode 0600,
// mirroring RelayStore.saveLocked so the key file is written atomically and
// owner-only.
func atomicWriteFile(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".push-signing-key-*")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, path); err != nil {
		return err
	}
	return os.Chmod(path, 0o600)
}
