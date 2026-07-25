// Command gen regenerates the code-link-v1 golden fixtures under
// protocol/code-link-v1/fixtures/ from the reference implementation.
//
//	go run ./protocol/code-link-v1/ref/gen
//
// Before writing anything it calls ref.VerifyDraftVectors, which recomputes the
// CPace-Ristretto255 vectors and asserts them against the hand-transcribed
// draft-15 literals in draft_anchor.go — so a broken implementation cannot
// launder itself by regenerating its own goldens. Every hotline value is
// deterministic (pinned scalar seeds / nonces).
package main

import (
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	ref "github.com/1broseidon/hotline/protocol/code-link-v1/ref"
)

var b64 = base64.RawURLEncoding.Strict()

func rep(b byte, n int) []byte {
	out := make([]byte, n)
	for i := range out {
		out[i] = b
	}
	return out
}

func hx(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

func writeJSON(path string, v any) {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		panic(err)
	}
	data = append(data, '\n')
	if err := os.WriteFile(path, data, 0o644); err != nil {
		panic(err)
	}
	fmt.Println("wrote", path)
}

// Pinned session inputs.
const (
	code      = "3YPJ24B8K7QM" // normalized; channel = "3YPJ"
	channel   = "3YPJ"
	relayBase = "https://relay.hotline.sh"
	relayHost = "relay.hotline.sh"
)

var (
	boxSeed    = rep(0x11, 64)
	clientSeed = rep(0x22, 64)
	payNonce   = hx("0102030405060708090a0b0c0d0e0f101112131415161718")
	pairURI    = `{"uri":"hotline://pair?v=1&u=https%3A%2F%2Frelay.hotline.sh&r=aBcDeFgHiJkLmNoPqRsTuv&s=3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&n=pi&e=1"}`
)

func session() ref.Material {
	ci := ref.BuildCI(relayHost)
	return must(ref.RunSession([]byte(code), ci, channel, boxSeed, clientSeed))
}

func main() {
	if err := ref.VerifyDraftVectors(); err != nil {
		panic("DRAFT ANCHOR MISMATCH — implementation diverged from draft-15 B.3:\n" + err.Error())
	}
	dir := "protocol/code-link-v1/fixtures"
	if len(os.Args) > 1 {
		dir = os.Args[1]
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		panic(err)
	}
	writeJSON(filepath.Join(dir, "cpace-r255.json"), buildCPace())
	writeJSON(filepath.Join(dir, "key-schedule.json"), buildKeySchedule())
	writeJSON(filepath.Join(dir, "payload.json"), buildPayload())
	writeJSON(filepath.Join(dir, "code-normalize.json"), buildCodeNormalize())
	writeJSON(filepath.Join(dir, "ci-host.json"), buildCIHost())
}

// ---------------------------------------------------------------------------
// cpace-r255.json — mirrors the draft anchor literals + hotline session + negs.
// ---------------------------------------------------------------------------

func buildCPace() any {
	d := ref.DraftB3
	m := session()

	// The draft block is emitted from the ANCHOR literals (not recomputed), so
	// the JSON echoes the external source of truth. VerifyDraftVectors (run in
	// main and in the test) is what proves the impl matches these bytes.
	draft := map[string]any{
		"source":               "draft-irtf-cfrg-cpace-15 Appendix B.3 (hand-transcribed anchor, see ref/draft_anchor.go)",
		"suite":                "CPACE-RISTRETTO255-SHA512",
		"dsi":                  ref.DSI,
		"dsi_isk":              ref.DSIISK,
		"H":                    "SHA-512",
		"s_in_bytes":           ref.SHA512BlockBytes,
		"prs_hex":              d.PRSHex,
		"ci_hex":               d.CIHex,
		"sid_hex":              d.SidHex,
		"ya_scalar_hex":        d.YaScalarHex,
		"yb_scalar_hex":        d.YbScalarHex,
		"ada_hex":              d.AdaHex,
		"adb_hex":              d.AdbHex,
		"generator_string_hex": d.GeneratorStringHex,
		"g_hex":                d.GHex,
		"Ya_hex":               d.YaHex,
		"Yb_hex":               d.YbHex,
		"K_hex":                d.KHex,
		"transcript_ir_hex":    d.TranscriptIRHex,
		"transcript_oc_hex":    d.TranscriptOCHex,
		"isk_ir_hex":           d.ISKIRHex,
		"isk_oc_hex":           d.ISKOCHex,
		"sid_output_label":     d.SidOutputLabel,
		"sid_output_ir_hex":    d.SidOutputIRHex,
		"sid_output_oc_hex":    d.SidOutputOCHex,
		"scalar_mult_vfy_valid": map[string]string{
			"s": d.VfyScalarHex, "X": d.VfyXHex, "result": d.VfyResultHex,
		},
		"scalar_mult_vfy_invalid": map[string]string{
			"Y_i1_bad_encoding": d.InvalidY1Hex, "Y_i2_identity": d.InvalidIdentityHex,
		},
	}

	hot := map[string]any{
		"code": code, "channel": channel, "prs_ascii": code,
		"relay_base_url": relayBase, "relay_host": relayHost,
		"ci":                 string(ref.BuildCI(relayHost)),
		"box_scalar_seed":    "11 x64 (SetUniformBytes input)",
		"client_scalar_seed": "22 x64 (SetUniformBytes input)",
		"g_b64":              b64.EncodeToString(ref.EncodeElement(must(ref.CalculateGenerator([]byte(code), ref.BuildCI(relayHost), []byte(channel))))),
		"msg_a_b64":          m.MsgA,
		"msg_b_b64":          m.MsgB,
		"isk_hex":            hex.EncodeToString(m.ISK),
	}

	// Negatives — peer-side aborts. Draft B.3.11 anchors reused, plus a
	// zero-scalar → identity case.
	identity := b64.EncodeToString(rep(0x00, 32))
	badEnc := b64.EncodeToString(hx(ref.DraftB3.InvalidY1Hex))
	negatives := []map[string]any{
		{"name": "peer-identity", "peer_b64": identity, "expect_error": "ErrPeerIdentity",
			"reason": "peer element is the group identity (draft B.3.11 Y_i2)"},
		{"name": "peer-bad-encoding", "peer_b64": badEnc, "expect_error": "ErrPeerEncoding",
			"reason": "not a canonical ristretto255 encoding (draft B.3.11 Y_i1, 2b3c…a51c)"},
		{"name": "peer-short", "peer_b64": b64.EncodeToString(rep(0x01, 31)), "expect_error": "ErrPeerEncoding",
			"reason": "31-byte peer element (wrong length)"},
		{"name": "zero-local-scalar", "uniform_seed": "00 x64", "expect_error": "ErrZeroScalar",
			"reason": "a zero scalar would make the local element the identity; sampling must reject/resample"},
	}

	return map[string]any{
		"note":            "CPace over ristretto255, CPACE-RISTRETTO255-SHA512. `draft` mirrors the hand-transcribed anchor (ref/draft_anchor.go); the impl is asserted against it by VerifyDraftVectors, not by self-regeneration. `hotline_session` is a full role run under hotline inputs (empty AD, initiator ordering, A=box).",
		"draft":           draft,
		"hotline_session": hot,
		"negatives":       negatives,
	}
}

// ---------------------------------------------------------------------------
// key-schedule.json — session keys, th, confirms + a REAL wrong-PRS negative.
// ---------------------------------------------------------------------------

func buildKeySchedule() any {
	m := session()

	confirmB := ref.ConfirmMAC(m.Keys.KCB, m.TH)
	confirmA := ref.ConfirmMAC(m.Keys.KCA, m.TH)

	// --- BLOCKER-1 fix: a REAL single-attempt wrong-PRS negative. ---
	// The client types the WRONG code (last symbol differs). The box keeps its
	// real Ya/ya; the client mixes a DIFFERENT generator (driven by PRS), so
	// the two sides derive DIFFERENT K -> different K_cb. The client's confirm_b
	// (honestly computed under its own wrong K) must FAIL under the box's real
	// K_cb over the shared th. If CalculateGenerator ignored PRS, both sides
	// would share K and the confirm WOULD verify — so this proves PRS drives
	// the generator. The positive control uses the SAME seeds with the RIGHT
	// code and succeeds, isolating PRS as the only difference.
	ci := ref.BuildCI(relayHost)
	wrongCode := "3YPJ24B8K7QN" // sid/channel identical; secret tail differs
	wrongClientSeed := rep(0x33, 64)

	box := must(ref.NewBoxInit([]byte(code), ci, channel, boxSeed))
	msgA := box.MsgA()
	_, msgBWrong, confirmBWrong := mustResponder(ref.NewClientResponder([]byte(wrongCode), ci, channel, msgA, wrongClientSeed))
	if _, err := box.Verify(msgBWrong, confirmBWrong); err == nil {
		panic("BLOCKER-1: wrong-PRS confirm_b verified — PRS is not driving the generator!")
	} else if !ref.IsFailedAttempt(err) {
		panic("BLOCKER-1: wrong-PRS error is not classified as a failed attempt: " + err.Error())
	}
	// Positive control.
	boxCtl := must(ref.NewBoxInit([]byte(code), ci, channel, boxSeed))
	_, msgBOk, confirmBOk := mustResponder(ref.NewClientResponder([]byte(code), ci, channel, boxCtl.MsgA(), wrongClientSeed))
	if _, err := boxCtl.Verify(msgBOk, confirmBOk); err != nil {
		panic("BLOCKER-1 control: right-PRS confirm_b failed: " + err.Error())
	}

	swapped := !ref.VerifyConfirmMAC(m.Keys.KCA, m.TH, confirmB)

	return map[string]any{
		"note":      "HKDF-SHA256 key schedule + SHA-256 transcript hash + HMAC-SHA256 confirmations (SPEC §4.3). Session = cpace-r255.json hotline_session.",
		"hkdf_salt": ref.HKDFSalt,
		"isk_hex":   hex.EncodeToString(m.ISK),
		"sid":       channel,
		"msg_a_b64": m.MsgA,
		"msg_b_b64": m.MsgB,
		"keys": map[string]any{
			"k_cb":  map[string]string{"info": ref.InfoConfirmB, "key_b64": b64.EncodeToString(m.Keys.KCB)},
			"k_ca":  map[string]string{"info": ref.InfoConfirmA, "key_b64": b64.EncodeToString(m.Keys.KCA)},
			"k_pay": map[string]string{"info": ref.InfoPayload, "key_b64": b64.EncodeToString(m.Keys.KPay)},
		},
		"th_framing":           ref.THPrefix + channel + "|" + m.MsgA + "|" + m.MsgB,
		"th_hex":               hex.EncodeToString(m.TH),
		"confirm_b":            confirmB,
		"confirm_a":            confirmA,
		"swapped_key_rejected": swapped,
		"wrong_prs_attempt": map[string]any{
			"note":              "REAL single attempt. Reproduce: box=NewBoxInit(code), client=NewClientResponder(wrong_code) over the box's msg_a; box.Verify MUST return a failed attempt. Control uses the same seeds with the right code and succeeds.",
			"code":              code,
			"wrong_code":        wrongCode,
			"box_scalar_seed":   "11 x64",
			"wrong_client_seed": "33 x64",
			"msg_a_b64":         msgA,
			"wrong_msg_b_b64":   msgBWrong,
			"wrong_confirm_b":   confirmBWrong,
			"expect":            "box.Verify -> ErrConfirmFailed (IsFailedAttempt == true)",
			"control_expect":    "same seeds + right code -> box.Verify succeeds (proves PRS drives the generator)",
		},
	}
}

func mustResponder(cs *ref.ClientAwaiting, msgB, confirmB string, err error) (*ref.ClientAwaiting, string, string) {
	if err != nil {
		panic(err)
	}
	return cs, msgB, confirmB
}

// ---------------------------------------------------------------------------
// payload.json — happy wire (via the enforced role API) + negatives.
// ---------------------------------------------------------------------------

func buildPayload() any {
	ci := ref.BuildCI(relayHost)
	box := must(ref.NewBoxInit([]byte(code), ci, channel, boxSeed))
	client, msgB, confirmB := mustResponder(ref.NewClientResponder([]byte(code), ci, channel, box.MsgA(), clientSeed))
	accepted := must(box.Verify(msgB, confirmB))
	confirmA := accepted.ConfirmA()
	n, c := must2(accepted.SealPayload(payNonce, []byte(pairURI)))
	got := must(client.Finish(confirmA, n, c))
	if string(got) != pairURI {
		panic("payload: honest Finish did not recover the URI")
	}

	ctBytes := must(b64.DecodeString(c))
	tampered := append([]byte(nil), ctBytes...)
	tampered[0] ^= 0x01

	return map[string]any{
		"note":      "XChaCha20-Poly1305 payload wrap under K_pay (SPEC §4.3), produced via the enforced role API (SealPayload only after confirm_b verifies; Finish verifies confirm_a before decrypt). Session = cpace-r255.json hotline_session.",
		"sid":       channel,
		"aad":       ref.PayloadAADPre + channel,
		"nonce_hex": hex.EncodeToString(payNonce),
		"plaintext": pairURI,
		"confirm_a": confirmA,
		"wire":      map[string]string{"n": n, "c": c},
		"negatives": []map[string]any{
			{"name": "tampered-ciphertext", "wire": map[string]string{"n": n, "c": b64.EncodeToString(tampered)},
				"expect": "Finish -> ErrPayloadOpen", "reason": "one ciphertext byte flipped -> Poly1305 tag mismatch"},
			{"name": "wrong-sid-aad", "wire": map[string]string{"n": n, "c": c}, "open_sid": "WRNG",
				"expect": "open -> ErrPayloadOpen", "reason": "different sid changes the AAD -> tag mismatch"},
			{"name": "bad-nonce-length", "wire": map[string]string{"n": b64.EncodeToString(rep(0x00, 23)), "c": c},
				"expect": "open -> ErrPayloadOpen", "reason": "23-byte nonce (must be 24)"},
			{"name": "confirm-a-wrong", "wrong_confirm_a": flip1(confirmA),
				"expect": "Finish -> ErrConfirmFailed", "reason": "client must reject before decrypting"},
		},
	}
}

func must2[A, B any](a A, b B, err error) (A, B) {
	if err != nil {
		panic(err)
	}
	return a, b
}

// flip1 returns a MAC string that decodes to the same length but differs in one
// byte (still valid strict base64url), to exercise a genuine MAC mismatch.
func flip1(macB64 string) string {
	raw := must(b64.DecodeString(macB64))
	raw[0] ^= 0x01
	return b64.EncodeToString(raw)
}

// ---------------------------------------------------------------------------
// code-normalize.json — Crockford + Unicode negatives.
// ---------------------------------------------------------------------------

func buildCodeNormalize() any {
	valid := []map[string]any{}
	for _, v := range []struct{ in, out string }{
		{"3YPJ-24B8-K7QM", "3YPJ24B8K7QM"},
		{"3ypj-24b8-k7qm", "3YPJ24B8K7QM"},
		{"3YPJ 24B8 K7QM", "3YPJ24B8K7QM"},
		{"3YPJ24B8K7QM", "3YPJ24B8K7QM"},
		{"  3ypj24b8k7qm  ", "3YPJ24B8K7QM"},
	} {
		got := must(ref.NormalizeCode(v.in))
		if got != v.out {
			panic("normalize " + v.in + " => " + got)
		}
		valid = append(valid, map[string]any{"input": v.in, "normalized": v.out, "channel": v.out[:4], "prs": v.out})
	}
	foldIn := "iIlL-oO01-2345"
	foldOut := must(ref.NormalizeCode(foldIn))
	valid = append(valid, map[string]any{"input": foldIn, "normalized": foldOut, "channel": foldOut[:4], "prs": foldOut,
		"note": "Crockford folds: i/I/l/L -> 1, o/O -> 0"})

	invalid := []map[string]any{
		{"input": "3YPJ-24B8-K7Q", "reason": "only 11 symbols"},
		{"input": "3YPJ-24B8-K7QMM", "reason": "13 symbols"},
		{"input": "3YPU-24B8-K7QM", "reason": "U is excluded from the alphabet and is not folded"},
		{"input": "3YP!-24B8-K7QM", "reason": "non-alphabet ASCII symbol"},
		{"input": "ŁYPJ-24B8-K7QM", "reason": "non-ASCII look-alike (Ł U+0141) must be rejected pre-narrowing, not folded to 'A'"},
		{"input": "3YPJ-24B8-K7QⓂ", "reason": "non-ASCII circled M (U+24C2)"},
	}
	for _, iv := range invalid {
		if _, err := ref.NormalizeCode(iv["input"].(string)); err == nil {
			panic("normalize accepted invalid: " + iv["input"].(string))
		}
	}

	return map[string]any{
		"note":     "Crockford base32 normalization (SPEC §2): strip ws/hyphens, uppercase, fold i/l->1 o->0, require exactly 12 symbols; reject non-ASCII BEFORE conversion. PRS = normalized 12 chars; channel/sid = first 4.",
		"alphabet": "0123456789ABCDEFGHJKMNPQRSTVWXYZ",
		"valid":    valid,
		"invalid":  invalid,
	}
}

// ---------------------------------------------------------------------------
// ci-host.json — relay-host canonicalization + CI byte string (SPEC §3).
// ---------------------------------------------------------------------------

func buildCIHost() any {
	cases := []map[string]any{}
	for _, u := range []string{
		"https://relay.hotline.sh",
		"https://Relay.Hotline.SH/v1/link-codes",
		"http://localhost:8787",
		"https://relay.example.com:8443/base",
	} {
		host := must(ref.CanonicalRelayHost(u))
		ci := must(ref.BuildCIFromURL(u))
		cases = append(cases, map[string]any{
			"base_url": u, "canonical_host": host, "ci": string(ci),
		})
	}
	return map[string]any{
		"note":      "CanonicalRelayHost extracts lowercased host[:port]; CI = \"hotline/code-link-v1|\" + host. Every downstream impl MUST derive CI identically (cross-deployment separation depends on it).",
		"ci_prefix": ref.CIPrefix,
		"cases":     cases,
	}
}
