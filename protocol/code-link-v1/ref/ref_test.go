package codelinkv1ref

import (
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

var tb64 = base64.RawURLEncoding.Strict()

func loadFixture(t *testing.T, name string, v any) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "fixtures", name))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
}

func mustHex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("hex %q: %v", s, err)
	}
	return b
}

func rep(b byte, n int) []byte {
	out := make([]byte, n)
	for i := range out {
		out[i] = b
	}
	return out
}

// Pinned session inputs (mirror ref/gen).
const (
	tCode      = "3YPJ24B8K7QM"
	tChannel   = "3YPJ"
	tRelayHost = "relay.hotline.sh"
)

var (
	tBoxSeed    = rep(0x11, 64)
	tClientSeed = rep(0x22, 64)
)

// ---------------------------------------------------------------------------
// BLOCKER 2 — the draft anchor is external, non-generated, and gates the impl.
// ---------------------------------------------------------------------------

// TestDraftVectorsAgainstExternalAnchor is the primary gate. It does NOT read a
// self-generated fixture: it recomputes every draft value and compares to the
// hand-transcribed draft-15 B.3 literals in draft_anchor.go. A broken impl
// fails here even after regenerating fixtures.
func TestDraftVectorsAgainstExternalAnchor(t *testing.T) {
	if err := VerifyDraftVectors(); err != nil {
		t.Fatal(err)
	}
}

// TestFixtureDraftBlockMatchesAnchor pins that the committed JSON `draft` block
// equals the anchor literals (so the fixture can't silently drift from the
// external source of truth).
func TestFixtureDraftBlockMatchesAnchor(t *testing.T) {
	var f struct {
		Draft map[string]string `json:"draft"`
	}
	// The draft block has nested objects; decode loosely.
	var raw struct {
		Draft map[string]json.RawMessage `json:"draft"`
	}
	loadFixture(t, "cpace-r255.json", &raw)
	_ = f
	get := func(k string) string {
		var s string
		if err := json.Unmarshal(raw.Draft[k], &s); err != nil {
			t.Fatalf("draft.%s: %v", k, err)
		}
		return s
	}
	d := DraftB3
	for k, want := range map[string]string{
		"g_hex":             d.GHex,
		"K_hex":             d.KHex,
		"isk_ir_hex":        d.ISKIRHex,
		"transcript_ir_hex": d.TranscriptIRHex,
		"sid_output_ir_hex": d.SidOutputIRHex,
		"generator_string_hex": d.GeneratorStringHex,
	} {
		if got := get(k); got != want {
			t.Errorf("fixture draft.%s = %s, want anchor %s", k, got, want)
		}
	}
}

// ---------------------------------------------------------------------------
// CPace negatives (SPEC §4.1 / draft B.3.11) with exact error types.
// ---------------------------------------------------------------------------

func TestScalarMultVfyNegatives(t *testing.T) {
	// A valid local scalar (the draft's B.3.10 s).
	s, err := ScalarFromCanonical(mustHex(t, DraftB3.VfyScalarHex))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name    string
		peerHex string
		want    error
	}{
		{"identity", DraftB3.InvalidIdentityHex, ErrPeerIdentity},
		{"bad-encoding-2b3c", DraftB3.InvalidY1Hex, ErrPeerEncoding},
	}
	for _, c := range cases {
		if _, err := ScalarMultVfy(s, mustHex(t, c.peerHex)); !errors.Is(err, c.want) {
			t.Errorf("%s: got %v, want %v", c.name, err, c.want)
		}
	}
	// Wrong-length peer element.
	if _, err := ScalarMultVfy(s, rep(0x01, 31)); !errors.Is(err, ErrPeerEncoding) {
		t.Errorf("short peer: got %v, want ErrPeerEncoding", err)
	}
	// Zero local scalar (sampling must reject).
	if _, err := ScalarFromUniform(rep(0x00, 64)); !errors.Is(err, ErrZeroScalar) {
		t.Errorf("zero scalar: got %v, want ErrZeroScalar", err)
	}
	// A valid scalar_mult_vfy (draft B.3.10) must succeed and match.
	ss, err := ScalarMultVfy(s, mustHex(t, DraftB3.VfyXHex))
	if err != nil {
		t.Fatalf("valid vfy aborted: %v", err)
	}
	if hex.EncodeToString(ss.bytes()) != DraftB3.VfyResultHex {
		t.Errorf("valid vfy result mismatch")
	}
}

// ---------------------------------------------------------------------------
// Role state machine — order is structurally enforced; the full happy path
// reproduces the committed wire (SPEC §4). Drives the ENFORCED API end to end.
// ---------------------------------------------------------------------------

func TestRoleHappyPathReproducesFixture(t *testing.T) {
	var ks keyScheduleFixture
	loadFixture(t, "key-schedule.json", &ks)
	var pf payloadFixture
	loadFixture(t, "payload.json", &pf)

	ci := BuildCI(tRelayHost)
	box, err := NewBoxInit([]byte(tCode), ci, tChannel, tBoxSeed)
	if err != nil {
		t.Fatal(err)
	}
	if box.MsgA() != ks.MsgAB64 {
		t.Errorf("msg_a = %s, want %s", box.MsgA(), ks.MsgAB64)
	}
	client, msgB, confirmB, err := NewClientResponder([]byte(tCode), ci, tChannel, box.MsgA(), tClientSeed)
	if err != nil {
		t.Fatal(err)
	}
	if msgB != ks.MsgBB64 {
		t.Errorf("msg_b = %s, want %s", msgB, ks.MsgBB64)
	}
	if confirmB != ks.ConfirmB {
		t.Errorf("confirm_b = %s, want %s", confirmB, ks.ConfirmB)
	}
	accepted, err := box.Verify(msgB, confirmB)
	if err != nil {
		t.Fatalf("box.Verify: %v", err)
	}
	if accepted.ConfirmA() != ks.ConfirmA {
		t.Errorf("confirm_a = %s, want %s", accepted.ConfirmA(), ks.ConfirmA)
	}
	// Client verifies confirm_a and decrypts the committed payload wire.
	uri, err := client.Finish(accepted.ConfirmA(), pf.Wire.N, pf.Wire.C)
	if err != nil {
		t.Fatalf("client.Finish: %v", err)
	}
	if string(uri) != pf.Plaintext {
		t.Errorf("recovered URI mismatch: %s", uri)
	}
	// The box re-seals byte-exact under the fixed nonce.
	n, c, err := accepted.SealPayload(mustHex(t, pf.NonceHex), []byte(pf.Plaintext))
	if err != nil {
		t.Fatal(err)
	}
	if n != pf.Wire.N || c != pf.Wire.C {
		t.Errorf("seal mismatch\n got n=%s c=%s\nwant n=%s c=%s", n, c, pf.Wire.N, pf.Wire.C)
	}
}

// TestRoleBoxRejectsBadConfirmBeforeSeal proves the box produces NO payload
// material on a failed confirm_b, and classifies it as a failed attempt.
func TestRoleBoxRejectsBadConfirmBeforeSeal(t *testing.T) {
	ci := BuildCI(tRelayHost)
	box, _ := NewBoxInit([]byte(tCode), ci, tChannel, tBoxSeed)
	_, msgB, _, _ := NewClientResponder([]byte(tCode), ci, tChannel, box.MsgA(), tClientSeed)
	// Tamper confirm_b.
	bad := flip1MAC(t, deriveConfirmB(t))
	accepted, err := box.Verify(msgB, bad)
	if accepted != nil {
		t.Fatal("box returned a BoxAccepted on bad confirm_b — payload seal is reachable!")
	}
	if !errors.Is(err, ErrConfirmFailed) || !IsFailedAttempt(err) {
		t.Errorf("bad confirm_b: got %v (IsFailedAttempt=%v)", err, IsFailedAttempt(err))
	}
}

// TestRoleClientRejectsBadConfirmABeforeDecrypt proves the client never
// decrypts the payload when confirm_a fails.
func TestRoleClientRejectsBadConfirmABeforeDecrypt(t *testing.T) {
	var pf payloadFixture
	loadFixture(t, "payload.json", &pf)
	ci := BuildCI(tRelayHost)
	box, _ := NewBoxInit([]byte(tCode), ci, tChannel, tBoxSeed)
	client, msgB, confirmB, _ := NewClientResponder([]byte(tCode), ci, tChannel, box.MsgA(), tClientSeed)
	accepted, _ := box.Verify(msgB, confirmB)
	bad := flip1MAC(t, accepted.ConfirmA())
	if _, err := client.Finish(bad, pf.Wire.N, pf.Wire.C); !errors.Is(err, ErrConfirmFailed) {
		t.Errorf("bad confirm_a: got %v, want ErrConfirmFailed", err)
	}
}

// deriveConfirmB recomputes the honest confirm_b for the pinned session.
func deriveConfirmB(t *testing.T) string {
	t.Helper()
	m, err := RunSession([]byte(tCode), BuildCI(tRelayHost), tChannel, tBoxSeed, tClientSeed)
	if err != nil {
		t.Fatal(err)
	}
	return ConfirmMAC(m.Keys.KCB, m.TH)
}

func flip1MAC(t *testing.T, macB64 string) string {
	t.Helper()
	raw, err := tb64.DecodeString(macB64)
	if err != nil {
		t.Fatal(err)
	}
	raw[0] ^= 0x01
	return tb64.EncodeToString(raw)
}

// ---------------------------------------------------------------------------
// BLOCKER 1 — the REAL wrong-PRS negative + positive control (PRS drives g).
// ---------------------------------------------------------------------------

func TestWrongPRSIsRejectedAndPRSDrivesGenerator(t *testing.T) {
	var ks keyScheduleFixture
	loadFixture(t, "key-schedule.json", &ks)
	wp := ks.WrongPRS
	ci := BuildCI(tRelayHost)
	wrongSeed := rep(0x33, 64)

	// Real box; wrong-code client over the box's msg_a.
	box, err := NewBoxInit([]byte(wp.Code), ci, tChannel, tBoxSeed)
	if err != nil {
		t.Fatal(err)
	}
	if box.MsgA() != wp.MsgAB64 {
		t.Fatalf("msg_a mismatch with fixture")
	}
	clientWrong, msgBWrong, confirmBWrong, err := NewClientResponder([]byte(wp.WrongCode), ci, tChannel, box.MsgA(), wrongSeed)
	if err != nil {
		t.Fatal(err)
	}
	_ = clientWrong
	if msgBWrong != wp.WrongMsgBB64 || confirmBWrong != wp.WrongConfirmB {
		t.Errorf("wrong-attempt wire drifted from fixture")
	}
	// The box MUST reject (failed attempt), producing no payload material.
	if acc, err := box.Verify(msgBWrong, confirmBWrong); acc != nil || !errors.Is(err, ErrConfirmFailed) {
		t.Fatalf("wrong-PRS: box.Verify acc=%v err=%v (want nil, ErrConfirmFailed)", acc, err)
	} else if !IsFailedAttempt(err) {
		t.Errorf("wrong-PRS error not a failed attempt")
	}

	// POSITIVE CONTROL: same seeds, RIGHT code -> verifies. Only PRS changed,
	// so PRS must be what drives the generator (else the wrong code would also
	// have verified above).
	boxCtl, _ := NewBoxInit([]byte(wp.Code), ci, tChannel, tBoxSeed)
	_, msgBOk, confirmBOk, err := NewClientResponder([]byte(wp.Code), ci, tChannel, boxCtl.MsgA(), wrongSeed)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := boxCtl.Verify(msgBOk, confirmBOk); err != nil {
		t.Fatalf("control: right-PRS must verify, got %v", err)
	}
}

// ---------------------------------------------------------------------------
// key-schedule.json vectors.
// ---------------------------------------------------------------------------

type keyEntry struct {
	Info   string `json:"info"`
	KeyB64 string `json:"key_b64"`
}

type keyScheduleFixture struct {
	HKDFSalt string `json:"hkdf_salt"`
	ISKHex   string `json:"isk_hex"`
	Sid      string `json:"sid"`
	MsgAB64  string `json:"msg_a_b64"`
	MsgBB64  string `json:"msg_b_b64"`
	Keys     struct {
		KCB  keyEntry `json:"k_cb"`
		KCA  keyEntry `json:"k_ca"`
		KPay keyEntry `json:"k_pay"`
	} `json:"keys"`
	THHex              string `json:"th_hex"`
	ConfirmB           string `json:"confirm_b"`
	ConfirmA           string `json:"confirm_a"`
	SwappedKeyRejected bool   `json:"swapped_key_rejected"`
	WrongPRS           struct {
		Code          string `json:"code"`
		WrongCode     string `json:"wrong_code"`
		MsgAB64       string `json:"msg_a_b64"`
		WrongMsgBB64  string `json:"wrong_msg_b_b64"`
		WrongConfirmB string `json:"wrong_confirm_b"`
	} `json:"wrong_prs_attempt"`
}

func TestKeyScheduleVectors(t *testing.T) {
	var f keyScheduleFixture
	loadFixture(t, "key-schedule.json", &f)

	if f.HKDFSalt != HKDFSalt {
		t.Errorf("salt = %q, want %q", f.HKDFSalt, HKDFSalt)
	}
	keys, err := DeriveSessionKeys(mustHex(t, f.ISKHex))
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		name, info, wantInfo, wantKey string
		got                           []byte
	}{
		{"k_cb", f.Keys.KCB.Info, InfoConfirmB, f.Keys.KCB.KeyB64, keys.KCB},
		{"k_ca", f.Keys.KCA.Info, InfoConfirmA, f.Keys.KCA.KeyB64, keys.KCA},
		{"k_pay", f.Keys.KPay.Info, InfoPayload, f.Keys.KPay.KeyB64, keys.KPay},
	} {
		if c.info != c.wantInfo {
			t.Errorf("%s info = %q, want %q", c.name, c.info, c.wantInfo)
		}
		if got := tb64.EncodeToString(c.got); got != c.wantKey {
			t.Errorf("%s = %s, want %s", c.name, got, c.wantKey)
		}
	}
	th := TranscriptHash(f.Sid, f.MsgAB64, f.MsgBB64)
	if got := hex.EncodeToString(th); got != f.THHex {
		t.Errorf("th:\n got %s\nwant %s", got, f.THHex)
	}
	if got := ConfirmMAC(keys.KCB, th); got != f.ConfirmB {
		t.Errorf("confirm_b = %s, want %s", got, f.ConfirmB)
	}
	if got := ConfirmMAC(keys.KCA, th); got != f.ConfirmA {
		t.Errorf("confirm_a = %s, want %s", got, f.ConfirmA)
	}
	if !VerifyConfirmMAC(keys.KCB, th, f.ConfirmB) || !VerifyConfirmMAC(keys.KCA, th, f.ConfirmA) {
		t.Error("confirm did not verify under its own key")
	}
	// Swapped-key: confirm_b must NOT verify under K_ca.
	if VerifyConfirmMAC(keys.KCA, th, f.ConfirmB) {
		t.Error("confirm_b verified under K_ca (key separation broken)")
	}
	if !f.SwappedKeyRejected {
		t.Error("fixture swapped_key_rejected must be true")
	}
}

// TestVerifyConfirmMACMalformedInputs pins that malformed MACs are rejected
// (and, by construction of VerifyConfirmMAC, without early return).
func TestVerifyConfirmMACMalformedInputs(t *testing.T) {
	m, err := RunSession([]byte(tCode), BuildCI(tRelayHost), tChannel, tBoxSeed, tClientSeed)
	if err != nil {
		t.Fatal(err)
	}
	good := ConfirmMAC(m.Keys.KCB, m.TH)
	if !VerifyConfirmMAC(m.Keys.KCB, m.TH, good) {
		t.Fatal("good MAC did not verify")
	}
	for _, bad := range []string{
		"",                          // empty
		"AAAA",                      // short (3 bytes)
		good + "AA",                 // long
		good + "=",                  // padded (RawURLEncoding rejects '=')
		"++++" + good[4:],           // standard-base64 chars '+' not in url alphabet
		flip1MAC(t, good),           // same length, wrong bytes
		good[:len(good)-1] + "B",    // non-canonical trailing bits (Strict rejects)
	} {
		if VerifyConfirmMAC(m.Keys.KCB, m.TH, bad) {
			t.Errorf("malformed MAC %q verified, must reject", bad)
		}
	}
}

// ---------------------------------------------------------------------------
// payload.json vectors + negatives.
// ---------------------------------------------------------------------------

type payloadFixture struct {
	Sid       string `json:"sid"`
	AAD       string `json:"aad"`
	NonceHex  string `json:"nonce_hex"`
	Plaintext string `json:"plaintext"`
	ConfirmA  string `json:"confirm_a"`
	Wire      struct {
		N string `json:"n"`
		C string `json:"c"`
	} `json:"wire"`
	Negatives []struct {
		Name string `json:"name"`
		Wire struct {
			N string `json:"n"`
			C string `json:"c"`
		} `json:"wire"`
		OpenSid       string `json:"open_sid"`
		WrongConfirmA string `json:"wrong_confirm_a"`
	} `json:"negatives"`
}

func TestPayloadVectorsAndNegatives(t *testing.T) {
	var f payloadFixture
	loadFixture(t, "payload.json", &f)
	if f.AAD != PayloadAADPre+f.Sid {
		t.Errorf("aad = %q, want %q", f.AAD, PayloadAADPre+f.Sid)
	}

	// Recover the session's K_pay via a full run so we can drive open/seal.
	m, err := RunSession([]byte(tCode), BuildCI(tRelayHost), tChannel, tBoxSeed, tClientSeed)
	if err != nil {
		t.Fatal(err)
	}
	// Seal reproduces the wire byte-exact.
	n, c, err := sealPayload(m.Keys.KPay, mustHex(t, f.NonceHex), []byte(f.Plaintext), f.Sid)
	if err != nil {
		t.Fatal(err)
	}
	if n != f.Wire.N || c != f.Wire.C {
		t.Errorf("seal mismatch\n got n=%s c=%s\nwant n=%s c=%s", n, c, f.Wire.N, f.Wire.C)
	}
	// Open recovers the exact plaintext; it is valid {"uri":...} JSON.
	pt, err := openPayload(m.Keys.KPay, f.Wire.N, f.Wire.C, f.Sid)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	if string(pt) != f.Plaintext {
		t.Errorf("open plaintext mismatch")
	}
	var body struct {
		URI string `json:"uri"`
	}
	if err := json.Unmarshal(pt, &body); err != nil || body.URI == "" {
		t.Errorf("payload is not a {\"uri\":...} object: %v", err)
	}

	// Wrong-key open must fail.
	if _, err := openPayload(rep(0x00, 32), f.Wire.N, f.Wire.C, f.Sid); !errors.Is(err, ErrPayloadOpen) {
		t.Errorf("wrong-key open: got %v, want ErrPayloadOpen", err)
	}

	// Fixture-driven negatives.
	for _, ng := range f.Negatives {
		switch ng.Name {
		case "confirm-a-wrong":
			// A wrong confirm_a must be rejected by the role client before decrypt.
			client, _, _, _ := NewClientResponder([]byte(tCode), BuildCI(tRelayHost), tChannel, m.MsgA, tClientSeed)
			if _, err := client.Finish(ng.WrongConfirmA, f.Wire.N, f.Wire.C); !errors.Is(err, ErrConfirmFailed) {
				t.Errorf("%s: got %v, want ErrConfirmFailed", ng.Name, err)
			}
		default:
			sid := f.Sid
			if ng.OpenSid != "" {
				sid = ng.OpenSid
			}
			if _, err := openPayload(m.Keys.KPay, ng.Wire.N, ng.Wire.C, sid); !errors.Is(err, ErrPayloadOpen) {
				t.Errorf("%s: open err = %v, want ErrPayloadOpen", ng.Name, err)
			}
		}
	}

	// Standard-base64 / non-canonical ciphertext must be rejected by strict b64.
	if _, err := openPayload(m.Keys.KPay, f.Wire.N, f.Wire.C+"==", f.Sid); err == nil {
		t.Error("padded ciphertext accepted, must reject")
	}
}

// ---------------------------------------------------------------------------
// code-normalize.json vectors (incl. Unicode negatives).
// ---------------------------------------------------------------------------

type codeNormalizeFixture struct {
	Alphabet string `json:"alphabet"`
	Valid    []struct {
		Input      string `json:"input"`
		Normalized string `json:"normalized"`
		Channel    string `json:"channel"`
		PRS        string `json:"prs"`
	} `json:"valid"`
	Invalid []struct {
		Input string `json:"input"`
	} `json:"invalid"`
}

func TestCodeNormalizeVectors(t *testing.T) {
	var f codeNormalizeFixture
	loadFixture(t, "code-normalize.json", &f)
	if len(f.Valid) == 0 || len(f.Invalid) == 0 {
		t.Fatal("need both valid and invalid cases")
	}
	for _, v := range f.Valid {
		got, err := NormalizeCode(v.Input)
		if err != nil {
			t.Errorf("normalize %q: unexpected error %v", v.Input, err)
			continue
		}
		if got != v.Normalized || Channel(got) != v.Channel || got != v.PRS {
			t.Errorf("normalize %q = %q (chan %q), want %q/%q", v.Input, got, Channel(got), v.Normalized, v.Channel)
		}
	}
	for _, v := range f.Invalid {
		if _, err := NormalizeCode(v.Input); err == nil {
			t.Errorf("normalize %q succeeded, must error", v.Input)
		}
	}
	// Explicit Unicode look-alike: must NOT become 'A'.
	if _, err := NormalizeCode("ŁYPJ24B8K7QM"); err == nil {
		t.Error("Ł-prefixed code accepted, must reject non-ASCII")
	}
}

// ---------------------------------------------------------------------------
// ci-host.json — canonicalization goldens.
// ---------------------------------------------------------------------------

type ciHostFixture struct {
	CIPrefix string `json:"ci_prefix"`
	Cases    []struct {
		BaseURL       string `json:"base_url"`
		CanonicalHost string `json:"canonical_host"`
		CI            string `json:"ci"`
	} `json:"cases"`
}

func TestCIHostVectors(t *testing.T) {
	var f ciHostFixture
	loadFixture(t, "ci-host.json", &f)
	if f.CIPrefix != CIPrefix {
		t.Errorf("ci_prefix = %q, want %q", f.CIPrefix, CIPrefix)
	}
	for _, c := range f.Cases {
		host, err := CanonicalRelayHost(c.BaseURL)
		if err != nil {
			t.Errorf("%s: %v", c.BaseURL, err)
			continue
		}
		if host != c.CanonicalHost {
			t.Errorf("%s: host = %q, want %q", c.BaseURL, host, c.CanonicalHost)
		}
		ci, err := BuildCIFromURL(c.BaseURL)
		if err != nil {
			t.Fatal(err)
		}
		if string(ci) != c.CI {
			t.Errorf("%s: ci = %q, want %q", c.BaseURL, ci, c.CI)
		}
		if !bytes.Equal(ci, BuildCI(host)) {
			t.Errorf("%s: BuildCIFromURL != BuildCI(host)", c.BaseURL)
		}
	}
}
