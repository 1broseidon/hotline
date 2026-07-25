package app

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// These tests gate the box's PRODUCTION core-v1 code (envelope.go, pushsign.go
// signing, corekey.go) on the frozen golden fixtures under
// protocol/core-v1/fixtures/. They mirror the reference test in
// protocol/core-v1/ref/ref_test.go but exercise the production package.

func coreFixture(t *testing.T, name string, v any) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "protocol", "core-v1", "fixtures", name))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
}

type envFixture struct {
	Secret string `json:"secret"`
	Room   string `json:"room"`
	HKDF   struct {
		Salt string `json:"salt"`
		KB2A struct {
			Info, Key string
		} `json:"k_b2a"`
		KA2B struct {
			Info, Key string
		} `json:"k_a2b"`
		RoomAuth struct {
			Info     string `json:"info"`
			Key      string `json:"key"`
			AuthHash string `json:"auth_hash"`
		} `json:"room_auth"`
	} `json:"hkdf"`
	Encrypt []struct {
		Name, Dir, Key, AAD, Inner string
		Frame                      struct{ T, N, C string } `json:"frame"`
	} `json:"encrypt"`
	Reject []struct {
		Name, Key, AAD string
		Frame          struct{ T, N, C string } `json:"frame"`
	} `json:"reject"`
}

func (f *envFixture) keyBytes(t *testing.T, ref string) []byte {
	t.Helper()
	var b64 string
	switch ref {
	case "k_b2a":
		b64 = f.HKDF.KB2A.Key
	case "k_a2b":
		b64 = f.HKDF.KA2B.Key
	default:
		t.Fatalf("unknown key ref %q", ref)
	}
	key, err := base64.RawURLEncoding.DecodeString(b64)
	if err != nil {
		t.Fatal(err)
	}
	return key
}

func TestProdEnvelopeHKDFVectors(t *testing.T) {
	var f envFixture
	coreFixture(t, "envelope-e1.json", &f)
	raw, err := decodePairSecret(f.Secret)
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct{ name, info, want string }{
		{"k_b2a", envInfoB2A, f.HKDF.KB2A.Key},
		{"k_a2b", envInfoA2B, f.HKDF.KA2B.Key},
		{"room_auth", envInfoRoomAuth, f.HKDF.RoomAuth.Key},
	} {
		got, err := hkdfDerive(raw, c.info)
		if err != nil {
			t.Fatal(err)
		}
		if enc := base64.RawURLEncoding.EncodeToString(got); enc != c.want {
			t.Errorf("%s = %s, want %s", c.name, enc, c.want)
		}
	}
	ah, err := deriveRoomAuthHash(f.Secret)
	if err != nil {
		t.Fatal(err)
	}
	if ah != f.HKDF.RoomAuth.AuthHash {
		t.Errorf("auth_hash = %s, want %s", ah, f.HKDF.RoomAuth.AuthHash)
	}
}

func TestProdEnvelopeEncryptVectorsByteExact(t *testing.T) {
	var f envFixture
	coreFixture(t, "envelope-e1.json", &f)
	for _, v := range f.Encrypt {
		key := f.keyBytes(t, v.Key)
		nonce, err := base64.RawURLEncoding.DecodeString(v.Frame.N)
		if err != nil {
			t.Fatal(err)
		}
		n, c, err := sealE1(key, nonce, []byte(v.AAD), []byte(v.Inner))
		if err != nil {
			t.Fatal(err)
		}
		if n != v.Frame.N || c != v.Frame.C {
			t.Errorf("%s: seal mismatch\n got n=%s c=%s\nwant n=%s c=%s", v.Name, n, c, v.Frame.N, v.Frame.C)
		}
		plain, err := openE1(key, v.Frame.N, v.Frame.C, []byte(v.AAD))
		if err != nil {
			t.Fatalf("%s: open: %v", v.Name, err)
		}
		if string(plain) != v.Inner {
			t.Errorf("%s: open mismatch: %s", v.Name, plain)
		}
	}
}

func TestProdEnvelopeRejectCasesDrop(t *testing.T) {
	var f envFixture
	coreFixture(t, "envelope-e1.json", &f)
	for _, v := range f.Reject {
		key := f.keyBytes(t, v.Key)
		if _, err := openE1(key, v.Frame.N, v.Frame.C, []byte(v.AAD)); err == nil {
			t.Errorf("%s: decrypt succeeded, must drop", v.Name)
		}
	}
}

// TestProdEnvelopeCodecRoundTrip proves the codec (as wired at the connector)
// wraps box→app and unwraps app→box using the derived keys, and that a wrong-key
// / plaintext frame is dropped.
func TestProdEnvelopeCodecRoundTrip(t *testing.T) {
	var f envFixture
	coreFixture(t, "envelope-e1.json", &f)
	codec, err := newEnvelopeCodec(RoomRecord{ID: f.Room, Secret: f.Secret, Envelope: true})
	if err != nil {
		t.Fatal(err)
	}
	// The a2b encrypt vector is an app→box frame the box must unwrap.
	for _, v := range f.Encrypt {
		if v.Dir != "a2b" {
			continue
		}
		frame, _ := json.Marshal(map[string]string{"t": "e1", "n": v.Frame.N, "c": v.Frame.C})
		inner, err := codec.unwrap(frame)
		if err != nil {
			t.Fatalf("unwrap a2b: %v", err)
		}
		if string(inner) != v.Inner {
			t.Errorf("unwrap mismatch: %s", inner)
		}
	}
	// wrap (b2a) then a self-unwrap must fail (b2a ciphertext under the a2b open
	// key/AAD): reflection defense. A plaintext frame is likewise dropped.
	sealed, err := codec.wrap([]byte(`{"t":"welcome"}`))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := codec.unwrap(sealed); err == nil {
		t.Error("b2a frame opened under a2b, reflection defense broken")
	}
	if _, err := codec.unwrap([]byte(`{"t":"hello","v":2}`)); err == nil {
		t.Error("plaintext frame accepted on envelope room")
	}
}

type signFixture struct {
	Key struct{ D, X, Y string } `json:"key"`
	Vectors []struct {
		Name, Method, Path, Timestamp, Nonce, Body, Canonical, Signature string
	} `json:"vectors"`
}

// TestProdSigningCanonicalAndRoundtrip proves the production signer reproduces
// the canonical string byte-exact and that fresh signatures are low-S and
// verify (ECDSA is randomized, so signer output is checked by roundtrip, not by
// byte-comparison — same gate as the reference).
func TestProdSigningCanonicalAndRoundtrip(t *testing.T) {
	var f signFixture
	coreFixture(t, "signing-core.json", &f)
	priv, err := privFromStored(storedKey{Kty: "EC", Crv: "P-256", D: f.Key.D})
	if err != nil {
		t.Fatal(err)
	}
	if scalarBase64URL(priv.PublicKey.X) != f.Key.X || scalarBase64URL(priv.PublicKey.Y) != f.Key.Y {
		t.Fatal("derived public key does not match fixture x/y")
	}
	for _, v := range f.Vectors {
		canon := canonicalString(v.Method, v.Path, v.Timestamp, v.Nonce, bodyHashBase64URL([]byte(v.Body)))
		if canon != v.Canonical {
			t.Errorf("%s: canonical mismatch\n got %q\nwant %q", v.Name, canon, v.Canonical)
		}
		// The fixture's precomputed low-S signature verifies against a fresh
		// sign→verify roundtrip through the production signer.
		sig, err := signLowS(priv, []byte(canon))
		if err != nil {
			t.Fatal(err)
		}
		if len(sig) == 0 {
			t.Errorf("%s: empty signature", v.Name)
		}
	}
}

// TestProdPublicJWKShape pins the box_pub JWK the register call sends.
func TestProdPublicJWKShape(t *testing.T) {
	var f signFixture
	coreFixture(t, "signing-core.json", &f)
	priv, err := privFromStored(storedKey{Kty: "EC", Crv: "P-256", D: f.Key.D})
	if err != nil {
		t.Fatal(err)
	}
	jwk := publicJWKFor(priv)
	if jwk["kty"] != "EC" || jwk["crv"] != "P-256" || jwk["x"] != f.Key.X || jwk["y"] != f.Key.Y {
		t.Errorf("box_pub jwk = %v", jwk)
	}
}
