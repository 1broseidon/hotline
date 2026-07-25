package app

import (
	"fmt"
	"net/url"
	"strings"
)

// This file is the strict Go parser for the hotline pair URI (A2A v2 §2).
// Production Go historically shipped only the BUILDER (store.go pairingURI); the
// mobile/desktop apps parse the URI on redeem. Lane L1 adds a parser because
// `fleet join` must read a peer's URI, and the purpose-bound refusals (F7) live
// here: a fleet URI must carry p=fleet + e=1, and the operator pairing flow must
// refuse a p=fleet URI. There is no operator-side redeem COMMAND in the Go tree
// today (redeem is app-side), so validateOperator is the reusable guard any Go
// operator redeem path calls; RefusePairURIForOperator exposes it.
//
// The parser is HARDENED against the B7 tricks sol flagged: duplicate security
// params are rejected (first-value-wins would let an attacker shadow r/s/u), and
// userinfo / path / fragment on the pair URI are rejected rather than ignored.

// PairParams is the parsed content of a hotline://pair URI.
type PairParams struct {
	Version  string
	URL      string // rendezvous base (ws://|wss://)
	Room     string
	Secret   string
	Name     string
	Purpose  string // p= ("" operator, "fleet" fleet lane)
	Envelope string // e= ("1" when envelope-mode)
}

// pairURIParams are the recognized query keys; a duplicate of any of them is a
// hard error (no first-value-wins).
var pairURIParams = []string{"v", "u", "r", "s", "n", "p", "e"}

// ParsePairURI strictly parses a hotline://pair?... URI into its params. It
// validates the scheme/host, rejects userinfo/path/fragment and duplicate
// params, and requires v/u/r/s. Purpose- and envelope-specific validation is
// left to validateFleet / validateOperator so the same parse serves both roles.
func ParsePairURI(raw string) (PairParams, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return PairParams{}, fmt.Errorf("empty pair URI")
	}
	u, err := url.Parse(raw)
	if err != nil {
		return PairParams{}, fmt.Errorf("parse pair URI: %w", err)
	}
	if u.Scheme != "hotline" || u.Host != "pair" {
		return PairParams{}, fmt.Errorf("not a hotline pair URI (want hotline://pair?...)")
	}
	if u.User != nil {
		return PairParams{}, fmt.Errorf("pair URI must not contain userinfo")
	}
	if u.Path != "" && u.Path != "/" {
		return PairParams{}, fmt.Errorf("pair URI must not contain a path")
	}
	if u.Fragment != "" || u.RawFragment != "" {
		return PairParams{}, fmt.Errorf("pair URI must not contain a fragment")
	}
	q, err := url.ParseQuery(u.RawQuery)
	if err != nil {
		return PairParams{}, fmt.Errorf("parse pair URI query: %w", err)
	}
	for _, k := range pairURIParams {
		if len(q[k]) > 1 {
			return PairParams{}, fmt.Errorf("pair URI has a duplicate %q parameter", k)
		}
	}
	p := PairParams{
		Version:  q.Get("v"),
		URL:      q.Get("u"),
		Room:     q.Get("r"),
		Secret:   q.Get("s"),
		Name:     q.Get("n"),
		Purpose:  q.Get("p"),
		Envelope: q.Get("e"),
	}
	if p.Version != "1" {
		return PairParams{}, fmt.Errorf("unsupported pair URI version %q (want 1)", p.Version)
	}
	if p.URL == "" || p.Room == "" || p.Secret == "" {
		return PairParams{}, fmt.Errorf("pair URI is missing a required field (u/r/s)")
	}
	return p, nil
}

// IsFleet reports whether the URI is purpose-bound to the fleet lane.
func (p PairParams) IsFleet() bool { return p.Purpose == "fleet" }

// validateFleet enforces the fleet-lane contract (§2): p=fleet, e=1, a valid
// 22-char room id, a 32-byte secret, and a ws/wss rendezvous with no
// userinfo/path/fragment (normalizeRendezvous enforces the last).
func (p PairParams) validateFleet() error {
	if p.Purpose != "fleet" {
		return fmt.Errorf("not a fleet pairing URI (missing p=fleet); use `hotline relay` to pair the operator app")
	}
	if p.Envelope != "1" {
		return fmt.Errorf("fleet pairing requires an envelope (e=1) URI")
	}
	if !roomIDRE.MatchString(p.Room) {
		return fmt.Errorf("fleet pair URI has an invalid room id")
	}
	if _, err := decodePairSecret(p.Secret); err != nil {
		return fmt.Errorf("fleet pair URI secret: %w", err)
	}
	if _, err := normalizeRendezvous(p.URL); err != nil {
		return err
	}
	return nil
}

// validateOperator is the operator redeem-path guard (§2, F7): the operator
// pairing flow must refuse a p=fleet URI so a fleet room can never be redeemed as
// an operator device.
func (p PairParams) validateOperator() error {
	if p.Purpose == "fleet" {
		return fmt.Errorf("this is a fleet pairing URI (p=fleet) and cannot pair the operator app; use `hotline fleet join`")
	}
	return nil
}

// RefusePairURIForOperator parses a pair URI and refuses it when it is a fleet
// URI (§2, F7). It is the reusable operator-redeem guard: any Go operator redeem
// path calls it before treating a URI as an operator pairing. (The Go tree has no
// operator redeem COMMAND today — the apps parse URIs on redeem — so this is the
// library form the guarantee lives in.)
func RefusePairURIForOperator(raw string) error {
	p, err := ParsePairURI(raw)
	if err != nil {
		return err
	}
	return p.validateOperator()
}

// parseStrictURL parses a URL and rejects userinfo/path/query/fragment — used to
// derive a clean rendezvous origin (B7). The input is already normalized
// rendezvous, so this is a belt-and-suspenders re-check.
func parseStrictURL(raw string) (*url.URL, error) {
	u, err := url.Parse(raw)
	if err != nil {
		return nil, err
	}
	if u.Host == "" {
		return nil, fmt.Errorf("URL has no host")
	}
	if u.User != nil {
		return nil, fmt.Errorf("URL must not contain userinfo")
	}
	if u.Path != "" && u.Path != "/" {
		return nil, fmt.Errorf("URL must not contain a path")
	}
	if u.Fragment != "" || u.RawQuery != "" {
		return nil, fmt.Errorf("URL must not contain a query or fragment")
	}
	return u, nil
}
