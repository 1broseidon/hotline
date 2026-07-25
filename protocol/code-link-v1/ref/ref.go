// Package codelinkv1ref is the reference implementation of the hotline
// code-link-v1 crypto core (protocol/code-link-v1/SPEC.md): CPace over
// ristretto255 (CPACE-RISTRETTO255-SHA512, draft-irtf-cfrg-cpace), the hotline
// HKDF-SHA256 key schedule, the SHA-256/HMAC transcript-confirmation MACs, the
// XChaCha20-Poly1305 payload wrap, and Crockford base32 code normalization.
//
// It is PURE crypto — no I/O, no network, no persistence. No production code
// imports it; it exists to define byte layout and to generate/validate the
// golden fixtures under protocol/code-link-v1/fixtures/. WP-CL1 (TS twin),
// WP-CL2 (relay) and WP-CL3 (box CLI) MUST reproduce these bytes exactly.
//
// # Two layers
//
//   - Low-level CPace/KDF primitives reproduce the draft's CPace-Ristretto255
//     test vectors byte-exact (see draft_anchor.go and fixtures/cpace-r255.json).
//     The raw shared secret K is NOT exported: ScalarMultVfy returns an opaque
//     *SharedSecret, and ISK consumes only that — an identity/invalid K can
//     never reach the key schedule.
//   - The ROLE state machine (BoxInit/BoxAccepted for A=box, ClientResponder/
//     ClientAwaiting for B=client) structurally enforces the load-bearing
//     order: B proves knowledge first (confirm_b); A releases the payload ONLY
//     after confirm_b verifies; B verifies confirm_a BEFORE decrypting. Payload
//     seal/open exist only as methods on the post-verification states, so there
//     is no way to seal-before-verify or decrypt-before-verify.
package codelinkv1ref

import (
	"crypto/hmac"
	"crypto/sha256"
	"crypto/sha512"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net/url"
	"strings"

	"github.com/gtank/ristretto255"
	"golang.org/x/crypto/chacha20poly1305"
	"golang.org/x/crypto/hkdf"
)

// ---------------------------------------------------------------------------
// Suite constants (draft-irtf-cfrg-cpace, CPACE-RISTRETTO255-SHA512 + hotline).
// ---------------------------------------------------------------------------

const (
	// DSI is the CPace domain-separation identifier for the ristretto255
	// suite; the draft value, so the draft vectors apply verbatim.
	DSI = "CPaceRistretto255"
	// DSIISK is the DSI used inside the ISK derivation (G.DSI || "_ISK").
	DSIISK = DSI + "_ISK"
	// SHA512BlockBytes is H.s_in_bytes for SHA-512 (the input block size),
	// used to size the CPace generator-string zero padding.
	SHA512BlockBytes = 128
	// FieldSizeBytes is G.field_size_bytes for ristretto255 (element/scalar
	// encoding length).
	FieldSizeBytes = 32
	// UniformScalarBytes is the input length for uniform scalar sampling
	// (64 bytes reduced mod the group order).
	UniformScalarBytes = 64

	// HKDFSalt is the fixed salt for every code-link-v1 HKDF-SHA256 derivation.
	HKDFSalt = "hotline-code-link-v1"
	// Key-schedule info labels (SPEC §4.3).
	InfoConfirmB = "hcl-confirm-b" // K_cb: client->box confirmation MAC key
	InfoConfirmA = "hcl-confirm-a" // K_ca: box->client confirmation MAC key
	InfoPayload  = "hcl-payload"   // K_pay: AEAD key for the pair-URI payload

	// Framing prefixes (SPEC §4.3).
	THPrefix      = "hotline/code-link-v1|"         // transcript-hash framing
	PayloadAADPre = "hotline/code-link-v1|payload|" // payload AEAD AAD prefix
	CIPrefix      = "hotline/code-link-v1|"         // CPace CI = prefix + relay_host

	// MACBytes / KeyBytes / NonceBytes are the fixed wire lengths.
	MACBytes   = 32
	KeyBytes   = 32
	NonceBytes = chacha20poly1305.NonceSizeX // 24
)

// b64 is STRICT base64url without padding — the single house encoding for
// every wire value. Strict() rejects non-canonical trailing bits, so the four
// distinct final characters that would otherwise decode to the same bytes are
// rejected (a malleability gap the plain encoder leaves open).
var b64 = base64.RawURLEncoding.Strict()

// Errors. Any of ErrPeerEncoding, ErrPeerIdentity, ErrConfirmFailed returned
// on a role transition is a FAILED ATTEMPT against the box's 3-strike cap
// (design §4.6); use IsFailedAttempt to classify.
var (
	ErrPeerEncoding  = errors.New("code-link: peer element is not a canonical ristretto255 encoding")
	ErrPeerIdentity  = errors.New("code-link: peer/shared element is the group identity")
	ErrConfirmFailed = errors.New("code-link: confirmation MAC did not verify")
	ErrZeroScalar    = errors.New("code-link: sampled scalar is zero (resample)")
	ErrBadCode       = errors.New("code-link: not 12 Crockford base32 symbols")
	ErrPayloadOpen   = errors.New("code-link: payload AEAD open failed")
	ErrBadLength     = errors.New("code-link: wire value has the wrong decoded length")
)

// IsFailedAttempt reports whether err (from a Box/Client role transition) is a
// failed PAKE attempt that the box MUST count toward its strike cap.
func IsFailedAttempt(err error) bool {
	return errors.Is(err, ErrPeerEncoding) ||
		errors.Is(err, ErrPeerIdentity) ||
		errors.Is(err, ErrConfirmFailed)
}

func decodeExact(s string, n int) ([]byte, error) {
	raw, err := b64.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("base64url decode: %w", err)
	}
	if len(raw) != n {
		return nil, ErrBadLength
	}
	return raw, nil
}

// ---------------------------------------------------------------------------
// CPace low-level string building blocks (draft §A.1–A.3).
// ---------------------------------------------------------------------------

// prependLen returns the LEB128 encoding of len(data) followed by data
// (draft A.1.1).
func prependLen(data []byte) []byte {
	length := len(data)
	var enc []byte
	for {
		if length < 128 {
			enc = append(enc, byte(length))
		} else {
			enc = append(enc, byte((length&0x7f)+0x80))
		}
		length >>= 7
		if length == 0 {
			break
		}
	}
	return append(enc, data...)
}

// lvCat is the length-value concatenation of its arguments (draft A.1.3).
func lvCat(args ...[]byte) []byte {
	var out []byte
	for _, a := range args {
		out = append(out, prependLen(a)...)
	}
	return out
}

// lexLarger reports whether a > b under lexicographical ordering (draft A.3.1).
func lexLarger(a, b []byte) bool {
	n := len(a)
	if len(b) < n {
		n = len(b)
	}
	for i := 0; i < n; i++ {
		if a[i] > b[i] {
			return true
		}
		if a[i] < b[i] {
			return false
		}
	}
	return len(a) > len(b)
}

// oCat is the ordered concatenation b"oc" + larger + smaller (draft A.3.2).
func oCat(a, b []byte) []byte {
	out := []byte("oc")
	if lexLarger(a, b) {
		return append(append(out, a...), b...)
	}
	return append(append(out, b...), a...)
}

// zeroBytes returns n zero octets (draft: zero_bytes).
func zeroBytes(n int) []byte { return make([]byte, n) }

// GeneratorString builds the CPace generator string
// lv_cat(DSI, PRS, zero_bytes(len_zpad), CI, sid) with the zero padding sized
// so DSI+PRS fill the first hash input block (draft §7.1, §A.2).
func GeneratorString(dsi, prs, ci, sid []byte, sInBytes int) []byte {
	lenZpad := sInBytes - 1 - len(prependLen(prs)) - len(prependLen(dsi))
	if lenZpad < 0 {
		lenZpad = 0
	}
	return lvCat(dsi, prs, zeroBytes(lenZpad), ci, sid)
}

// ---------------------------------------------------------------------------
// CPace group operations over ristretto255 (draft §7.3).
// ---------------------------------------------------------------------------

// CalculateGenerator returns the CPace generator _g as a decoded ristretto255
// element: _g = element_derivation(SHA-512(generator_string)) (draft §7.3).
func CalculateGenerator(prs, ci, sid []byte) (*ristretto255.Element, error) {
	gs := GeneratorString([]byte(DSI), prs, ci, sid, SHA512BlockBytes)
	h := sha512.Sum512(gs)
	// 2*field_size_bytes = 64 = the full SHA-512 output.
	return ristretto255.NewElement().SetUniformBytes(h[:])
}

// EncodeElement returns the 32-byte public encoding of an element.
func EncodeElement(e *ristretto255.Element) []byte { return e.Encode(nil) }

// ScalarMult returns encode(y * g) for a decoded generator g (draft:
// G.scalar_mult). Used by a party to compute its OWN message Ya/Yb (g is
// derived, y is the party's own fresh scalar — no peer validation applies).
func ScalarMult(y *ristretto255.Scalar, g *ristretto255.Element) []byte {
	return ristretto255.NewElement().ScalarMult(y, g).Encode(nil)
}

// SharedSecret is an opaque, VALIDATED CPace shared point K. It can only be
// produced by ScalarMultVfy (which rejects invalid encodings and the identity)
// and can only be consumed by ISK. The raw K bytes are never exported — draft
// §9.3: parties output the ISK, not K. (An in-package accessor exists solely to
// reproduce the draft's published K vector.)
type SharedSecret struct{ enc []byte }

func (s *SharedSecret) bytes() []byte { return s.enc }

// ScalarMultVfy implements G.scalar_mult_vfy(y, X) with the hotline abort
// rules (SPEC §4.1 / draft §B.3.11): decode(X) must be a valid, non-identity
// ristretto255 element and the resulting shared point y*decode(X) must be
// non-identity. On any failure it returns a nil *SharedSecret and an error the
// caller MUST treat as a failed attempt.
func ScalarMultVfy(y *ristretto255.Scalar, peerEnc []byte) (*SharedSecret, error) {
	if len(peerEnc) != FieldSizeBytes {
		return nil, ErrPeerEncoding
	}
	peer := ristretto255.NewElement()
	if _, err := peer.SetCanonicalBytes(peerEnc); err != nil {
		return nil, ErrPeerEncoding
	}
	id := ristretto255.NewIdentityElement()
	if peer.Equal(id) == 1 {
		return nil, ErrPeerIdentity
	}
	k := ristretto255.NewElement().ScalarMult(y, peer)
	if k.Equal(id) == 1 {
		return nil, ErrPeerIdentity
	}
	return &SharedSecret{enc: k.Encode(nil)}, nil
}

// TranscriptIR is the initiator-ordered transcript
// lv_cat(Ya,ADa) || lv_cat(Yb,ADb) (draft §A.3.4). A is always the box.
func TranscriptIR(ya, ada, yb, adb []byte) []byte {
	return append(lvCat(ya, ada), lvCat(yb, adb)...)
}

// TranscriptOC is the parallel/unordered transcript
// o_cat(lv_cat(Ya,ADa), lv_cat(Yb,ADb)) (draft §A.3.6). Provided to reproduce
// the draft's ISK_SY vector; hotline uses the initiator ordering.
func TranscriptOC(ya, ada, yb, adb []byte) []byte {
	return oCat(lvCat(ya, ada), lvCat(yb, adb))
}

// ISK computes the intermediate session key
// H.hash(lv_cat(G.DSI||"_ISK", sid, K) || transcript) (draft §6.2). It consumes
// a VALIDATED *SharedSecret, so an identity/invalid K can never be mixed in.
// Returns 64 bytes.
func ISK(sid []byte, ss *SharedSecret, transcript []byte) []byte {
	pre := lvCat([]byte(DSIISK), sid, ss.bytes())
	h := sha512.Sum512(append(pre, transcript...))
	return h[:]
}

// SidOutput is the optional derived session id
// H.hash("CPaceSidOutput" || transcript). Reproduces draft §B.3.7; unused by
// hotline.
func SidOutput(label string, transcript []byte) []byte {
	h := sha512.Sum512(append([]byte(label), transcript...))
	return h[:]
}

// ScalarFromCanonical decodes a 32-byte little-endian canonical scalar
// (value < group order). Used to load the draft's fixed test-vector scalars.
func ScalarFromCanonical(le []byte) (*ristretto255.Scalar, error) {
	return ristretto255.NewScalar().SetCanonicalBytes(le)
}

// ScalarFromUniform reduces 64 uniform random bytes mod the group order into a
// fresh session scalar (SPEC §4.1). SetUniformBytes gives a distribution
// statistically indistinguishable from uniform over [0, l) (bias ~2^-262.5).
// The one value that must not be used is 0 (probability ~2^-252), which would
// make Ya/Yb the identity; this returns ErrZeroScalar so the caller resamples.
func ScalarFromUniform(uniform []byte) (*ristretto255.Scalar, error) {
	if len(uniform) != UniformScalarBytes {
		return nil, fmt.Errorf("uniform is %d bytes, want %d", len(uniform), UniformScalarBytes)
	}
	s, err := ristretto255.NewScalar().SetUniformBytes(uniform)
	if err != nil {
		return nil, err
	}
	if s.Equal(ristretto255.NewScalar()) == 1 { // NewScalar() == 0
		return nil, ErrZeroScalar
	}
	return s, nil
}

// ---------------------------------------------------------------------------
// hotline key schedule (SPEC §4.3): HKDF-SHA256 over the ISK.
// ---------------------------------------------------------------------------

// SessionKeys are the three 32-byte keys derived from the ISK.
type SessionKeys struct {
	KCB  []byte // client->box confirmation MAC key
	KCA  []byte // box->client confirmation MAC key
	KPay []byte // pair-URI payload AEAD key
}

// DeriveSessionKeys runs the three HKDF-SHA256 derivations of SPEC §4.3 over
// the 64-byte ISK. salt = "hotline-code-link-v1"; only the info label differs.
func DeriveSessionKeys(isk []byte) (SessionKeys, error) {
	var k SessionKeys
	for _, d := range []struct {
		out  *[]byte
		info string
	}{
		{&k.KCB, InfoConfirmB},
		{&k.KCA, InfoConfirmA},
		{&k.KPay, InfoPayload},
	} {
		buf := make([]byte, KeyBytes)
		r := hkdf.New(sha256.New, isk, []byte(HKDFSalt), []byte(d.info))
		if _, err := io.ReadFull(r, buf); err != nil {
			return SessionKeys{}, err
		}
		*d.out = buf
	}
	return k, nil
}

// ---------------------------------------------------------------------------
// Transcript hash + confirmation MACs (SPEC §4.3).
// ---------------------------------------------------------------------------

// TranscriptHash computes th = SHA-256(THPrefix ‖ sid ‖ "|" ‖ msgA_b64 ‖ "|"
// ‖ msgB_b64) over the ASCII framing of the base64url messages. Returns the
// 32-byte digest.
func TranscriptHash(sid, msgAB64, msgBB64 string) []byte {
	s := THPrefix + sid + "|" + msgAB64 + "|" + msgBB64
	h := sha256.Sum256([]byte(s))
	return h[:]
}

// ConfirmMAC = base64url(HMAC-SHA256(key, th)).
func ConfirmMAC(key, th []byte) string {
	m := hmac.New(sha256.New, key)
	m.Write(th)
	return b64.EncodeToString(m.Sum(nil))
}

// VerifyConfirmMAC recomputes the MAC and compares in constant time. It is
// written so the work is independent of whether macB64 parses or has the right
// length: it decodes into a fixed 32-byte buffer, ALWAYS computes the HMAC,
// always compares two 32-byte values with subtle.ConstantTimeCompare, and
// folds the parse-valid bit into the result. Order is load-bearing (SPEC §4.3):
// the box MUST verify confirm_b before releasing anything; the client MUST
// verify confirm_a before trusting the payload. Prefer the role API, which
// enforces this structurally.
func VerifyConfirmMAC(key, th []byte, macB64 string) bool {
	var got [MACBytes]byte
	raw, err := b64.DecodeString(macB64)
	parseOK := 0
	if err == nil && len(raw) == MACBytes {
		parseOK = 1
		copy(got[:], raw)
	}
	m := hmac.New(sha256.New, key)
	m.Write(th)
	want := m.Sum(nil) // 32 bytes
	eq := subtle.ConstantTimeCompare(want, got[:])
	return parseOK&eq == 1
}

// ---------------------------------------------------------------------------
// Payload wrap (SPEC §4.3): XChaCha20-Poly1305 under K_pay. Unexported — the
// only sanctioned paths are (*BoxAccepted).SealPayload and
// (*ClientAwaiting).Finish, which run AFTER the confirmation checks.
// ---------------------------------------------------------------------------

// PayloadAAD is the AEAD associated data: ASCII PayloadAADPre ‖ sid.
func PayloadAAD(sid string) []byte { return []byte(PayloadAADPre + sid) }

func sealPayload(kPay, nonce, plaintext []byte, sid string) (n, c string, err error) {
	aead, err := chacha20poly1305.NewX(kPay)
	if err != nil {
		return "", "", err
	}
	if len(nonce) != NonceBytes {
		return "", "", fmt.Errorf("nonce is %d bytes, want %d", len(nonce), NonceBytes)
	}
	ct := aead.Seal(nil, nonce, plaintext, PayloadAAD(sid))
	return b64.EncodeToString(nonce), b64.EncodeToString(ct), nil
}

func openPayload(kPay []byte, n, c, sid string) ([]byte, error) {
	nonce, err := decodeExact(n, NonceBytes)
	if err != nil {
		return nil, ErrPayloadOpen
	}
	ct, err := b64.DecodeString(c)
	if err != nil || len(ct) < chacha20poly1305.Overhead {
		return nil, ErrPayloadOpen
	}
	aead, err := chacha20poly1305.NewX(kPay)
	if err != nil {
		return nil, err
	}
	pt, err := aead.Open(nil, nonce, ct, PayloadAAD(sid))
	if err != nil {
		return nil, ErrPayloadOpen
	}
	return pt, nil
}

// ---------------------------------------------------------------------------
// Role state machine — structurally enforces the load-bearing order (SPEC §4).
// ---------------------------------------------------------------------------

// BoxInit is party A (the box, initiator) after computing its message. It holds
// the secret scalar and the state needed to finish a session.
type BoxInit struct {
	ya    *ristretto255.Scalar
	sid   string
	msgA  string // base64url(Ya)
	yaEnc []byte // Ya encoding (32B)
}

// NewBoxInit derives the box's CPace generator and message from the code inputs
// and a fresh 64-byte uniform scalar seed. prs = normalized code, sid = channel
// ASCII, ci = BuildCI(host). Returns the box state and msg_a to POST.
func NewBoxInit(prs, ci []byte, sid string, uniformScalar []byte) (*BoxInit, error) {
	ya, err := ScalarFromUniform(uniformScalar)
	if err != nil {
		return nil, err
	}
	g, err := CalculateGenerator(prs, ci, []byte(sid))
	if err != nil {
		return nil, err
	}
	yaEnc := ScalarMult(ya, g)
	return &BoxInit{ya: ya, sid: sid, msgA: b64.EncodeToString(yaEnc), yaEnc: yaEnc}, nil
}

// MsgA is the box's base64url message to publish (create).
func (b *BoxInit) MsgA() string { return b.msgA }

// Verify processes the client's msg_b + confirm_b. It computes K (aborting on
// an identity/invalid peer element), derives the session keys and transcript
// hash, and verifies confirm_b in constant time. It returns a *BoxAccepted —
// the ONLY object able to seal the payload — exclusively when confirm_b
// verifies. Any error (ErrPeerEncoding, ErrPeerIdentity, ErrConfirmFailed) is a
// failed attempt (IsFailedAttempt) and the box MUST count it toward its cap; no
// payload material is produced.
func (b *BoxInit) Verify(msgB, confirmB string) (*BoxAccepted, error) {
	ybEnc, err := decodeExact(msgB, FieldSizeBytes)
	if err != nil {
		return nil, ErrPeerEncoding
	}
	ss, err := ScalarMultVfy(b.ya, ybEnc)
	if err != nil {
		return nil, err
	}
	tr := TranscriptIR(b.yaEnc, nil, ybEnc, nil)
	keys, err := DeriveSessionKeys(ISK([]byte(b.sid), ss, tr))
	if err != nil {
		return nil, err
	}
	th := TranscriptHash(b.sid, b.msgA, msgB)
	if !VerifyConfirmMAC(keys.KCB, th, confirmB) {
		return nil, ErrConfirmFailed
	}
	return &BoxAccepted{sid: b.sid, kca: keys.KCA, kPay: keys.KPay, th: th}, nil
}

// BoxAccepted is reachable only after confirm_b verified. It can emit confirm_a
// and seal the payload — nothing else can.
type BoxAccepted struct {
	sid  string
	kca  []byte
	kPay []byte
	th   []byte
}

// ConfirmA returns confirm_a = base64url(HMAC-SHA256(K_ca, th)).
func (a *BoxAccepted) ConfirmA() string { return ConfirmMAC(a.kca, a.th) }

// SealPayload wraps the pair-URI JSON under K_pay with a fresh 24-byte nonce.
// Only callable post-verification (a *BoxAccepted exists only after Verify).
func (a *BoxAccepted) SealPayload(nonce, plaintext []byte) (n, c string, err error) {
	return sealPayload(a.kPay, nonce, plaintext, a.sid)
}

// NewClientResponder builds party B's (client) reply. It decodes the box's
// msg_a, computes its own message and K, and returns the state plus
// msg_b/confirm_b to POST. An identity/invalid msg_a aborts
// (ErrPeerEncoding/Identity).
func NewClientResponder(prs, ci []byte, sid, msgA string, uniformScalar []byte) (cs *ClientAwaiting, msgB, confirmB string, err error) {
	yaEnc, err := decodeExact(msgA, FieldSizeBytes)
	if err != nil {
		return nil, "", "", ErrPeerEncoding
	}
	yb, err := ScalarFromUniform(uniformScalar)
	if err != nil {
		return nil, "", "", err
	}
	g, err := CalculateGenerator(prs, ci, []byte(sid))
	if err != nil {
		return nil, "", "", err
	}
	ybEnc := ScalarMult(yb, g)
	ss, err := ScalarMultVfy(yb, yaEnc)
	if err != nil {
		return nil, "", "", err
	}
	msgB = b64.EncodeToString(ybEnc)
	tr := TranscriptIR(yaEnc, nil, ybEnc, nil)
	keys, err := DeriveSessionKeys(ISK([]byte(sid), ss, tr))
	if err != nil {
		return nil, "", "", err
	}
	th := TranscriptHash(sid, msgA, msgB)
	confirmB = ConfirmMAC(keys.KCB, th)
	return &ClientAwaiting{sid: sid, kca: keys.KCA, kPay: keys.KPay, th: th}, msgB, confirmB, nil
}

// ClientAwaiting is party B after sending confirm_b, awaiting the box's reply.
type ClientAwaiting struct {
	sid  string
	kca  []byte
	kPay []byte
	th   []byte
}

// Finish verifies confirm_a in constant time and, ONLY if it verifies, decrypts
// the payload. A bad confirm_a returns ErrConfirmFailed and never touches the
// ciphertext; a decrypt failure returns ErrPayloadOpen. This is the structural
// guarantee that B never trusts a payload before authenticating A.
func (c *ClientAwaiting) Finish(confirmA, payloadN, payloadC string) (uri []byte, err error) {
	if !VerifyConfirmMAC(c.kca, c.th, confirmA) {
		return nil, ErrConfirmFailed
	}
	return openPayload(c.kPay, payloadN, payloadC, c.sid)
}

// ---------------------------------------------------------------------------
// Reference session helper (fixture/interop only; pure). Runs one full CPace
// exchange from both scalars and returns the shared material. It exposes the
// ISK and derived keys (draft §9.3: parties output the ISK) but NEVER the raw
// shared point K. Production code uses the role API above, not this.
// ---------------------------------------------------------------------------

// Material is the shared output of one code-link session.
type Material struct {
	MsgA string // base64url(Ya)
	MsgB string // base64url(Yb)
	ISK  []byte // 64-byte intermediate session key
	Keys SessionKeys
	TH   []byte // 32-byte transcript hash
}

// RunSession derives both parties' messages and the shared session material
// from the code inputs and two 64-byte uniform scalar seeds. A = box
// (initiator). Both ScalarMultVfy directions must agree (they do by CPace
// correctness); it returns an error if either party would abort.
func RunSession(prs, ci []byte, sid string, boxScalar, clientScalar []byte) (Material, error) {
	ya, err := ScalarFromUniform(boxScalar)
	if err != nil {
		return Material{}, fmt.Errorf("box scalar: %w", err)
	}
	yb, err := ScalarFromUniform(clientScalar)
	if err != nil {
		return Material{}, fmt.Errorf("client scalar: %w", err)
	}
	g, err := CalculateGenerator(prs, ci, []byte(sid))
	if err != nil {
		return Material{}, err
	}
	yaEnc := ScalarMult(ya, g)
	ybEnc := ScalarMult(yb, g)
	ssA, err := ScalarMultVfy(ya, ybEnc)
	if err != nil {
		return Material{}, fmt.Errorf("box vfy: %w", err)
	}
	ssB, err := ScalarMultVfy(yb, yaEnc)
	if err != nil {
		return Material{}, fmt.Errorf("client vfy: %w", err)
	}
	if subtle.ConstantTimeCompare(ssA.bytes(), ssB.bytes()) != 1 {
		return Material{}, errors.New("code-link: CPace shared secrets disagree")
	}
	tr := TranscriptIR(yaEnc, nil, ybEnc, nil)
	isk := ISK([]byte(sid), ssA, tr)
	keys, err := DeriveSessionKeys(isk)
	if err != nil {
		return Material{}, err
	}
	msgA := b64.EncodeToString(yaEnc)
	msgB := b64.EncodeToString(ybEnc)
	return Material{MsgA: msgA, MsgB: msgB, ISK: isk, Keys: keys, TH: TranscriptHash(sid, msgA, msgB)}, nil
}

// ---------------------------------------------------------------------------
// Relay-host canonicalization for CI (SPEC §3): every impl must derive CI
// identically or the cross-deployment separation breaks.
// ---------------------------------------------------------------------------

// CanonicalRelayHost extracts the lowercased host[:port] from a relay base URL.
// A default port is NOT added or removed — whatever host[:port] the URL carries
// is lowercased verbatim, so both endpoints must configure the same base URL.
func CanonicalRelayHost(baseURL string) (string, error) {
	u, err := url.Parse(strings.TrimSpace(baseURL))
	if err != nil {
		return "", err
	}
	if u.Host == "" {
		return "", fmt.Errorf("no host in relay base URL %q", baseURL)
	}
	return strings.ToLower(u.Host), nil
}

// BuildCI returns the CPace CI bytes for a canonical relay host:
// "hotline/code-link-v1|" ‖ lower(host[:port]).
func BuildCI(relayHost string) []byte { return []byte(CIPrefix + strings.ToLower(relayHost)) }

// BuildCIFromURL canonicalizes a relay base URL and returns its CI bytes.
func BuildCIFromURL(baseURL string) ([]byte, error) {
	host, err := CanonicalRelayHost(baseURL)
	if err != nil {
		return nil, err
	}
	return BuildCI(host), nil
}

// ---------------------------------------------------------------------------
// Code normalization (SPEC §2): Crockford base32.
// ---------------------------------------------------------------------------

// crockfordAlphabet is the canonical 32-symbol Crockford base32 alphabet
// (0-9 A-Z minus I L O U).
const crockfordAlphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

// NormalizeCode strips whitespace and hyphens, uppercases, applies the
// Crockford decode folds (I/L -> 1, O -> 0) and requires exactly 12 alphabet
// symbols. NON-ASCII runes are rejected BEFORE any conversion, so a look-alike
// such as 'Ł' (U+0141) can never be truncated into a valid ASCII symbol. The
// returned 12-char ASCII string is the PRS byte string (SPEC §2).
func NormalizeCode(raw string) (string, error) {
	var b strings.Builder
	for _, r := range raw {
		if r > 0x7f {
			return "", ErrBadCode // reject non-ASCII before rune->byte narrowing
		}
		switch r {
		case ' ', '\t', '\n', '\r', '-':
			continue
		}
		if r >= 'a' && r <= 'z' {
			r -= 'a' - 'A'
		}
		switch r {
		case 'I', 'L':
			r = '1'
		case 'O':
			r = '0'
		}
		// U is excluded from the Crockford alphabet and is NOT folded (SPEC §2
		// folds only I/L->1 and O->0); it falls through to rejection below.
		if !isCrockford(byte(r)) {
			return "", ErrBadCode
		}
		b.WriteByte(byte(r))
	}
	s := b.String()
	if len(s) != 12 {
		return "", ErrBadCode
	}
	return s, nil
}

func isCrockford(c byte) bool { return strings.IndexByte(crockfordAlphabet, c) >= 0 }

// Channel returns the channel (sid) group — the first 4 symbols of a normalized
// 12-char code (SPEC §2). It assumes a normalized input.
func Channel(normalized string) string { return normalized[:4] }
