package app

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/loop"
)

// writeFile writes a fixture file under the box state root.
func writeStateFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

const capsLoopsFixture = `{"loops":[` +
	`{"label":"email-triage","every":"5m","cmd":"echo hi","approved":true,"createdAt":"2026-01-01T00:00:00Z"},` +
	`{"label":"nightly-report","every":"1h","cmd":"echo x","approved":true,"paused":true,"createdAt":"2026-01-01T00:00:00Z"}` +
	`]}`

const capsSchedulesFixture = `{"schedules":[` +
	`{"id":"a1b2c3","prompt":"do the SECRET private thing","source":"app","chatId":"app",` +
	`"recurrence":{"kind":"daily","timeOfDay":"09:00"},"nextFire":"2026-07-24T09:00:00Z","createdAt":"2026-01-01T00:00:00Z"}` +
	`]}`

// TestCapsManifestDeterminism proves the builder reads every field deterministically
// from box state (caps-design §1), that schedule PROMPTS never leak (names are
// id+cadence only), and that a paused loop is counted but not active.
func TestCapsManifestDeterminism(t *testing.T) {
	s := newFleetServer(t)
	if _, _, err := s.store.SeedIdentityName("BoxA"); err != nil {
		t.Fatalf("seed identity: %v", err)
	}
	s.agentInfo = AgentInfo{Harness: "pi", Model: "luna", Effort: "xhigh"}
	s.appVersion = AppVersion{Version: "1.2.3", Commit: "abcdef0", Date: "2026-01-01"}
	writeStateFile(t, loop.Path(s.stateRoot()), capsLoopsFixture)
	writeStateFile(t, s.schedulesPath(), capsSchedulesFixture)

	c := s.buildCapsManifest()

	if c.V != fleetCapsVersion || c.At == "" {
		t.Fatalf("v/at wrong: %+v", c)
	}
	if c.Box != "BoxA" {
		t.Fatalf("box = %q want BoxA", c.Box)
	}
	if len(c.KeyFP) != 43 {
		t.Fatalf("key_fp not a full thumbprint: %q", c.KeyFP)
	}
	if c.Bin == nil || c.Bin.Version != "1.2.3" || c.Bin.Commit != "abcdef0" {
		t.Fatalf("bin wrong: %+v", c.Bin)
	}
	if c.StartedAt == "" || c.UptimeS < 0 {
		t.Fatalf("uptime/started_at wrong: started=%q up=%d", c.StartedAt, c.UptimeS)
	}
	if c.Harness == nil || c.Harness.Kind != "pi" || c.Harness.Model != "luna" || c.Harness.Effort != "xhigh" {
		t.Fatalf("harness wrong: %+v", c.Harness)
	}
	if c.Loops == nil || c.Loops.Count != 2 || c.Loops.Active != 1 {
		t.Fatalf("loops wrong: %+v", c.Loops)
	}
	// B3: COUNTS ONLY — loop/schedule NAMES are never exported (agent-controlled free
	// text with no durable provenance would exfiltrate private text to every peer).
	if len(c.Loops.Names) != 0 {
		t.Fatalf("loop names must not be exported (B3): %+v", c.Loops.Names)
	}
	if c.Schedules == nil || c.Schedules.Count != 1 || c.Schedules.Active != 1 {
		t.Fatalf("schedules wrong: %+v", c.Schedules)
	}
	if len(c.Schedules.Names) != 0 {
		t.Fatalf("schedule names must not be exported (B3): %+v", c.Schedules.Names)
	}
	if c.Fleet == nil {
		t.Fatalf("fleet edge count missing")
	}
	// Zero agent-supplied fields, and neither the schedule PROMPT nor any label rides the
	// manifest.
	blob, _ := json.Marshal(c)
	for _, forbidden := range []string{"SECRET", "do the", "private thing", "email-triage", "nightly-report", "a1b2c3"} {
		if strings.Contains(string(blob), forbidden) {
			t.Fatalf("manifest leaked forbidden token %q: %s", forbidden, blob)
		}
	}
}

// TestCapsFourKiBBound proves the serialized manifest stays within the 4 KiB cap and
// that fitWithin trims name lists (then falls back to counts-only) under a tight bound
// (caps-design §1).
func TestCapsFourKiBBound(t *testing.T) {
	s := newFleetServer(t)
	// 40 loops with long labels — the builder clamps names to 16 entries of 48 runes.
	var b strings.Builder
	b.WriteString(`{"loops":[`)
	for i := 0; i < 40; i++ {
		if i > 0 {
			b.WriteString(",")
		}
		b.WriteString(`{"label":"` + strings.Repeat("x", 80) + string(rune('a'+i%26)) + `","every":"5m","cmd":"c","approved":true,"createdAt":"2026-01-01T00:00:00Z"}`)
	}
	b.WriteString(`]}`)
	writeStateFile(t, loop.Path(s.stateRoot()), b.String())

	c := s.buildCapsManifest()
	if c.Loops.Count != 40 {
		t.Fatalf("loop count = %d want 40", c.Loops.Count)
	}
	if len(c.Loops.Names) > fleetCapsMaxNames {
		t.Fatalf("names not clamped to %d: %d", fleetCapsMaxNames, len(c.Loops.Names))
	}
	for _, n := range c.Loops.Names {
		if len([]rune(n)) > fleetCapsMaxNameRunes {
			t.Fatalf("name not clamped to %d runes: %q", fleetCapsMaxNameRunes, n)
		}
	}
	wire, _ := s.currentCapsWire()
	if len(wire) > fleetCapsMaxBytes {
		t.Fatalf("manifest %d bytes exceeds the %d cap", len(wire), fleetCapsMaxBytes)
	}
	// Tight bound: fitWithin must trim names and still return within budget.
	c2 := s.buildCapsManifest()
	tight := c2.fitWithin(300)
	if len(tight) > 300 {
		t.Fatalf("fitWithin(300) returned %d bytes", len(tight))
	}
}

// TestCapsOldPeerCompat proves the mixed-version safety rails (caps-design §2):
// (a) a welcome_fleet carrying caps is parseable by an OLD (caps-blind) welcome
// struct — the extra field is ignored, never a protocol error;
// (b) applyWelcome reports capsAware=false for a caps-less welcome (a NEW dialer then
// never sends fleet_caps to an OLD serve peer) and true + stores caps when present;
// (c) a NEW serve session advertises the caps signal in its welcome AND accepts an
// inbound fleet_caps frame without closing.
func TestCapsOldPeerCompat(t *testing.T) {
	s := newFleetServer(t)

	// (a) old-binary tolerance of the extra welcome field.
	capsBlob := json.RawMessage(`{"v":1,"at":"2026-07-23T00:00:00Z","key_fp":"fp-one","harness":{"kind":"pi","model":"luna"}}`)
	welcome := welcomeFleetFrame("edgeabcd", "PeerBox", "fp-one", "pi", "luna", capsBlob)
	var oldView struct {
		Box   string `json:"box"`
		KeyFP string `json:"key_fp"`
	}
	if err := json.Unmarshal(welcome, &oldView); err != nil {
		t.Fatalf("old welcome parser rejected caps-bearing welcome: %v", err)
	}
	if oldView.Box != "PeerBox" || oldView.KeyFP != "fp-one" {
		t.Fatalf("old welcome parse wrong: %+v", oldView)
	}

	// (b) capsAware gating.
	edge := dialEdge(t, s, "peer")
	if ok, aware := s.applyWelcome(edge, mustMarshal(map[string]any{"t": "welcome_fleet", "box": "PeerBox", "key_fp": "fp-one"})); !ok || aware {
		t.Fatalf("caps-less welcome: ok=%v aware=%v (want true,false)", ok, aware)
	}
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc != nil {
		t.Fatalf("caps stored from a caps-less welcome")
	}
	if ok, aware := s.applyWelcome(edge, welcomeFleetFrame(edge.EdgeID, "PeerBox", "fp-one", "pi", "luna", capsBlob)); !ok || !aware {
		t.Fatalf("caps-bearing welcome: ok=%v aware=%v (want true,true)", ok, aware)
	}
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc == nil || pc.Caps.Harness == nil || pc.Caps.Harness.Model != "luna" {
		t.Fatalf("caps not stored from a caps-bearing welcome: %+v", pc)
	}

	// (c) serve advertises the signal + accepts an inbound fleet_caps frame.
	sedge, hello := serveEdge(t, s, "dialerpeer")
	capsFrame := []byte(`{"t":"fleet_caps","caps":{"v":1,"at":"2026-07-23T00:00:00Z","key_fp":"fp-dialer","harness":{"kind":"pi"}}}`)
	res, frames := runFleetSession(t, s, sedge, hello, capsFrame)
	if res.closeCode != 0 {
		t.Fatalf("serve session should not close on a fleet_caps frame: %+v", res)
	}
	w := frameOfType(frames, "welcome_fleet")
	if w == nil || w["caps"] == nil {
		t.Fatalf("serve welcome did not advertise the caps signal: %v", w)
	}
	pc, _ := s.fleetStore.PeerCaps(sedge.EdgeID)
	if pc == nil || pc.Caps.Harness == nil || pc.Caps.Harness.Kind != "pi" {
		t.Fatalf("serve did not store the dialer's fleet_caps: %+v", pc)
	}
}

// TestCapsRefreshOnLoopChange proves the fingerprint gate (caps-design §5): an
// unchanged box does not resend (fingerprint stable across polls), a loops.json change
// flips the fingerprint, and broadcastCapsLocked resends only on a real change and only
// to caps-sendable sessions.
func TestCapsRefreshOnLoopChange(t *testing.T) {
	s := newFleetServer(t)

	_, fp1 := s.currentCapsWire()
	_, fp1b := s.currentCapsWire()
	if fp1 != fp1b {
		t.Fatalf("fingerprint not stable across two builds: %q vs %q", fp1, fp1b)
	}
	writeStateFile(t, loop.Path(s.stateRoot()), capsLoopsFixture)
	_, fp2 := s.currentCapsWire()
	if fp2 == fp1 {
		t.Fatalf("loops.json change did not move the fingerprint")
	}

	// Two live sessions: one caps-sendable, one not.
	aware, _ := serveEdge(t, s, "aware")
	unaware, _ := serveEdge(t, s, "unaware")
	var awareFrames, unawareFrames [][]byte
	deregA := s.registerFleetSession(aware.EdgeID, func(b []byte) error { awareFrames = append(awareFrames, b); return nil }, true)
	defer deregA()
	deregU := s.registerFleetSession(unaware.EdgeID, func(b []byte) error { unawareFrames = append(unawareFrames, b); return nil }, false)
	defer deregU()

	// B2: the broadcaster reads the CACHE, so refresh it first (what the 60s poll does).
	s.refreshCapsOut()
	s.capsMu.Lock()
	s.broadcastCapsLocked() // first: fingerprint changes "" -> X, sends
	s.broadcastCapsLocked() // second: unchanged, no send
	s.capsMu.Unlock()
	if got := capsFrameCount(awareFrames); got != 1 {
		t.Fatalf("caps-sendable session got %d caps frames, want 1", got)
	}
	if got := capsFrameCount(unawareFrames); got != 0 {
		t.Fatalf("non-caps-sendable session got %d caps frames, want 0", got)
	}

	// A real change resends (refresh the cache first, as the poll would).
	writeStateFile(t, s.schedulesPath(), capsSchedulesFixture)
	s.refreshCapsOut()
	s.capsMu.Lock()
	s.broadcastCapsLocked()
	s.capsMu.Unlock()
	if got := capsFrameCount(awareFrames); got != 2 {
		t.Fatalf("after change, caps-sendable session got %d caps frames, want 2", got)
	}
}

// TestCapsPreambleMatrix proves the inbound tag upgrade logic (caps-design §3.3 +
// George's FINAL taste call #2 — minimal [box-attested]) and its interaction with a
// key mismatch (mismatch always wins).
func TestCapsPreambleMatrix(t *testing.T) {
	cases := []struct {
		claim    string
		mismatch bool
		attest   fleetAttest
		want     string
	}{
		{"", false, fleetAttestNone, "boxa"},
		{"BoxA", false, fleetAttestNone, "boxa [claims: BoxA, unverified]"},
		{"BoxA", false, fleetAttestBox, "boxa [box-attested]"},
		{"BoxA", false, fleetAttestBoxStale, "boxa [box-attested, stale]"},
		{"BoxA", true, fleetAttestBox, "boxa [claims: BoxA, KEY MISMATCH]"},
		{"", true, fleetAttestBox, "boxa [KEY MISMATCH, unverified]"},
	}
	for _, tc := range cases {
		if got := fleetUserDisplay("boxa", tc.claim, tc.mismatch, tc.attest); got != tc.want {
			t.Fatalf("display(claim=%q mismatch=%v attest=%d) = %q want %q", tc.claim, tc.mismatch, tc.attest, got, tc.want)
		}
	}
}

// TestCapsAttestFor proves the attestation state machine (caps-design §3.3/§5): a
// manifest whose key_fp matches the pin is box-attested; a manifest older than 24h is
// stale; a non-matching key_fp or no manifest is none.
func TestCapsAttestFor(t *testing.T) {
	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "peer")

	// B2: the attestation reads the in-memory cache; seedCapsAttest rebuilds it from disk
	// (what startup does). Re-seed after each direct disk mutation below.
	s.seedCapsAttest()
	// No pin, no caps → none.
	if got := s.capsAttestFor(edge); got != fleetAttestNone {
		t.Fatalf("no caps: attest = %d want none", got)
	}
	// Pin the peer key, store a matching manifest → box-attested.
	if _, _, err := s.fleetStore.PinPeerKeyFP(edge.EdgeID, "fp-peer"); err != nil {
		t.Fatalf("pin: %v", err)
	}
	if err := s.fleetStore.SetPeerCaps(edge.EdgeID, FleetCaps{V: 1, KeyFP: "fp-peer"}); err != nil {
		t.Fatalf("set caps: %v", err)
	}
	s.seedCapsAttest()
	live, _ := s.fleetStore.LiveEdge(edge.EdgeID)
	if got := s.capsAttestFor(live); got != fleetAttestBox {
		t.Fatalf("matching manifest: attest = %d want box", got)
	}
	// A non-matching manifest key_fp → none (never attest a mismatch).
	if err := s.fleetStore.SetPeerCaps(edge.EdgeID, FleetCaps{V: 1, KeyFP: "fp-other"}); err != nil {
		t.Fatalf("set caps: %v", err)
	}
	s.seedCapsAttest()
	if got := s.capsAttestFor(live); got != fleetAttestNone {
		t.Fatalf("mismatched manifest key_fp: attest = %d want none", got)
	}
	// An old received_at → stale.
	if err := s.fleetStore.mutateEdgeState(edge.EdgeID, func(st *fleetEdgeState) error {
		st.PeerCaps = &FleetPeerCaps{Caps: FleetCaps{V: 1, KeyFP: "fp-peer"}, ReceivedAt: time.Now().Add(-48 * time.Hour).UTC().Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("mutate: %v", err)
	}
	s.seedCapsAttest()
	if got := s.capsAttestFor(live); got != fleetAttestBoxStale {
		t.Fatalf("old manifest: attest = %d want stale", got)
	}
	// B4: a persisted KeyFPMismatch overrides attestation even with a matching manifest.
	if err := s.fleetStore.FlagKeyFPMismatch(edge.EdgeID); err != nil {
		t.Fatalf("flag mismatch: %v", err)
	}
	if err := s.fleetStore.mutateEdgeState(edge.EdgeID, func(st *fleetEdgeState) error {
		st.PeerCaps = &FleetPeerCaps{Caps: FleetCaps{V: 1, KeyFP: "fp-peer"}, ReceivedAt: time.Now().UTC().Format(time.RFC3339)}
		return nil
	}); err != nil {
		t.Fatalf("mutate: %v", err)
	}
	s.seedCapsAttest()
	if got, mm := s.capsAttestForID(edge.EdgeID); got != fleetAttestNone || !mm {
		t.Fatalf("persisted mismatch: attest = %d mismatch=%v want none,true", got, mm)
	}
}

// TestCapsOperatorPathIsolation proves the load-bearing safety rail (caps-design §1,
// constraint 1): NO caps code path touches the operator outbox / mailbox / durable
// state. It mirrors TestFleetOperatorMailboxIsolation but exercises the caps surfaces —
// an inbound fleet_caps frame AND a caps broadcast — and asserts the operator durable
// head, mailbox head, queued items, and outbox.jsonl bytes are all untouched.
func TestCapsOperatorPathIsolation(t *testing.T) {
	s := newFleetServer(t)

	link, err := s.store.MintLinkMode(fleetTestRelay, "Operator", false)
	if err != nil {
		t.Fatalf("mint operator room: %v", err)
	}
	const deviceID = "dev-abc123"
	if _, _, err := s.store.VerifyAndLink(link.Room, deviceID, link.Secret); err != nil {
		t.Fatalf("verify+link: %v", err)
	}
	if err := s.provisionMailbox(deviceID); err != nil {
		t.Fatalf("provision mailbox: %v", err)
	}
	_, _, _, sub, err := s.mailbox.stateAndSubscribe(deviceID)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	defer s.mailbox.unsubscribe(deviceID, sub)

	outboxPath := filepath.Join(s.cfg.StateDir, "outbox.jsonl")
	outboxBefore := readFileOrEmpty(t, outboxPath)
	durBefore := s.durableHead()
	cursorBefore := s.outbox.cursor()
	opHeadBefore := s.mailbox.contiguousHead(deviceID)

	// Caps surface 1: a serve session receives an inbound fleet_caps frame.
	edge, hello := serveEdge(t, s, "peer")
	capsFrame := []byte(`{"t":"fleet_caps","caps":{"v":1,"at":"2026-07-23T00:00:00Z","key_fp":"fp-x","harness":{"kind":"pi"}}}`)
	if res, _ := runFleetSession(t, s, edge, hello, capsFrame); res.closeCode != 0 {
		t.Fatalf("fleet_caps serve session should end cleanly: %+v", res)
	}
	// Caps surface 2: a change broadcast to a live caps-sendable session.
	var peerFrames [][]byte
	dereg := s.registerFleetSession(edge.EdgeID, func(b []byte) error { peerFrames = append(peerFrames, b); return nil }, true)
	writeStateFile(t, loop.Path(s.stateRoot()), capsLoopsFixture)
	s.capsMu.Lock()
	s.broadcastCapsLocked()
	s.capsMu.Unlock()
	dereg()

	// The operator durable state is byte-identical: caps wrote nothing durable, moved no
	// mailbox head, queued no item, and the peer never got an operator frame.
	if got := s.durableHead(); got != durBefore {
		t.Fatalf("caps advanced the operator durable head %d -> %d", durBefore, got)
	}
	// A2: the raw shared outbox cursor is untouched by any caps OR fleet_state activity
	// the session's attach triggered (fleet_state now rides a separate seq domain).
	if got := s.outbox.cursor(); got != cursorBefore {
		t.Fatalf("fleet activity moved the raw shared outbox cursor %d -> %d", cursorBefore, got)
	}
	if got := s.mailbox.contiguousHead(deviceID); got != opHeadBefore {
		t.Fatalf("caps moved the operator mailbox head %d -> %d", opHeadBefore, got)
	}
	if after := readFileOrEmpty(t, outboxPath); string(after) != string(outboxBefore) {
		t.Fatalf("caps changed outbox.jsonl: %d -> %d bytes", len(outboxBefore), len(after))
	}
	select {
	case f := <-sub.transients:
		t.Fatalf("operator device received a transient from the caps path: %s", f)
	default:
	}
	// The peer got exactly a fleet_caps frame on the fleet lane (never an operator frame).
	if capsFrameCount(peerFrames) != 1 {
		t.Fatalf("caps-sendable peer got %d caps frames, want 1", capsFrameCount(peerFrames))
	}
}

// TestCapsLoopLabelNeverExported is the review B3 proof: an agent creates a loop whose
// LABEL carries a secret, through the real setup_loop side effect (loop.Setup, the exact
// call the MCP handleSetupLoop makes on the agent path — no self-approve). The label must
// never appear in ANY serialized manifest bytes (builder output or bounded wire).
func TestCapsLoopLabelNeverExported(t *testing.T) {
	s := newFleetServer(t)
	const secret = "EXFIL-a9f3-secret-prompt-fragment"
	// Agent path: sourceSet=false, preApprove=false (agents cannot self-approve loops).
	if _, err := loop.Setup(s.stateRoot(), loop.Loop{
		Label: secret, Every: "5m", Cmd: "echo hi",
	}, false, false, time.Now()); err != nil {
		t.Fatalf("setup_loop: %v", err)
	}

	c := s.buildCapsManifest()
	if c.Loops == nil || c.Loops.Count != 1 {
		t.Fatalf("loop not counted: %+v", c.Loops)
	}
	if len(c.Loops.Names) != 0 {
		t.Fatalf("loop names exported (B3): %+v", c.Loops.Names)
	}
	// The secret must not appear in the builder output, the bounded wire, or the cached wire.
	blob, _ := json.Marshal(c)
	wire, _ := s.currentCapsWire()
	s.refreshCapsOut()
	cached, _ := s.currentCapsWireCached()
	for _, b := range [][]byte{blob, wire, cached} {
		if strings.Contains(string(b), secret) {
			t.Fatalf("agent-controlled loop label leaked into a manifest: %s", b)
		}
	}
}

// TestCapsBindingRejectsUnboundManifest is the review B4 proof: storePeerCaps rejects a
// manifest whose key_fp does not equal the expected (pinned/session) key, and one with a
// bad version / unparsable at — none is ever stored.
func TestCapsBindingRejectsUnboundManifest(t *testing.T) {
	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "peer")
	at := time.Now().UTC().Format(time.RFC3339)

	// key_fp != expected → rejected.
	s.storePeerCaps(edge.EdgeID, "fp-pinned", json.RawMessage(`{"v":1,"at":"`+at+`","key_fp":"fp-attacker"}`))
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc != nil {
		t.Fatalf("stored a manifest not bound to the pinned key: %+v", pc)
	}
	// bad version → rejected.
	s.storePeerCaps(edge.EdgeID, "fp-pinned", json.RawMessage(`{"v":2,"at":"`+at+`","key_fp":"fp-pinned"}`))
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc != nil {
		t.Fatalf("stored a manifest with a bad version: %+v", pc)
	}
	// unparsable at → rejected.
	s.storePeerCaps(edge.EdgeID, "fp-pinned", json.RawMessage(`{"v":1,"at":"not-a-time","key_fp":"fp-pinned"}`))
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc != nil {
		t.Fatalf("stored a manifest with an unparsable at: %+v", pc)
	}
	// A bound manifest IS stored, and the attestation cache reflects it.
	s.storePeerCaps(edge.EdgeID, "fp-pinned", json.RawMessage(`{"v":1,"at":"`+at+`","key_fp":"fp-pinned"}`))
	if pc, _ := s.fleetStore.PeerCaps(edge.EdgeID); pc == nil || pc.Caps.KeyFP != "fp-pinned" {
		t.Fatalf("bound manifest not stored: %+v", pc)
	}
}

// TestCapsFourKiBFailClosed is the review B5 proof: an oversized model/version string and
// an escaped-unicode expansion both yield a wire PROVABLY ≤ 4 KiB — never oversized.
func TestCapsFourKiBFailClosed(t *testing.T) {
	s := newFleetServer(t)
	// Oversized model + version + a control-char run that JSON-escapes to \u00XX (6 bytes
	// each) — the escaped-unicode expansion case.
	s.agentInfo = AgentInfo{Harness: "pi", Model: strings.Repeat("M", 9000), Effort: strings.Repeat("\x01", 4000)}
	s.appVersion = AppVersion{Version: strings.Repeat("V", 9000), Commit: strings.Repeat("\x02", 4000), Date: strings.Repeat("D", 9000)}

	wire, _ := s.currentCapsWire()
	if len(wire) > fleetCapsMaxBytes {
		t.Fatalf("oversized scalars produced a %d-byte wire (> %d cap)", len(wire), fleetCapsMaxBytes)
	}
	// It must still be valid JSON with v==1.
	var c FleetCaps
	if err := json.Unmarshal(wire, &c); err != nil || c.V != fleetCapsVersion {
		t.Fatalf("fail-closed wire is not a valid v1 manifest: err=%v %s", err, wire)
	}
	// A pathological tiny budget forces the minimal {v,at} fallback — still ≤ budget.
	c2 := s.buildCapsManifest()
	tiny := c2.fitWithin(20)
	if len(tiny) > 20 {
		t.Fatalf("fitWithin(20) returned %d bytes (fail-open)", len(tiny))
	}
	var c3 FleetCaps
	if err := json.Unmarshal(tiny, &c3); err != nil || c3.V != fleetCapsVersion {
		t.Fatalf("minimal fallback is not a valid v1 manifest: err=%v %s", err, tiny)
	}
}

func capsFrameCount(frames [][]byte) int {
	n := 0
	for _, f := range frames {
		var m map[string]any
		if json.Unmarshal(f, &m) == nil && m["t"] == "fleet_caps" {
			n++
		}
	}
	return n
}
