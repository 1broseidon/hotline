package app

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/schedule"
	"github.com/1broseidon/hotline/internal/supervise"
)

// This file is Lane L5 — the fleet agent-CAPABILITIES layer (caps-design-2026-07-23).
// It replaces the peer's "trust me" self-report ([claims: …, unverified]) with a
// BOX-ATTESTED manifest: the peer's BOX asserts facts it reads deterministically
// from its own state (harness, model, loops, schedules, uptime, binary), the same
// trust posture as the box-stamped from{} on fleet_msg — the peer's AGENT cannot
// fib, and the manifest is displayed only when the sender's pinned key_fp matches
// (M5). It NEVER touches the operator outbox / reserveTransient / mailboxes /
// device broadcasts — everything rides the fleet lane and fleet state files.
//
// Wire (caps-design §2): the serve side embeds caps in welcome_fleet (the "I speak
// caps" signal an old dialer safely ignores as an unknown field); the dial side,
// having seen that signal, replies with the new fleet_caps frame (an old serve peer
// never receives it, so it never protocol_errors on an unknown frame type). Caps are
// STATE not messages: latest-wins, transient, never journaled, never acked.

// fleetCapsVersion freezes the manifest schema (caps-design §1). Bumped only on a
// breaking manifest shape change.
const fleetCapsVersion = 1

const (
	// fleetCapsMaxBytes caps the serialized manifest (caps-design §1): the builder
	// truncates name lists first, then drops them, before ever exceeding it.
	fleetCapsMaxBytes = 4 * 1024
	// fleetCapsMaxFrameBytes bounds an inbound fleet_caps frame (caps-design §2 abuse
	// bounds): a larger frame is dropped + logged, never parsed.
	fleetCapsMaxFrameBytes = 8 * 1024
	// fleetCapsMaxNames / fleetCapsMaxNameRunes bound each name list (caps-design §1):
	// at most 16 entries, each at most 48 runes, control-stripped.
	fleetCapsMaxNames     = 16
	fleetCapsMaxNameRunes = 48
	// fleetCapsMaxCount clamps a received count field to a sane maximum (caps-design
	// §2): a peer cannot claim an absurd loop/schedule/edge count.
	fleetCapsMaxCount = 4096
	// fleetCapsPollInterval is how often the off-hot-path refresh goroutine re-reads
	// loops.json / schedules.json (written by other processes) and resends on a delta
	// (caps-design §5). Cheap, off every hot path, never under fsMu.
	fleetCapsPollInterval = 60 * time.Second
	// fleetCapsDebounce is the min interval between change-driven resends (caps-design
	// §5, "debounced ≥30s, same throttle discipline as fleetStateChanged").
	fleetCapsDebounce = 30 * time.Second
	// fleetCapsRateWindow is the inbound accept floor per edge (caps-design §2): at
	// most one fleet_caps per 30s per edge; the excess is dropped + logged (no close).
	fleetCapsRateWindow = 30 * time.Second
	// fleetCapsStaleAfter is when a stored manifest is marked stale in the preamble /
	// surfaces (caps-design §5): older than 24h (or the edge disconnected).
	fleetCapsStaleAfter = 24 * time.Hour
)

// AppVersion is the box binary's version info (cmd/hotline main.version/commit/date),
// resolved in package main and threaded into NewProvider so the caps builder — which
// lives in internal/app and cannot read package main — can stamp bin{} (caps-design
// §1, box-read).
type AppVersion struct {
	Version string
	Commit  string
	Date    string
}

// FleetCaps is the box-attested capabilities manifest (caps-design §1). Every field
// is optional except V and At; an absent field means UNKNOWABLE on that box (the
// builder never guesses). Exported so the CLI (`fleet ls --json`) and the fleet MCP
// tool (`action:"caps"`) can render a stored peer manifest.
type FleetCaps struct {
	V         int               `json:"v"`
	At        string            `json:"at"`
	Box       string            `json:"box,omitempty"`
	KeyFP     string            `json:"key_fp,omitempty"`
	Bin       *FleetCapsBin     `json:"bin,omitempty"`
	UptimeS   int64             `json:"uptime_s,omitempty"`
	StartedAt string            `json:"started_at,omitempty"`
	Harness   *FleetCapsHarness `json:"harness,omitempty"`
	Loops     *FleetCapsList    `json:"loops,omitempty"`
	Schedules *FleetCapsList    `json:"schedules,omitempty"`
	Fleet     *FleetCapsFleet   `json:"fleet,omitempty"`
}

// FleetCapsBin is the box binary identity (versionInfo, box-read).
type FleetCapsBin struct {
	Version string `json:"version,omitempty"`
	Commit  string `json:"commit,omitempty"`
	Date    string `json:"date,omitempty"`
}

// FleetCapsHarness is the box's harness identity. Kind/StartedAt are box-read; Model
// and Effort are harness-REPORTED (over the control channel) and box-relayed — one
// hop weaker, still not agent-token-writable (caps-design §4, two-tier provenance).
type FleetCapsHarness struct {
	Kind      string `json:"kind,omitempty"`
	StartedAt string `json:"started_at,omitempty"`
	Model     string `json:"model,omitempty"`
	Effort    string `json:"effort,omitempty"`
}

// FleetCapsList is a count/active tally with a bounded, cadence-only NAME list
// (caps-design §1). For loops the name is the label; for schedules it is
// id + " " + Describe(recurrence) — NEVER the prompt/body (prompts can hold private
// detail; the id+cadence is the capability).
type FleetCapsList struct {
	Count     int      `json:"count"`
	Active    int      `json:"active"`
	Names     []string `json:"names,omitempty"`
	Truncated bool     `json:"truncated,omitempty"`
}

// FleetCapsFleet is the live (non-tombstoned) edge count only — no aliases/ids: v1
// does not ship topology to peers (caps-design §1).
type FleetCapsFleet struct {
	Edges int `json:"edges"`
}

// FleetPeerCaps is the per-edge stored peer manifest (caps-design §3): the received,
// clamped manifest plus the LOCAL clock arrival time (skew-proof staleness anchor).
type FleetPeerCaps struct {
	Caps       FleetCaps `json:"caps"`
	ReceivedAt string    `json:"received_at"`
}

// buildCapsManifest is the pure builder (caps-design §1): it reads loops.json +
// schedules.json fresh from disk, the in-memory agentInfo/startedAt/version, and the
// registry edge count. It runs ONLY off the hot path (session-start + the caps timer
// goroutine, per post-mortem fix 3), never inline on the fleet_msg send/receive path
// and never under fsMu. Every field is read deterministically from box state — ZERO
// agent-supplied fields (caps-design §1). An unreadable/absent field is simply
// omitted; the builder never guesses.
func (s *Server) buildCapsManifest() FleetCaps {
	c := FleetCaps{V: fleetCapsVersion, At: time.Now().UTC().Format(time.RFC3339)}
	c.Box = s.fleetBoxName()
	if fp, err := s.fleetKeyFP(); err == nil {
		c.KeyFP = fp
	}
	if v := s.appVersion; v.Version != "" || v.Commit != "" || v.Date != "" {
		c.Bin = &FleetCapsBin{Version: v.Version, Commit: v.Commit, Date: v.Date}
	}
	// Box process uptime, sender-computed so clock skew can never fake it (caps-design
	// §1/§5). The box process IS this connector process; startedAt is set in NewServer.
	if !s.startedAt.IsZero() {
		c.StartedAt = s.startedAt.UTC().Format(time.RFC3339)
		if up := int64(time.Since(s.startedAt).Seconds()); up > 0 {
			c.UptimeS = up
		}
	}
	h := &FleetCapsHarness{}
	info := s.currentAgentInfo()
	h.Kind = info.Harness
	// harness.model / .effort: harness-reported, box-relayed. Unknown → omit; an
	// empty value (never reported, or an explicit clear) carries nothing, so omit too.
	if info.Model != "" {
		h.Model = info.Model
	}
	if info.Effort != "" {
		h.Effort = info.Effort
	}
	// harness.started_at from the supervisor state (box-read); omit when the box has
	// no supervisor (a bare `hotline run`, or a test config). This is a small disk
	// read, done off the hot path exactly like the loops/schedules reads below.
	if s.supervisorDir != "" {
		if st, err := supervise.ReadState(s.supervisorDir); err == nil && st != nil && st.HarnessStartedAt != "" {
			h.StartedAt = st.HarnessStartedAt
		}
	}
	if h.Kind != "" || h.Model != "" || h.Effort != "" || h.StartedAt != "" {
		c.Harness = h
	}
	// loops.* — box-read (caps-design §1): COUNTS ONLY (review B3). Loop LABELS are
	// agent-controlled free text (setup_loop) with no durable operator/CLI provenance,
	// so exporting them would let an agent exfiltrate arbitrary private text (a secret in
	// a label) to every paired peer. Until provenance exists, ship count + active only —
	// never names. (This amends the operator "names on" taste call; security overrides.)
	if d, err := loop.Load(loop.Path(s.stateRoot())); err == nil {
		lst := &FleetCapsList{}
		for _, l := range d.Loops {
			lst.Count++
			if !l.Paused {
				lst.Active++
			}
		}
		c.Loops = lst
	}
	// schedules.* — box-read (caps-design §1): COUNTS ONLY (review B3), same rationale
	// as loops. Schedule names (id + cadence) also lack durable provenance in v1.
	if d, err := schedule.Load(s.schedulesPath()); err == nil {
		lst := &FleetCapsList{}
		for _, sc := range d.Schedules {
			lst.Count++
			if !sc.Paused {
				lst.Active++
			}
		}
		c.Schedules = lst
	}
	// fleet.edges — live (non-tombstoned) count only, no topology (caps-design §1).
	if s.fleetStore != nil {
		if edges, err := s.fleetStore.Edges(); err == nil {
			n := 0
			for _, e := range edges {
				if !e.Removed() {
					n++
				}
			}
			c.Fleet = &FleetCapsFleet{Edges: n}
		}
	}
	return c
}

// clampRunes truncates s to at most n Unicode code points.
func clampRunes(s string, n int) string {
	r := []rune(s)
	if len(r) > n {
		return string(r[:n])
	}
	return s
}

// clampCount bounds a received count field to [0, fleetCapsMaxCount] (caps-design §2).
func clampCount(n int) int {
	if n < 0 {
		return 0
	}
	if n > fleetCapsMaxCount {
		return fleetCapsMaxCount
	}
	return n
}

// clamp sanitizes a RECEIVED manifest before it is stored or displayed (caps-design
// §2/§4): every string is control-stripped + length-clamped, every count clamped to a
// sane maximum, and every name list bounded. Peer caps are peer data — this runs on
// receipt, before the manifest touches any state file, CLI/tool output, or the user
// meta line.
func (c *FleetCaps) clamp() {
	c.At = clampIdent(stripFleetControl(c.At))
	c.Box = clampIdent(strings.TrimSpace(stripFleetControl(c.Box)))
	c.KeyFP = clampIdent(stripFleetControl(c.KeyFP))
	c.StartedAt = clampIdent(stripFleetControl(c.StartedAt))
	if c.UptimeS < 0 {
		c.UptimeS = 0
	}
	if c.Bin != nil {
		c.Bin.Version = clampIdent(stripFleetControl(c.Bin.Version))
		c.Bin.Commit = clampIdent(stripFleetControl(c.Bin.Commit))
		c.Bin.Date = clampIdent(stripFleetControl(c.Bin.Date))
	}
	if c.Harness != nil {
		c.Harness.Kind = clampIdent(stripFleetControl(c.Harness.Kind))
		c.Harness.Model = clampIdent(stripFleetControl(c.Harness.Model))
		c.Harness.Effort = clampIdent(stripFleetControl(c.Harness.Effort))
		c.Harness.StartedAt = clampIdent(stripFleetControl(c.Harness.StartedAt))
	}
	clampReceivedList(c.Loops)
	clampReceivedList(c.Schedules)
	if c.Fleet != nil {
		c.Fleet.Edges = clampCount(c.Fleet.Edges)
	}
}

func clampReceivedList(l *FleetCapsList) {
	if l == nil {
		return
	}
	l.Count = clampCount(l.Count)
	l.Active = clampCount(l.Active)
	if len(l.Names) > fleetCapsMaxNames {
		l.Names = l.Names[:fleetCapsMaxNames]
		l.Truncated = true
	}
	for i, n := range l.Names {
		l.Names[i] = clampRunes(stripFleetControl(n), fleetCapsMaxNameRunes)
	}
}

// fleetCapsMaxScalarRunes bounds every OUTGOING manifest scalar before serialization
// (review B5): version/commit/date/model/effort/box/key_fp/started_at can never expand
// the wire past the budget one field at a time.
const fleetCapsMaxScalarRunes = 96

// clampOutgoingScalars bounds every serialized scalar to fleetCapsMaxScalarRunes runes
// and control-strips it (review B5): a huge or escaped-unicode model/version string can
// never make the manifest exceed the budget on its own. Names are already never exported
// (B3), so scalars are the only remaining size lever.
func (c *FleetCaps) clampOutgoingScalars() {
	c.Box = clampRunes(stripFleetControl(c.Box), fleetCapsMaxScalarRunes)
	c.KeyFP = clampRunes(stripFleetControl(c.KeyFP), fleetCapsMaxScalarRunes)
	c.StartedAt = clampRunes(stripFleetControl(c.StartedAt), fleetCapsMaxScalarRunes)
	c.At = clampRunes(stripFleetControl(c.At), fleetCapsMaxScalarRunes)
	if c.Bin != nil {
		c.Bin.Version = clampRunes(stripFleetControl(c.Bin.Version), fleetCapsMaxScalarRunes)
		c.Bin.Commit = clampRunes(stripFleetControl(c.Bin.Commit), fleetCapsMaxScalarRunes)
		c.Bin.Date = clampRunes(stripFleetControl(c.Bin.Date), fleetCapsMaxScalarRunes)
	}
	if c.Harness != nil {
		c.Harness.Kind = clampRunes(stripFleetControl(c.Harness.Kind), fleetCapsMaxScalarRunes)
		c.Harness.Model = clampRunes(stripFleetControl(c.Harness.Model), fleetCapsMaxScalarRunes)
		c.Harness.Effort = clampRunes(stripFleetControl(c.Harness.Effort), fleetCapsMaxScalarRunes)
		c.Harness.StartedAt = clampRunes(stripFleetControl(c.Harness.StartedAt), fleetCapsMaxScalarRunes)
	}
}

// fitWithin returns a serialized manifest PROVABLY ≤ max bytes (review B5, fail-closed):
// it never returns oversized JSON. First it clamps every scalar; if still too large it
// drops optional sections in a DETERMINISTIC order (bin → fleet → schedules → loops →
// harness → uptime/started_at → box); if even the bare identity is somehow too large it
// falls back to the minimal {v, at} object, which is a couple dozen bytes. The only
// fields that survive the final fallback are v and a clamped at, so the result is always
// well under any sane budget.
func (c *FleetCaps) fitWithin(max int) []byte {
	c.clampOutgoingScalars()
	if b, err := json.Marshal(c); err == nil && len(b) <= max {
		return b
	}
	// Deterministic section-drop order (cheapest-signal-loss last).
	drops := []func(*FleetCaps){
		func(x *FleetCaps) { x.Bin = nil },
		func(x *FleetCaps) { x.Fleet = nil },
		func(x *FleetCaps) { x.Schedules = nil },
		func(x *FleetCaps) { x.Loops = nil },
		func(x *FleetCaps) { x.Harness = nil },
		func(x *FleetCaps) { x.UptimeS = 0; x.StartedAt = "" },
		func(x *FleetCaps) { x.Box = ""; x.KeyFP = "" },
	}
	for _, drop := range drops {
		drop(c)
		if b, err := json.Marshal(c); err == nil && len(b) <= max {
			return b
		}
	}
	// Final fail-closed fallback: the minimal valid manifest, provably tiny.
	minimal := FleetCaps{V: c.V, At: clampRunes(c.At, fleetCapsMaxScalarRunes)}
	b, err := json.Marshal(&minimal)
	if err != nil || len(b) > max {
		// {"v":1,"at":""} — unconditionally within any budget ≥ ~15 bytes.
		return []byte(fmt.Sprintf(`{"v":%d,"at":""}`, fleetCapsVersion))
	}
	return b
}

// currentCapsWire builds the current manifest FROM DISK and returns its bounded wire
// bytes plus a content fingerprint (caps-design §5). The fingerprint hashes the manifest
// with the volatile fields (At, UptimeS) zeroed, so an unchanged box does NOT resend
// every poll just because the clock advanced.
//
// B2: this does filesystem reads (loops.json/schedules.json/supervisor), so it runs ONLY
// off the hot path — the refresh goroutine (refreshCapsOut) and MergeAgentInfo. The
// handshake + broadcast paths read the cached result via currentCapsWireCached instead.
func (s *Server) currentCapsWire() (wire []byte, fingerprint string) {
	c := s.buildCapsManifest()
	wire = c.fitWithin(fleetCapsMaxBytes)
	fp := c // shallow copy; zero only the value-typed volatile fields
	fp.At = ""
	fp.UptimeS = 0
	b, _ := json.Marshal(fp)
	sum := sha256.Sum256(b)
	return wire, hex.EncodeToString(sum[:])
}

// refreshCapsOut rebuilds the in-memory OUTGOING manifest cache from disk (B2). Called
// only off the hot path: once at construction, on every 60s refresh tick, and after a
// harness identity change (MergeAgentInfo). Never on a handshake / inbound path.
func (s *Server) refreshCapsOut() {
	wire, fp := s.currentCapsWire()
	s.capsOutMu.Lock()
	s.capsOutWire = wire
	s.capsOutFP = fp
	s.capsOutMu.Unlock()
}

// currentCapsWireCached returns the cached OUTGOING manifest wire + fingerprint with
// ZERO filesystem access (B2) — the read the serve/dial handshakes and the caps
// broadcaster use. The cache is primed at construction and kept fresh by the refresh
// goroutine, so it is effectively always populated; the nil-cache branch is a
// belt-and-suspenders one-time build for a handshake that somehow beats priming.
func (s *Server) currentCapsWireCached() (wire []byte, fingerprint string) {
	s.capsOutMu.RLock()
	w, fp := s.capsOutWire, s.capsOutFP
	s.capsOutMu.RUnlock()
	if w != nil {
		return w, fp
	}
	return s.currentCapsWire()
}

// fleetCapsFrame builds the transient fleet_caps frame carrying a manifest
// (caps-design §2). Never journaled, never acked.
func fleetCapsFrame(caps json.RawMessage) []byte {
	return mustMarshal(map[string]any{"t": "fleet_caps", "caps": caps})
}

// capsBindingOK is the ONE centralized manifest-binding validator (review B4): a
// received manifest is bound iff it is v==1, carries a sane RFC3339 `at`, a non-empty
// key_fp, and — when we already know the peer's key (the pin, or the session key of the
// frame that carried it) — that key_fp EQUALS it. A failing manifest is rejected, never
// stored. When expectedKeyFP is "" (serve side before any fleet_msg pinned the peer)
// the claim is accepted but only ever ATTESTED once a message-identity pin matches it —
// capsAttestFor requires caps.key_fp == the pin, so an unbound claim never displays.
func capsBindingOK(c *FleetCaps, expectedKeyFP string) (bool, string) {
	if c.V != fleetCapsVersion {
		return false, fmt.Sprintf("bad version %d", c.V)
	}
	if t, err := time.Parse(time.RFC3339, c.At); err != nil {
		return false, "unparsable at"
	} else if d := time.Since(t); d > 3650*24*time.Hour || d < -2*24*time.Hour {
		return false, "at out of sane range"
	}
	if c.KeyFP == "" {
		return false, "empty key_fp"
	}
	if expectedKeyFP != "" && c.KeyFP != expectedKeyFP {
		return false, "key_fp not bound to pinned peer key"
	}
	return true, ""
}

// storePeerCaps parses, clamps, VALIDATES BINDING (review B4), and durably stores a
// peer's manifest on the edge state.json with a local received_at (caps-design §3,
// latest-wins), then refreshes the in-memory attestation cache (B2). expectedKeyFP is
// the key the manifest must be bound to (the pin, or the session key of the carrying
// frame; "" on a serve edge not yet pinned). A manifest failing binding is REJECTED —
// never stored. Called from the welcome handler (dial) and the fleet_caps handler (both
// sides) — never on the fleet_msg hot path. A tombstoned/missing edge is a no-op inside
// the store.
func (s *Server) storePeerCaps(edgeID, expectedKeyFP string, raw json.RawMessage) {
	if s.fleetStore == nil {
		return
	}
	var c FleetCaps
	if json.Unmarshal(raw, &c) != nil {
		s.fleetLog.logf("edge=%s peer caps dropped: unparsable", shortEdgeID(edgeID))
		return
	}
	c.clamp()
	if ok, why := capsBindingOK(&c, expectedKeyFP); !ok {
		s.fleetLog.logf("edge=%s peer caps REJECTED (unbound): %s", shortEdgeID(edgeID), why)
		return
	}
	if err := s.fleetStore.SetPeerCaps(edgeID, c); err != nil {
		s.fleetLog.logf("edge=%s peer caps store failed: %v", shortEdgeID(edgeID), err)
		return
	}
	s.cacheSetCaps(edgeID, c.KeyFP, time.Now())
}

// handleFleetCaps applies an inbound fleet_caps frame (caps-design §2): frame-size
// bound, per-edge rate cap (1/30s), then parse + clamp + BIND (B4) + store. No ack, not
// journaled; a lost frame self-heals on the next reconnect or change-resend. The
// manifest is bound to the edge's pinned peer key (edge.PeerKeyFP) when one exists.
func (s *Server) handleFleetCaps(edge FleetEdge, raw []byte) {
	tag := shortEdgeID(edge.EdgeID)
	if len(raw) > fleetCapsMaxFrameBytes {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_caps dropped: frame %d > cap %d", tag, len(raw), fleetCapsMaxFrameBytes)
		return
	}
	if !s.fleetCapsRateAllow(edge.EdgeID) {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_caps dropped: rate cap 1/%s", tag, fleetCapsRateWindow)
		return
	}
	var env struct {
		Caps json.RawMessage `json:"caps"`
	}
	if json.Unmarshal(raw, &env) != nil || len(env.Caps) == 0 {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_caps dropped: no caps object", tag)
		return
	}
	s.storePeerCaps(edge.EdgeID, edge.PeerKeyFP, env.Caps)
	s.fleetLog.logf("edge=%s dir=in kind=fleet_caps stored (%d bytes)", tag, len(env.Caps))
}

// fleetCapsRateAllow reports whether an inbound fleet_caps is within the 1/30s per-edge
// floor (caps-design §2). In-memory (the box is the sole serving process).
func (s *Server) fleetCapsRateAllow(edgeID string) bool {
	now := time.Now()
	s.fleetCapsRateMu.Lock()
	defer s.fleetCapsRateMu.Unlock()
	if last, ok := s.fleetCapsRate[edgeID]; ok && now.Sub(last) < fleetCapsRateWindow {
		return false
	}
	s.fleetCapsRate[edgeID] = now
	return true
}

// fleetCapsChanged is the caps change hook (caps-design §5): it schedules a
// fingerprint-gated resend to every caps-sendable live session, debounced to at most
// one per fleetCapsDebounce with a trailing send (mirrors fleetStateChanged). Called
// by the agentInfo merge hook and by the 60s poll goroutine. Takes only capsMu — no
// disk I/O under fsMu, never on a fleet_msg hot path.
func (s *Server) fleetCapsChanged() {
	if s.fleetStore == nil {
		return
	}
	s.capsMu.Lock()
	defer s.capsMu.Unlock()
	if s.capsClosed {
		return
	}
	if s.capsTimer != nil {
		return // a trailing send is already scheduled; it will carry the latest manifest
	}
	wait := fleetCapsDebounce - time.Since(s.capsLastSent)
	if wait <= 0 {
		s.broadcastCapsLocked()
		return
	}
	s.capsTimer = time.AfterFunc(wait, func() {
		s.capsMu.Lock()
		defer s.capsMu.Unlock()
		s.capsTimer = nil
		if s.capsClosed {
			return
		}
		s.broadcastCapsLocked()
	})
}

// broadcastCapsLocked resends the current manifest to every caps-sendable session, but
// ONLY when its content fingerprint changed since the last send (caps-design §5). Caller
// holds capsMu.
func (s *Server) broadcastCapsLocked() {
	s.capsLastSent = time.Now()
	wire, fp := s.currentCapsWireCached()
	if fp == s.capsFingerprint {
		return // unchanged content — nothing to resend
	}
	s.capsFingerprint = fp
	frame := fleetCapsFrame(json.RawMessage(wire))
	for _, w := range s.capsSendableWriters() {
		_ = w(frame)
	}
}

// capsSendableWriters returns the writers of every live session we may push a
// fleet_caps frame to without risking an old-peer protocol_error (caps-design §2):
// serve sessions always (an old dialer ignores the unknown frame type), dial sessions
// only when the peer advertised caps-awareness in its welcome.
func (s *Server) capsSendableWriters() []func([]byte) error {
	s.fleetSessMu.Lock()
	defer s.fleetSessMu.Unlock()
	var out []func([]byte) error
	for _, sess := range s.fleetSessions {
		if sess.canSendCaps {
			out = append(out, sess.write)
		}
	}
	return out
}

// runFleetCapsRefresh is the 60s poll goroutine (caps-design §5): loops.json /
// schedules.json are written by OTHER processes (CLI, scheduler, MCP tools), so the box
// polls and resends on a delta. The fingerprint gate inside fleetCapsChanged makes an
// unchanged poll a no-op. Cheap, off every hot path, no fsMu.
func (s *Server) runFleetCapsRefresh(ctx context.Context) {
	if s.fleetStore == nil {
		<-ctx.Done()
		return
	}
	// B2: prime the outgoing manifest cache once at Run start (off every hot path), so
	// the first handshake reads it with no filesystem access. Done here — not at
	// construction — because building the manifest creates the box identity key, which an
	// inert (never-run) box must not create.
	s.refreshCapsOut()
	tick := time.NewTicker(fleetCapsPollInterval)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			// B2: rebuild the outgoing manifest cache from disk HERE (off every hot path),
			// then let the fingerprint gate decide whether to resend.
			s.refreshCapsOut()
			s.pruneFleetCapsRate()
			s.fleetCapsChanged()
		}
	}
}

// pruneFleetCapsRate drops per-edge inbound-rate entries older than the accept window
// (NIT): they permit the next frame anyway, so removing them simply bounds the map so it
// cannot retain every historical edge id indefinitely.
func (s *Server) pruneFleetCapsRate() {
	cutoff := time.Now().Add(-fleetCapsRateWindow)
	s.fleetCapsRateMu.Lock()
	for id, last := range s.fleetCapsRate {
		if last.Before(cutoff) {
			delete(s.fleetCapsRate, id)
		}
	}
	s.fleetCapsRateMu.Unlock()
}

// stopFleetCapsEmitter marks the caps emitter closed and stops any pending trailing
// send (mirrors stopFleetStateEmitter).
func (s *Server) stopFleetCapsEmitter() {
	s.capsMu.Lock()
	defer s.capsMu.Unlock()
	s.capsClosed = true
	if s.capsTimer != nil {
		s.capsTimer.Stop()
		s.capsTimer = nil
	}
}

// fleetAttest is the caps attestation state of an inbound fleet turn (caps-design §3.3):
// none (no matching manifest → today's unverified claim), box-attested, or box-attested
// but stale.
type fleetAttest int

const (
	fleetAttestNone fleetAttest = iota
	fleetAttestBox
	fleetAttestBoxStale
)

// edgeCapsAttest is the in-memory, per-edge attestation cache entry (B2): everything the
// inbound preamble needs to attest a peer WITHOUT touching disk under the per-edge
// delivery lock. pinFP is the pinned peer key (registry), mismatch the persisted M5
// KeyFPMismatch, and hasCaps/capsKeyFP/receivedAt the last stored manifest binding.
type edgeCapsAttest struct {
	pinFP      string
	mismatch   bool
	hasCaps    bool
	capsKeyFP  string
	receivedAt time.Time
}

// capsAttestForID computes the attestation state from the in-memory cache (B2 + B4):
// box-attested iff a stored manifest is bound to the pinned peer key AND no persisted
// KeyFPMismatch overrides it; stale when older than 24h; none otherwise. Zero disk I/O —
// safe to call under the per-edge delivery lock. Returns the persisted mismatch flag too
// so the caller can OR it into the display (B4: a persisted mismatch always wins, even
// when the current message's fingerprint matches or is omitted).
func (s *Server) capsAttestForID(edgeID string) (attest fleetAttest, mismatch bool) {
	s.capsAttestMu.Lock()
	e := s.capsAttest[edgeID]
	s.capsAttestMu.Unlock()
	if e == nil {
		return fleetAttestNone, false
	}
	mismatch = e.mismatch
	if e.mismatch || !e.hasCaps || e.pinFP == "" || e.capsKeyFP == "" || e.capsKeyFP != e.pinFP {
		return fleetAttestNone, mismatch
	}
	if !e.receivedAt.IsZero() && time.Since(e.receivedAt) > fleetCapsStaleAfter {
		return fleetAttestBoxStale, mismatch
	}
	return fleetAttestBox, mismatch
}

// capsAttestFor is the edge-typed convenience wrapper used by tests. It defers to the
// in-memory cache (capsAttestForID) — the SAME pin-aware logic the inbound path uses.
func (s *Server) capsAttestFor(edge FleetEdge) fleetAttest {
	attest, _ := s.capsAttestForID(edge.EdgeID)
	return attest
}

// seedCapsAttest rebuilds the in-memory attestation cache from disk (B2): once at
// construction, and re-run by tests after they mutate edge state directly. It reads the
// pin (registry) + persisted mismatch + stored manifest for every non-removed edge, so
// the inbound path never has to. Off the hot path.
func (s *Server) seedCapsAttest() {
	if s.fleetStore == nil {
		return
	}
	edges, err := s.fleetStore.Edges()
	if err != nil {
		return
	}
	next := map[string]*edgeCapsAttest{}
	for _, e := range edges {
		if e.Removed() {
			continue
		}
		ent := &edgeCapsAttest{pinFP: e.PeerKeyFP}
		if st, serr := s.fleetStore.EdgeState(e.EdgeID); serr == nil {
			ent.mismatch = st.KeyFPMismatch
			if st.PeerCaps != nil {
				ent.hasCaps = true
				ent.capsKeyFP = st.PeerCaps.Caps.KeyFP
				if t, perr := time.Parse(time.RFC3339, st.PeerCaps.ReceivedAt); perr == nil {
					ent.receivedAt = t
				}
			}
		}
		next[e.EdgeID] = ent
	}
	s.capsAttestMu.Lock()
	s.capsAttest = next
	s.capsAttestMu.Unlock()
}

// cacheEntryLocked returns (creating) the cache entry for an edge. Caller holds capsAttestMu.
func (s *Server) cacheEntryLocked(edgeID string) *edgeCapsAttest {
	e := s.capsAttest[edgeID]
	if e == nil {
		e = &edgeCapsAttest{}
		s.capsAttest[edgeID] = e
	}
	return e
}

// cacheSetPin records a first-use pin in the attestation cache (B2), mirroring the
// durable PinPeerKeyFP write so the inbound path never reloads the registry.
func (s *Server) cacheSetPin(edgeID, fp string) {
	s.capsAttestMu.Lock()
	e := s.cacheEntryLocked(edgeID)
	if e.pinFP == "" {
		e.pinFP = fp
	}
	s.capsAttestMu.Unlock()
}

// cacheSetMismatch records the persisted M5 mismatch in the cache (B2/B4): once set it
// overrides attestation until the edge is removed.
func (s *Server) cacheSetMismatch(edgeID string) {
	s.capsAttestMu.Lock()
	s.cacheEntryLocked(edgeID).mismatch = true
	s.capsAttestMu.Unlock()
}

// cacheSetCaps records a freshly stored, bound manifest in the cache (B2).
func (s *Server) cacheSetCaps(edgeID, capsKeyFP string, receivedAt time.Time) {
	s.capsAttestMu.Lock()
	e := s.cacheEntryLocked(edgeID)
	e.hasCaps = true
	e.capsKeyFP = capsKeyFP
	e.receivedAt = receivedAt
	s.capsAttestMu.Unlock()
}

// Summary renders a stored manifest as a one-line capability summary for `fleet ls`
// and the fleet tool (caps-design §3), e.g. "pi/luna 4 loops 2 sched up 2d6h".
func (c FleetCaps) Summary() string {
	var parts []string
	if c.Harness != nil {
		hp := c.Harness.Kind
		if c.Harness.Model != "" {
			if hp != "" {
				hp += "/" + c.Harness.Model
			} else {
				hp = c.Harness.Model
			}
		}
		if hp != "" {
			parts = append(parts, hp)
		}
	}
	if c.Loops != nil {
		parts = append(parts, fmt.Sprintf("%d loops", c.Loops.Count))
	}
	if c.Schedules != nil {
		parts = append(parts, fmt.Sprintf("%d sched", c.Schedules.Count))
	}
	if c.UptimeS > 0 {
		parts = append(parts, "up "+humanDurShort(c.UptimeS))
	}
	return strings.Join(parts, " ")
}

// humanDurShort renders a second count as a compact duration (caps-design §3): "2d6h",
// "6h5m", "5m", "45s".
func humanDurShort(sec int64) string {
	if sec < 0 {
		sec = 0
	}
	d := sec / 86400
	h := (sec % 86400) / 3600
	m := (sec % 3600) / 60
	switch {
	case d > 0:
		if h > 0 {
			return fmt.Sprintf("%dd%dh", d, h)
		}
		return fmt.Sprintf("%dd", d)
	case h > 0:
		if m > 0 {
			return fmt.Sprintf("%dh%dm", h, m)
		}
		return fmt.Sprintf("%dh", h)
	case m > 0:
		return fmt.Sprintf("%dm", m)
	default:
		return fmt.Sprintf("%ds", sec)
	}
}
