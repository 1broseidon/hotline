package app

import "testing"

func TestFleetPairURIBuildParseRoundTrip(t *testing.T) {
	r, secret, err := mintRoom(fleetTestRelay, "", true)
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	uri := FleetPairingURI(r.URL, r.ID, secret, "peerbox")

	p, err := ParsePairURI(uri)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if p.Version != "1" || p.URL != r.URL || p.Room != r.ID || p.Secret != secret || p.Name != "peerbox" {
		t.Fatalf("round trip mismatch: %+v", p)
	}
	if !p.IsFleet() || p.Purpose != "fleet" || p.Envelope != "1" {
		t.Fatalf("fleet purpose/envelope not carried: %+v", p)
	}
	if err := p.validateFleet(); err != nil {
		t.Fatalf("validateFleet on a fleet URI: %v", err)
	}
}

func TestOperatorRedeemRefusesFleetURI(t *testing.T) {
	r, secret, _ := mintRoom(fleetTestRelay, "", true)
	fleetURI := FleetPairingURI(r.URL, r.ID, secret, "peer")
	if err := RefusePairURIForOperator(fleetURI); err == nil {
		t.Fatalf("operator redeem accepted a p=fleet URI")
	}
	// A normal operator URI passes the same guard.
	operatorURI := PairingURIMode(r.URL, r.ID, secret, "op", true)
	if err := RefusePairURIForOperator(operatorURI); err != nil {
		t.Fatalf("operator redeem refused a normal URI: %v", err)
	}
}

func TestFleetJoinValidationRejects(t *testing.T) {
	r, secret, _ := mintRoom(fleetTestRelay, "", true)
	// Missing p=fleet.
	if _, err := ParsePairURI(PairingURIMode(r.URL, r.ID, secret, "n", true)); err != nil {
		t.Fatalf("parse operator URI: %v", err)
	} else if p, _ := ParsePairURI(PairingURIMode(r.URL, r.ID, secret, "n", true)); p.validateFleet() == nil {
		t.Fatalf("validateFleet accepted a URI without p=fleet")
	}
	// p=fleet but no e=1 (envelope required).
	noEnv := pairingURI(r.URL, r.ID, secret, "n", false, "fleet")
	p, err := ParsePairURI(noEnv)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if p.validateFleet() == nil {
		t.Fatalf("validateFleet accepted a fleet URI without e=1")
	}
}

func TestParsePairURIRejectsGarbage(t *testing.T) {
	for _, raw := range []string{"", "https://example.com/pair", "hotline://other?v=1", "hotline://pair?v=2&u=wss://x&r=abc&s=def"} {
		if _, err := ParsePairURI(raw); err == nil {
			t.Fatalf("ParsePairURI accepted garbage: %q", raw)
		}
	}
}

func TestParsePairURIHardening(t *testing.T) {
	// Duplicate security params (first-value-wins shadowing) are rejected (B7).
	if _, err := ParsePairURI("hotline://pair?v=1&u=wss://a&r=aaaaaaaaaaaaaaaaaaaaaa&r=bbbbbbbbbbbbbbbbbbbbbb&s=x"); err == nil {
		t.Fatalf("accepted a duplicate r param")
	}
	// Userinfo, path, and fragment on the pair URI are rejected (B7).
	for _, raw := range []string{
		"hotline://user@pair?v=1&u=wss://a&r=aaaaaaaaaaaaaaaaaaaaaa&s=x",
		"hotline://pair/evil?v=1&u=wss://a&r=aaaaaaaaaaaaaaaaaaaaaa&s=x",
		"hotline://pair?v=1&u=wss://a&r=aaaaaaaaaaaaaaaaaaaaaa&s=x#frag",
	} {
		if _, err := ParsePairURI(raw); err == nil {
			t.Fatalf("accepted userinfo/path/fragment: %q", raw)
		}
	}
}
