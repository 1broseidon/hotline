package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

func fleetURIForRelay(relay string) string {
	// A valid fleet pair URI: 22-char room id + a 43-char (32-byte) base64url
	// secret + p=fleet + e=1.
	room := strings.Repeat("A", 22)
	secret := strings.Repeat("A", 43)
	return app.FleetPairingURI(relay, room, secret, "peerbox")
}

func fleetTestURI() string { return fleetURIForRelay("wss://relay.test") }

func TestCmdFleetLinkAndLs(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://relay.test")

	var out bytes.Buffer
	if err := cmdFleet("", []string{"link", "--alias", "peer"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet link: %v", err)
	}
	if !strings.Contains(out.String(), "hotline://pair?") || !strings.Contains(out.String(), "p=fleet") {
		t.Fatalf("link did not print a fleet pair URI:\n%s", out.String())
	}

	out.Reset()
	if err := cmdFleet("", []string{"ls"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet ls: %v", err)
	}
	if !strings.Contains(out.String(), "peer") || !strings.Contains(out.String(), "serve") {
		t.Fatalf("ls missing the serve edge:\n%s", out.String())
	}

	// ls --json must NOT leak the serve secret (B5).
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatalf("load app cfg: %v", err)
	}
	fs, err := app.OpenFleetStore(cfg.StateDir)
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	edges, err := fs.Edges()
	if err != nil || len(edges) != 1 || edges[0].Secret == "" {
		t.Fatalf("expected 1 serve edge with a secret: %v %+v", err, edges)
	}
	secret := edges[0].Secret
	out.Reset()
	if err := cmdFleet("", []string{"ls", "--json"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet ls --json: %v", err)
	}
	if strings.Contains(out.String(), secret) || strings.Contains(out.String(), `"secret"`) {
		t.Fatalf("ls --json leaked a secret:\n%s", out.String())
	}
}

func TestCmdFleetJoinSSRFGate(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://relay.test")
	// A URI pointing at a different origin is refused without --allow-relay.
	otherURI := fleetURIForRelay("wss://other.test")
	if err := cmdFleet("", []string{"join", otherURI, "--alias", "peer"}, &bytes.Buffer{}, &bytes.Buffer{}); err == nil {
		t.Fatalf("join stored an untrusted relay origin without --allow-relay")
	}
	// With the explicit override it is accepted.
	if err := cmdFleet("", []string{"join", otherURI, "--alias", "peer", "--allow-relay", "wss://other.test"}, &bytes.Buffer{}, &bytes.Buffer{}); err != nil {
		t.Fatalf("join with --allow-relay failed: %v", err)
	}
}

func TestCmdFleetJoinAndRemoveDialWarning(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://relay.test")

	var out bytes.Buffer
	if err := cmdFleet("", []string{"join", fleetTestURI(), "--alias", "peer"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet join: %v", err)
	}
	if !strings.Contains(out.String(), "flt-") {
		t.Fatalf("join did not register a device id:\n%s", out.String())
	}

	out.Reset()
	if err := cmdFleet("", []string{"rm", "peer"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet rm: %v", err)
	}
	if !strings.Contains(out.String(), "LOCAL") {
		t.Fatalf("dial-side rm missing the local-only warning:\n%s", out.String())
	}
}

func TestCmdFleetJoinRefusesOperatorURI(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	room := strings.Repeat("A", 22)
	secret := strings.Repeat("A", 43)
	operatorURI := app.PairingURIMode("wss://relay.test", room, secret, "op", true)
	if err := cmdFleet("", []string{"join", operatorURI, "--alias", "peer"}, &bytes.Buffer{}, &bytes.Buffer{}); err == nil {
		t.Fatalf("fleet join accepted a non-fleet operator URI")
	}
}

func TestReadFleetURIStdin(t *testing.T) {
	got, err := readFleetURI("-", strings.NewReader("  hotline://pair?v=1&u=wss://x&r=y&s=z&p=fleet&e=1  \n"))
	if err != nil {
		t.Fatalf("readFleetURI stdin: %v", err)
	}
	if got != "hotline://pair?v=1&u=wss://x&r=y&s=z&p=fleet&e=1" {
		t.Fatalf("stdin URI not trimmed/returned: %q", got)
	}
}

func TestAliasFlagRequired(t *testing.T) {
	if _, err := aliasFlag(nil); err == nil {
		t.Fatalf("aliasFlag accepted a missing --alias")
	}
	got, err := aliasFlag([]string{"--alias", "beta"})
	if err != nil || got != "beta" {
		t.Fatalf("aliasFlag(--alias beta) = %q, %v", got, err)
	}
	if got, _ := aliasFlag([]string{"--alias=gamma"}); got != "gamma" {
		t.Fatalf("aliasFlag(--alias=gamma) = %q", got)
	}
}

// TestCmdFleetGrantRevoke is F1's operator surface: grant/revoke are the ONLY way
// orchestrator authority is ever created or destroyed, they bind to the pinned peer
// key, they show up in `fleet ls`, and both write an audit line to fleet.log.
func TestCmdFleetGrantRevoke(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://relay.test")

	var out bytes.Buffer
	if err := cmdFleet("", []string{"link", "--alias", "boxa"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet link: %v", err)
	}
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatalf("load cfg: %v", err)
	}
	fs, err := app.OpenFleetStore(cfg.StateDir)
	if err != nil {
		t.Fatalf("open store: %v", err)
	}

	// Before the peer key is pinned there is nothing to bind a grant to.
	if err := cmdFleet("", []string{"grant", "boxa"}, &bytes.Buffer{}, &bytes.Buffer{}); err == nil {
		t.Fatal("grant succeeded on an edge with no pinned peer key")
	}
	edges, _ := fs.Edges()
	fp := strings.Repeat("a", 43)
	if _, _, err := fs.PinPeerKeyFP(edges[0].EdgeID, fp); err != nil {
		t.Fatalf("pin: %v", err)
	}

	out.Reset()
	if err := cmdFleet("", []string{"grant", "boxa", "--ttl", "12h"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet grant: %v", err)
	}
	if !strings.Contains(out.String(), fp) || !strings.Contains(out.String(), "Expires") {
		t.Fatalf("grant output missing the binding/expiry:\n%s", out.String())
	}
	if !strings.Contains(out.String(), "cannot approve pairings") && !strings.Contains(out.String(), "still cannot approve") {
		t.Fatalf("grant output must state what authority does NOT cover:\n%s", out.String())
	}
	edges, _ = fs.Edges()
	if edges[0].Authority == nil || edges[0].Authority.KeyFP != fp || edges[0].Authority.ExpiresAt == "" {
		t.Fatalf("grant not persisted as bound+expiring: %+v", edges[0].Authority)
	}

	out.Reset()
	if err := cmdFleet("", []string{"ls"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet ls: %v", err)
	}
	if !strings.Contains(out.String(), "authority=granted") {
		t.Fatalf("ls does not show the grant:\n%s", out.String())
	}

	// A bad TTL is refused rather than silently ignored.
	if err := cmdFleet("", []string{"grant", "boxa", "--ttl", "nope"}, &bytes.Buffer{}, &bytes.Buffer{}); err == nil {
		t.Fatal("grant accepted an unparsable --ttl")
	}

	out.Reset()
	if err := cmdFleet("", []string{"revoke", "boxa"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet revoke: %v", err)
	}
	if !strings.Contains(out.String(), "next inbound frame") {
		t.Fatalf("revoke output missing the take-effect note:\n%s", out.String())
	}
	edges, _ = fs.Edges()
	if edges[0].Authority != nil {
		t.Fatalf("revoke left a grant: %+v", edges[0].Authority)
	}
	// A second revoke is a plain no-op, not an error.
	out.Reset()
	if err := cmdFleet("", []string{"revoke", "boxa"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("idempotent revoke: %v", err)
	}
	if !strings.Contains(out.String(), "no orchestrator authority") {
		t.Fatalf("second revoke should say there was nothing to revoke:\n%s", out.String())
	}

	// Both operator acts are in the fleet.log audit trail.
	log, err := os.ReadFile(filepath.Join(cfg.StateDir, "fleet.log"))
	if err != nil {
		t.Fatalf("read fleet.log: %v", err)
	}
	if !strings.Contains(string(log), "AUTHORITY GRANTED by operator") || !strings.Contains(string(log), "AUTHORITY REVOKED by operator") {
		t.Fatalf("fleet.log missing the grant/revoke audit lines:\n%s", log)
	}
}

func TestTTLFlag(t *testing.T) {
	if d, err := ttlFlag(nil); err != nil || d != 0 {
		t.Fatalf("no --ttl should mean no expiry: %v %v", d, err)
	}
	if d, err := ttlFlag([]string{"--ttl", "90m"}); err != nil || d != 90*time.Minute {
		t.Fatalf("ttlFlag(--ttl 90m) = %v, %v", d, err)
	}
	if d, err := ttlFlag([]string{"--ttl=2h"}); err != nil || d != 2*time.Hour {
		t.Fatalf("ttlFlag(--ttl=2h) = %v, %v", d, err)
	}
	for _, bad := range [][]string{{"--ttl"}, {"--ttl", "0s"}, {"--ttl", "-1h"}, {"--nope"}} {
		if _, err := ttlFlag(bad); err == nil {
			t.Fatalf("ttlFlag(%v) accepted a bad value", bad)
		}
	}
}

// TestCmdFleetLsLivenessFlags is F2's operator surface: a cold-retrying edge reads as
// recoverable (never "removed"), and outbound queued for a peer that has gone quiet is
// flagged instead of accumulating invisibly.
func TestCmdFleetLsLivenessFlags(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://relay.test")
	if err := cmdFleet("", []string{"link", "--alias", "boxb"}, &bytes.Buffer{}, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet link: %v", err)
	}
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatalf("load cfg: %v", err)
	}
	fs, err := app.OpenFleetStore(cfg.StateDir)
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	edges, _ := fs.Edges()
	id := edges[0].EdgeID

	// One frame queued for a peer that has never connected, plus the cold-retry mark.
	frame := []byte(`{"t":"fleet_msg","cid":"flt-lsflags00001","text":"queued"}`)
	if _, _, err := fs.EnqueueOutboundTx(id, "flt-lsflags00001", frame); err != nil {
		t.Fatalf("enqueue: %v", err)
	}
	if err := fs.MarkEdgeUnreachable(id, 12); err != nil {
		t.Fatalf("mark unreachable: %v", err)
	}

	var out bytes.Buffer
	if err := cmdFleet("", []string{"ls"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet ls: %v", err)
	}
	if !strings.Contains(out.String(), "stale_pending") || !strings.Contains(out.String(), "unreachable:cold_retry") {
		t.Fatalf("ls does not surface the liveness flags:\n%s", out.String())
	}
	// Recoverable, not dead: `unreachable` must never be rendered as a removal.
	if strings.Contains(out.String(), "dead") || strings.Contains(out.String(), "removed") {
		t.Fatalf("a cold-retrying edge is being shown as dead/removed:\n%s", out.String())
	}
	out.Reset()
	if err := cmdFleet("", []string{"ls", "--json"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("fleet ls --json: %v", err)
	}
	if !strings.Contains(out.String(), `"stale_pending": true`) || !strings.Contains(out.String(), `"unreachable"`) {
		t.Fatalf("ls --json missing the liveness fields:\n%s", out.String())
	}
}
