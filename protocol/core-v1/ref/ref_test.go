package corev1ref

import (
	"crypto/ecdsa"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// truncateRunes mirrors the box's internal/app/util.go truncate: a rune-safe
// clip to n runes, appending "…" only when it had to cut. The core-v1 preview
// truncation contract (§3.4) is defined by this behavior.
func truncateRunes(s string, n int) string {
	rs := []rune(s)
	if len(rs) <= n {
		return s
	}
	return string(rs[:n]) + "…"
}

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

type e1Frame struct {
	T string `json:"t"`
	N string `json:"n"`
	C string `json:"c"`
}

type envelopeFixture struct {
	Secret string `json:"secret"`
	Room   string `json:"room"`
	HKDF   struct {
		Salt string `json:"salt"`
		KB2A struct {
			Info string `json:"info"`
			Key  string `json:"key"`
		} `json:"k_b2a"`
		KA2B struct {
			Info string `json:"info"`
			Key  string `json:"key"`
		} `json:"k_a2b"`
		RoomAuth struct {
			Info     string `json:"info"`
			Key      string `json:"key"`
			AuthHash string `json:"auth_hash"`
		} `json:"room_auth"`
	} `json:"hkdf"`
	Encrypt []struct {
		Name  string  `json:"name"`
		Dir   string  `json:"dir"`
		Key   string  `json:"key"`
		AAD   string  `json:"aad"`
		Inner string  `json:"inner"`
		Frame e1Frame `json:"frame"`
	} `json:"encrypt"`
	Reject []struct {
		Name  string  `json:"name"`
		Key   string  `json:"key"`
		AAD   string  `json:"aad"`
		Frame e1Frame `json:"frame"`
	} `json:"reject"`
}

func (f *envelopeFixture) keyBytes(t *testing.T, name string) []byte {
	t.Helper()
	var b64 string
	switch name {
	case "k_b2a":
		b64 = f.HKDF.KB2A.Key
	case "k_a2b":
		b64 = f.HKDF.KA2B.Key
	default:
		t.Fatalf("unknown key ref %q", name)
	}
	key, err := base64.RawURLEncoding.DecodeString(b64)
	if err != nil {
		t.Fatal(err)
	}
	return key
}

func TestEnvelopeHKDFVectors(t *testing.T) {
	var f envelopeFixture
	loadFixture(t, "envelope-e1.json", &f)

	raw, err := DecodeSecret(f.Secret)
	if err != nil {
		t.Fatal(err)
	}
	keys, err := DeriveKeys(raw)
	if err != nil {
		t.Fatal(err)
	}
	if f.HKDF.Salt != "hotline-e1" {
		t.Fatalf("fixture salt = %q", f.HKDF.Salt)
	}
	for _, c := range []struct {
		name, info, wantInfo, want string
		got                        []byte
	}{
		{"k_b2a", f.HKDF.KB2A.Info, InfoB2A, f.HKDF.KB2A.Key, keys.KB2A},
		{"k_a2b", f.HKDF.KA2B.Info, InfoA2B, f.HKDF.KA2B.Key, keys.KA2B},
		{"room_auth", f.HKDF.RoomAuth.Info, InfoRoomAuth, f.HKDF.RoomAuth.Key, keys.RoomAuth},
	} {
		if c.info != c.wantInfo {
			t.Errorf("%s info = %q, want %q", c.name, c.info, c.wantInfo)
		}
		if got := base64.RawURLEncoding.EncodeToString(c.got); got != c.want {
			t.Errorf("%s = %s, want %s", c.name, got, c.want)
		}
	}
	if got := AuthHash(keys.RoomAuth); got != f.HKDF.RoomAuth.AuthHash {
		t.Errorf("auth_hash = %s, want %s", got, f.HKDF.RoomAuth.AuthHash)
	}
}

func TestEnvelopeEncryptVectorsByteExact(t *testing.T) {
	var f envelopeFixture
	loadFixture(t, "envelope-e1.json", &f)
	if len(f.Encrypt) != 2 {
		t.Fatalf("want 2 encrypt vectors (both directions), got %d", len(f.Encrypt))
	}
	dirs := map[string]bool{}
	for _, v := range f.Encrypt {
		dirs[v.Dir] = true
		key := f.keyBytes(t, v.Key)
		aad, err := AAD(f.Room, v.Dir)
		if err != nil {
			t.Fatal(err)
		}
		if string(aad) != v.AAD {
			t.Fatalf("%s: computed AAD %q != fixture %q", v.Name, aad, v.AAD)
		}
		nonce, err := base64.RawURLEncoding.DecodeString(v.Frame.N)
		if err != nil {
			t.Fatal(err)
		}
		// Encrypt: byte-exact ciphertext under the fixed nonce.
		n, c, err := Seal(key, nonce, aad, []byte(v.Inner))
		if err != nil {
			t.Fatal(err)
		}
		if n != v.Frame.N || c != v.Frame.C {
			t.Errorf("%s: seal mismatch\n got n=%s c=%s\nwant n=%s c=%s", v.Name, n, c, v.Frame.N, v.Frame.C)
		}
		if v.Frame.T != "e1" {
			t.Errorf("%s: frame t = %q", v.Name, v.Frame.T)
		}
		// Decrypt: recovers the exact inner-frame bytes.
		plain, err := Open(key, v.Frame.N, v.Frame.C, aad)
		if err != nil {
			t.Fatalf("%s: open: %v", v.Name, err)
		}
		if string(plain) != v.Inner {
			t.Errorf("%s: open mismatch: %s", v.Name, plain)
		}
		// The inner plaintext is a valid JSON v2 frame with a "t".
		var inner struct {
			T string `json:"t"`
		}
		if err := json.Unmarshal(plain, &inner); err != nil || inner.T == "" {
			t.Errorf("%s: inner frame not a v2 frame: %v", v.Name, err)
		}
	}
	if !dirs["b2a"] || !dirs["a2b"] {
		t.Fatalf("encrypt vectors must cover both directions, got %v", dirs)
	}
}

func TestEnvelopeRejectCasesDrop(t *testing.T) {
	var f envelopeFixture
	loadFixture(t, "envelope-e1.json", &f)
	if len(f.Reject) < 4 {
		t.Fatalf("want >= 4 reject cases, got %d", len(f.Reject))
	}
	for _, v := range f.Reject {
		key := f.keyBytes(t, v.Key)
		if _, err := Open(key, v.Frame.N, v.Frame.C, []byte(v.AAD)); err == nil {
			t.Errorf("%s: decrypt succeeded, must drop", v.Name)
		}
	}
}

type signingFixture struct {
	Key struct {
		D string `json:"d"`
		X string `json:"x"`
		Y string `json:"y"`
	} `json:"key"`
	Room    string `json:"room"`
	Vectors []struct {
		Name      string `json:"name"`
		Method    string `json:"method"`
		Path      string `json:"path"`
		Timestamp string `json:"timestamp"`
		Nonce     string `json:"nonce"`
		Body      string `json:"body"`
		Canonical string `json:"canonical"`
		Signature string `json:"signature"`
	} `json:"vectors"`
	Reject []struct {
		Name      string `json:"name"`
		Vector    string `json:"vector"`
		Signature string `json:"signature"`
		Body      string `json:"body"`
		Path      string `json:"path"`
		Expect    string `json:"expect"`
	} `json:"reject"`
	Rules struct {
		TimestampWindowSeconds int `json:"timestamp_window_seconds"`
	} `json:"rules"`
}

func (f *signingFixture) key(t *testing.T) *ecdsa.PrivateKey {
	t.Helper()
	priv, err := KeyFromScalar(f.Key.D)
	if err != nil {
		t.Fatal(err)
	}
	if ScalarB64(priv.PublicKey.X) != f.Key.X || ScalarB64(priv.PublicKey.Y) != f.Key.Y {
		t.Fatal("fixture public key x/y do not match scalar d")
	}
	return priv
}

func (f *signingFixture) vector(t *testing.T, name string) (canonical string, sig string) {
	t.Helper()
	for _, v := range f.Vectors {
		if v.Name == name {
			return v.Canonical, v.Signature
		}
	}
	t.Fatalf("no vector %q", name)
	return "", ""
}

func TestSigningVectorsVerify(t *testing.T) {
	var f signingFixture
	loadFixture(t, "signing-core.json", &f)
	priv := f.key(t)
	if len(f.Vectors) != 2 {
		t.Fatalf("want 2 vectors (register, wake), got %d", len(f.Vectors))
	}
	for _, v := range f.Vectors {
		// Canonical string reconstructs byte-exact from the parts.
		canon := CanonicalString(v.Method, v.Path, v.Timestamp, v.Nonce, []byte(v.Body))
		if canon != v.Canonical {
			t.Errorf("%s: canonical mismatch\n got %q\nwant %q", v.Name, canon, v.Canonical)
		}
		if !strings.Contains(v.Path, f.Room) {
			t.Errorf("%s: path %q does not name the fixture room", v.Name, v.Path)
		}
		// The precomputed signature verifies and is low-S.
		if err := VerifyLowS(&priv.PublicKey, []byte(canon), v.Signature); err != nil {
			t.Errorf("%s: precomputed signature rejected: %v", v.Name, err)
		}
	}
	if f.Rules.TimestampWindowSeconds != 300 {
		t.Errorf("timestamp window = %d, want 300", f.Rules.TimestampWindowSeconds)
	}
}

func TestSigningRejectCases(t *testing.T) {
	var f signingFixture
	loadFixture(t, "signing-core.json", &f)
	priv := f.key(t)
	seen := map[string]bool{}
	for _, r := range f.Reject {
		seen[r.Name] = true
		canonical, goodSig := f.vector(t, r.Vector)
		switch r.Name {
		case "high-s":
			if err := VerifyLowS(&priv.PublicKey, []byte(canonical), r.Signature); err == nil {
				t.Error("high-S twin verified, must reject")
			}
			// Sanity: it IS the twin of the valid signature.
			twin, err := HighSTwin(goodSig)
			if err != nil || twin != r.Signature {
				t.Errorf("fixture high-s is not the twin of the valid signature")
			}
		case "tampered-body":
			var v = f.Vectors[0]
			canon := CanonicalString(v.Method, v.Path, v.Timestamp, v.Nonce, []byte(r.Body))
			if canon == canonical {
				t.Error("tampered body produced identical canonical string")
			}
			if err := VerifyLowS(&priv.PublicKey, []byte(canon), goodSig); err == nil {
				t.Error("signature verified over tampered body, must reject")
			}
		case "wrong-path":
			var v = f.Vectors[0]
			canon := CanonicalString(v.Method, r.Path, v.Timestamp, v.Nonce, []byte(v.Body))
			if err := VerifyLowS(&priv.PublicKey, []byte(canon), goodSig); err == nil {
				t.Error("signature verified over wrong path, must reject")
			}
		case "stale-timestamp-past", "stale-timestamp-future", "replayed-nonce":
			// Time-window and replay semantics are verifier-side state, exercised
			// by WP1's worker tests; here we pin that the fixture declares them.
			if r.Expect == "" {
				t.Errorf("%s: missing expect code", r.Name)
			}
		default:
			t.Errorf("unknown reject case %q", r.Name)
		}
	}
	for _, want := range []string{"high-s", "tampered-body", "wrong-path", "stale-timestamp-past", "stale-timestamp-future", "replayed-nonce"} {
		if !seen[want] {
			t.Errorf("missing reject case %q", want)
		}
	}
}

// TestSignRoundtripLowS validates the signer path: fresh signatures verify and
// are always low-S (ECDSA is randomized, so this is the signer's gate).
func TestSignRoundtripLowS(t *testing.T) {
	var f signingFixture
	loadFixture(t, "signing-core.json", &f)
	priv := f.key(t)
	canonical, _ := f.vector(t, "wake")
	for i := 0; i < 64; i++ {
		sig, err := SignLowS(priv, []byte(canonical))
		if err != nil {
			t.Fatal(err)
		}
		if err := VerifyLowS(&priv.PublicKey, []byte(canonical), sig); err != nil {
			t.Fatalf("iteration %d: %v", i, err)
		}
	}
}

// TestBehavioralFixturesWellFormed pins the invariants of the hand-authored
// behavior fixtures: valid JSON, the wake Expo golden carries exactly the
// contract fields, and the register golden matches the signing fixture key.
func TestBehavioralFixturesWellFormed(t *testing.T) {
	var wake struct {
		RegisteredName string `json:"registered_name"`
		Room           string `json:"room"`
		ExpoPostGolden struct {
			Body map[string]json.RawMessage `json:"body"`
		} `json:"expo_post_golden"`
		RequestGolden        map[string]any `json:"request_golden"`
		PreviewRequestGolden map[string]any `json:"preview_request_golden"`
		PreviewExpoGolden    struct {
			Body map[string]json.RawMessage `json:"body"`
		} `json:"preview_expo_golden"`
		Truncation struct {
			Limit   int `json:"limit"`
			Vectors []struct {
				Name     string `json:"name"`
				Input    string `json:"input"`
				Expected string `json:"expected"`
			} `json:"vectors"`
		} `json:"truncation"`
	}
	loadFixture(t, "wake-hint.json", &wake)
	for _, field := range []string{"to", "title", "body", "data", "sound", "priority"} {
		if _, ok := wake.ExpoPostGolden.Body[field]; !ok {
			t.Errorf("expo_post_golden missing %q", field)
		}
	}
	var title string
	_ = json.Unmarshal(wake.ExpoPostGolden.Body["title"], &title)
	if title != wake.RegisteredName {
		t.Errorf("expo title %q != registered name %q", title, wake.RegisteredName)
	}
	var data struct {
		URL  string `json:"url"`
		Room string `json:"room"`
	}
	_ = json.Unmarshal(wake.ExpoPostGolden.Body["data"], &data)
	if data.URL != "hotline://chat" || data.Room != wake.Room {
		t.Errorf("expo data = %+v", data)
	}
	if pc, ok := wake.RequestGolden["preview_c"]; !ok || pc != nil {
		t.Errorf("request_golden preview_c must be present and null, got %v", pc)
	}

	// Clear-preview golden: the preview request carries a non-empty "preview"
	// and null "preview_c"; the preview Expo body uses that text verbatim as the
	// notification body, with the title still the registered room name.
	pv, ok := wake.PreviewRequestGolden["preview"].(string)
	if !ok || pv == "" {
		t.Errorf("preview_request_golden.preview must be a non-empty string, got %v", wake.PreviewRequestGolden["preview"])
	}
	if pc := wake.PreviewRequestGolden["preview_c"]; pc != nil {
		t.Errorf("preview_request_golden.preview_c must be null (mutually exclusive with preview), got %v", pc)
	}
	var pBody, pTitle string
	_ = json.Unmarshal(wake.PreviewExpoGolden.Body["body"], &pBody)
	_ = json.Unmarshal(wake.PreviewExpoGolden.Body["title"], &pTitle)
	if pBody != pv {
		t.Errorf("preview_expo_golden body %q != preview text %q", pBody, pv)
	}
	if pTitle != wake.RegisteredName {
		t.Errorf("preview_expo_golden title %q != registered name %q", pTitle, wake.RegisteredName)
	}

	// Truncation vectors: rune-safe clip to the 140-rune limit, matching the
	// box's truncate(). Re-derive each expected value from the input so the
	// fixture and the box share one truncation contract.
	if wake.Truncation.Limit != 140 {
		t.Errorf("truncation.limit = %d, want 140", wake.Truncation.Limit)
	}
	for _, v := range wake.Truncation.Vectors {
		got := truncateRunes(strings.TrimSpace(v.Input), wake.Truncation.Limit)
		if got != v.Expected {
			t.Errorf("truncation %q: got %q, want %q", v.Name, got, v.Expected)
		}
	}

	var sf signingFixture
	loadFixture(t, "signing-core.json", &sf)
	var reg struct {
		Cases []struct {
			Name    string `json:"name"`
			Request struct {
				BoxPub struct {
					X string `json:"x"`
					Y string `json:"y"`
				} `json:"box_pub"`
				AuthHash string `json:"auth_hash"`
			} `json:"request"`
		} `json:"cases"`
	}
	loadFixture(t, "room-register.json", &reg)
	first := reg.Cases[0]
	if first.Request.BoxPub.X != sf.Key.X || first.Request.BoxPub.Y != sf.Key.Y {
		t.Error("room-register box_pub does not match the signing fixture key")
	}
	var env envelopeFixture
	loadFixture(t, "envelope-e1.json", &env)
	if first.Request.AuthHash != env.HKDF.RoomAuth.AuthHash {
		t.Error("room-register auth_hash does not match the envelope fixture derivation")
	}

	// pair-uri-e and device-tokens: valid JSON with cases.
	for _, name := range []string{"pair-uri-e.json", "device-tokens.json"} {
		var g struct {
			Cases []json.RawMessage `json:"cases"`
		}
		loadFixture(t, name, &g)
		if len(g.Cases) == 0 {
			t.Errorf("%s has no cases", name)
		}
	}
}
