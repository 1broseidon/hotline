package app

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"path/filepath"
	"sync"
	"time"
)

// This file is the box's minimal serve-side fleet session handler (A2A v2 §3.1),
// Lane L1. A fleet room is served ONLY by this handler, which speaks NO app
// protocol at all (no typing/read/blobs/push/SDK/agent_state — F4 dead
// structurally): the accepted inbound set is exactly {hello, ping, fleet_ack,
// fleet_msg}, and everything else closes the session with a protocol error + a
// fleet.log line. e=1 is REQUIRED — a plaintext hello never reaches this handler
// because the envelope codec drops any non-e1 frame at the socket boundary
// (readSessionInputs, connector.go), so hello-on-plaintext gets no reply (F1).
//
// The edge is re-verified fresh from the registry at hello AND before every
// journaled frame (B2), so a `hotline fleet rm` (tombstone) terminates a live
// session promptly and a peer can never keep an edge alive past removal. Frames
// are bounded (outer ws read limit + a decrypted-size cap) and every field is
// length/charset-validated before use (B3). fleet_msg is journaled as a frozen,
// fsync'd, monotonically-sequenced wire frame (B4); agent injection is Lane L2.

// fleetHello is the peer's device hello against a served fleet room. It carries
// NO push block (§3.1: no push registration — F4).
type fleetHello struct {
	T        string `json:"t"`
	V        int    `json:"v"`
	DeviceID string `json:"device_id"`
	Secret   string `json:"secret"`
}

// fleetFrom is the sender identity stamped on a fleet_msg (§4, F9/E3). On the
// receive side it is the PEER's identity — taken from the frame when present,
// else synthesized from the edge's stored peer fields.
type fleetFrom struct {
	Box     string `json:"box,omitempty"`
	KeyFP   string `json:"key_fp,omitempty"`
	Harness string `json:"harness,omitempty"`
	Model   string `json:"model,omitempty"`
}

// fleetMsgFrame is the frozen wire shape (§4, F9). L1 validates + journals it;
// agent injection is Lane L2.
type fleetMsgFrame struct {
	T    string     `json:"t"`
	CID  string     `json:"cid"`
	Text string     `json:"text"`
	Kind string     `json:"kind,omitempty"`
	TS   string     `json:"ts,omitempty"`
	From *fleetFrom `json:"from,omitempty"`
}

// fleetResult is the outcome of one fleet session: a non-zero closeCode is
// written as a WebSocket close by the conn wrapper.
type fleetResult struct {
	closeCode   int
	closeReason string
}

// welcomeFleetFrame carries the box identity both directions (§3.1, E3/F14):
// {edge_id, box, key_fp, harness, model}. key_fp is a full RFC 7638 JWK
// thumbprint and is REQUIRED (the caller refuses to serve without it —
// should-fix 2); harness/model are omitted when empty.
func welcomeFleetFrame(edgeID, box, keyFP, harness, model string, caps json.RawMessage) []byte {
	m := map[string]any{"t": "welcome_fleet", "v": ProtocolVersion, "edge_id": edgeID, "box": box, "key_fp": keyFP}
	if harness != "" {
		m["harness"] = harness
	}
	if model != "" {
		m["model"] = model
	}
	// caps-design §2: embedding the box-attested manifest in welcome_fleet IS the "I
	// speak caps" signal. An old dialer ignores the unknown field (JSON unmarshal into
	// its welcome struct drops it), so this is old-binary-safe both directions.
	if len(caps) > 0 {
		m["caps"] = caps
	}
	return mustMarshal(m)
}

// readFleetHello drains input until the first exact-typed frame. It must be a
// well-formed hello with a flt-<edgeID> device id (§3.1); anything else closes
// with a protocol error. Mirrors readHello (connector.go) but for the fleet role.
func (s *Server) readFleetHello(ctx context.Context, tag string, inputs <-chan sessionInput, write func([]byte) error) (fleetHello, bool) {
	for {
		var in sessionInput
		select {
		case <-ctx.Done():
			return fleetHello{}, false
		case in = <-inputs:
		}
		if in.err != nil {
			return fleetHello{}, false
		}
		if len(in.raw) > fleetMaxFrameBytes {
			s.fleetLog.logf("edge=%s hello refused: frame %d > cap %d", tag, len(in.raw), fleetMaxFrameBytes)
			_ = write(errorFrame("protocol_error", "frame too large"))
			return fleetHello{}, false
		}
		t, exact := exactType(in.raw)
		if !exact {
			s.fleetLog.logf("edge=%s hello refused: malformed frame", tag)
			_ = write(errorFrame("protocol_error", "malformed frame"))
			return fleetHello{}, false
		}
		if t != "hello" {
			s.fleetLog.logf("edge=%s hello refused: first frame kind=%q", tag, t)
			_ = write(errorFrame("protocol_error", "first fleet frame must be hello"))
			return fleetHello{}, false
		}
		var hello fleetHello
		if json.Unmarshal(in.raw, &hello) != nil || !fleetDeviceRE.MatchString(hello.DeviceID) || hello.Secret == "" || len(hello.Secret) > fleetMaxIdentField {
			s.fleetLog.logf("edge=%s hello refused: invalid shape", tag)
			_ = write(errorFrame("protocol_error", "invalid fleet hello"))
			return fleetHello{}, false
		}
		return hello, true
	}
}

// serveFleetSession runs one fleet session for a serve-side edge (§3.1). It
// re-loads the edge FRESH from the registry (B2), verifies the hello secret,
// requires a box identity key (should-fix 2), sends welcome_fleet, then handles
// ping / fleet_ack / fleet_msg until the peer disconnects, the edge is removed,
// or the protocol is violated. It writes ONLY to the edge journal and fleet.log
// — never the operator mailbox/outbox/sink.
func (s *Server) serveFleetSession(ctx context.Context, edgeID string, hello fleetHello, inputs <-chan sessionInput, write func([]byte) error) fleetResult {
	tag := shortEdgeID(edgeID)
	edge, ok := s.liveEdge(edgeID)
	if !ok || edge.Secret == "" || !hashEqual(hello.Secret, secretHash(edge.Secret)) {
		s.fleetLog.logf("edge=%s refused hello: unauthorized", tag)
		_ = write(errorFrame("unauthorized", ""))
		return fleetResult{closeCode: 4003, closeReason: "unauthorized"}
	}
	keyFP, err := s.fleetKeyFP()
	if err != nil || keyFP == "" {
		// should-fix 2: fail serving rather than emit an identity-less welcome.
		s.fleetLog.logf("edge=%s refused: box identity key unavailable: %v", tag, err)
		_ = write(errorFrame("server_error", "box identity unavailable"))
		return fleetResult{closeCode: 4000, closeReason: "server_error"}
	}
	// caps-design §2: the serve side carries its box-attested manifest in welcome_fleet
	// (the caps-awareness signal). B2: read the CACHED wire — zero filesystem access on
	// session establishment (the cache is kept fresh by the refresh goroutine).
	capsWire, _ := s.currentCapsWireCached()
	if err := write(welcomeFleetFrame(edge.EdgeID, s.fleetBoxName(), keyFP, s.fleetHarness(), s.fleetModel(), capsWire)); err != nil {
		return fleetResult{}
	}
	s.fleetLog.logf("edge=%s alias=%q device=%q session up", tag, edge.Alias, hello.DeviceID)
	// Register the live session so fleet_send + the caps broadcaster can push to this
	// peer (H4). A serve session may always push fleet_caps: an old dialer ignores the
	// unknown frame type (caps-design §2). Draining is concurrent + resume-pruned (B2).
	deregister := s.attachFleetSession(edgeID, write, true)
	defer deregister()
	ob := newFleetOutbox()
	resumeSeen := make(chan struct{})
	var resumeOnce sync.Once
	drainCtx, drainCancel := context.WithCancel(ctx)
	s.sendFleetResume(edgeID, write)
	drainDone := make(chan struct{})
	go func() { defer close(drainDone); s.drainPendingConcurrent(drainCtx, edgeID, write, ob, resumeSeen) }()
	// Wait for the drain goroutine to fully exit before the session returns — no
	// orphaned goroutine writes past the session's lifetime.
	defer func() { drainCancel(); <-drainDone }()
	// Note (review B5b): a `fleet rm` sends an authenticated revoked to THIS active
	// session deterministically from the room manager's reap path (it holds the live
	// writer), then cancels ctx below — no liveness-poll race in the session loop.
	for {
		var in sessionInput
		select {
		case <-ctx.Done():
			return fleetResult{}
		case in = <-inputs:
		}
		if in.err != nil {
			return fleetResult{}
		}
		if len(in.raw) > fleetMaxFrameBytes {
			// Cap decoder memory BEFORE parsing (B3).
			s.fleetLog.logf("edge=%s refused: frame %d > cap %d", tag, len(in.raw), fleetMaxFrameBytes)
			_ = write(errorFrame("protocol_error", "frame too large"))
			return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
		}
		t, exact := exactType(in.raw)
		if !exact {
			// A malformed frame (no exact lowercase t) is NOT tolerated (B3).
			s.fleetLog.logf("edge=%s refused: malformed frame", tag)
			_ = write(errorFrame("protocol_error", "malformed frame"))
			return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
		}
		switch t {
		case "ping":
			n, ok := parsePing(in.raw)
			if !ok {
				s.fleetLog.logf("edge=%s dir=in kind=ping refused: bad n", tag)
				_ = write(errorFrame("protocol_error", "ping.n must be a non-negative integer"))
				return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
			}
			_ = write(pongFrame(n))
		case "fleet_ack":
			// {cid}: the peer acked one of our delivered fleet_msg by echoing its cid
			// (C1). Durably drain the matching pending outbound (review B4) and mark
			// the session outbox so a concurrent drain skips it (B2).
			var a struct {
				CID string `json:"cid"`
			}
			if json.Unmarshal(in.raw, &a) != nil || !fleetCIDRE.MatchString(a.CID) {
				s.fleetLog.logf("edge=%s dir=in kind=fleet_ack refused: bad cid", tag)
				_ = write(errorFrame("protocol_error", "fleet_ack.cid must be a valid client id"))
				return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
			}
			s.fleetTouch(edgeID)
			s.recordPeerAck(edgeID, a.CID, ob)
		case "fleet_resume":
			// The peer advertised its committed-inbound cids (review B2): prune our
			// matching outbound without a body replay, and release the drain grace.
			s.peerResume(edgeID, in.raw, ob)
			resumeOnce.Do(func() { close(resumeSeen) })
		case "fleet_msg":
			switch s.deliverInbound(ctx, edge, edge.Alias, in.raw, write, true) {
			case inCloseProtocol:
				return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
			case inCloseRevoked:
				return fleetResult{closeCode: 4003, closeReason: "revoked"}
			}
		case "fleet_caps":
			// caps-design §2: the dialer's box-attested manifest (its establish-time send
			// or a mid-session change). State, not a message: stored latest-wins, no ack,
			// never journaled, rate-capped 1/30s.
			s.handleFleetCaps(edge, in.raw)
		default:
			// NO app-protocol surface: device_send/typing/read/presence/blob_*/
			// mailbox_ack/push_challenge/live_activity_token and any unknown type all
			// close with a protocol error (§3.1 — F4 dead structurally).
			s.fleetLog.logf("edge=%s alias=%q dir=in kind=%q refused: not a fleet frame", tag, edge.Alias, t)
			_ = write(errorFrame("protocol_error", "unsupported fleet frame"))
			return fleetResult{closeCode: 4002, closeReason: "protocol_error"}
		}
	}
}

// inboundAction is deliverInbound's directive to the calling session loop.
type inboundAction int

const (
	inKeepSession   inboundAction = iota // handled (or leniently dropped) — continue
	inCloseProtocol                      // strict: malformed frame → close protocol_error
	inCloseRevoked                       // edge no longer live → close revoked
)

// deliverInbound is the SHARED inbound fleet_msg path (review B1/B3) for BOTH the
// serve handler and the dialer. It validates the frame, then runs the single durable
// CommitInbound transaction (liveness + complete journal-cid dedup + rate cap +
// journal append + cursor advance, all under one flock). On a live commit OR a
// durable duplicate it sends the fleet_ack (so the peer drains) and injects to the
// agent (idempotent via InboundDelivered). The ack is sent ONLY when the frame is
// durably held — a rate drop or a store error sends none, so the peer redelivers.
// strict=true (serve) closes on a malformed frame; strict=false (dial) drops it.
func (s *Server) deliverInbound(ctx context.Context, edge FleetEdge, alias string, raw []byte, write func([]byte) error, strict bool) inboundAction {
	tag := shortEdgeID(edge.EdgeID)
	var f fleetMsgFrame
	if json.Unmarshal(raw, &f) != nil || !fleetCIDRE.MatchString(f.CID) {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg refused: bad cid/shape", tag)
		if strict {
			_ = write(errorFrame("protocol_error", "invalid fleet_msg"))
			return inCloseProtocol
		}
		return inKeepSession
	}
	kind := f.Kind
	if kind == "" {
		kind = "brief"
	}
	if !fleetKinds[kind] {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg cid=%q refused: bad kind=%q", tag, f.CID, kind)
		if strict {
			_ = write(errorFrame("protocol_error", "invalid fleet_msg kind"))
			return inCloseProtocol
		}
		return inKeepSession
	}
	if len(f.Text) > fleetTextCap {
		s.fleetLog.logf("edge=%s alias=%q dir=in kind=fleet_msg cid=%q dropped: text %d > cap %d", tag, alias, f.CID, len(f.Text), fleetTextCap)
		if strict {
			_ = write(errorFrame("text_too_large", ""))
		}
		return inKeepSession
	}
	if s.fleetStore == nil {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg reject: no fleet store", tag)
		if strict {
			_ = write(errorFrame("server_error", ""))
			return inCloseProtocol
		}
		return inKeepSession
	}
	// Build the FROZEN, complete wire frame the journal stores (B4): the sender
	// identity is the PEER's — from the frame when present (bounded), else the edge's
	// stored peer fields.
	from := s.resolveFleetFrom(f.From, edge)
	ts := f.TS
	if ts == "" || len(ts) > 40 {
		ts = time.Now().UTC().Format(time.RFC3339Nano)
	}
	wire := map[string]any{"t": "fleet_msg", "cid": f.CID, "ts": ts, "text": f.Text, "kind": kind, "from": from}
	frame, err := json.Marshal(wire)
	if err != nil {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg reject: marshal: %v", tag, err)
		if strict {
			_ = write(errorFrame("server_error", ""))
			return inCloseProtocol
		}
		return inKeepSession
	}
	commit, err := s.fleetStore.CommitInbound(edge.EdgeID, f.CID, frame, func() bool { return s.fleetRateAllow(edge.EdgeID) })
	if err != nil {
		// Durability failure → NO ack; the peer redelivers.
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg cid=%q reject: commit: %v", tag, f.CID, err)
		if strict {
			_ = write(errorFrame("server_error", "journal write failed"))
			return inCloseProtocol
		}
		return inKeepSession
	}
	if !commit.Live {
		s.fleetLog.logf("edge=%s dir=in kind=fleet_msg refused: edge removed", tag)
		if strict {
			_ = write(errorFrame("revoked", ""))
		}
		return inCloseRevoked
	}
	if commit.RateDropped {
		_ = s.fleetStore.IncDroppedInbound(edge.EdgeID)
		s.fleetLog.logf("edge=%s alias=%q dir=in kind=fleet_msg cid=%q dropped: rate cap %d/%s", tag, alias, f.CID, fleetInboundRateN, fleetInboundRateWindow)
		return inKeepSession
	}
	s.fleetTouch(edge.EdgeID)
	if commit.Committed {
		s.fleetLog.logf("edge=%s alias=%q dir=in kind=fleet_msg cid=%q seq=%d bytes=%d", tag, alias, f.CID, commit.Seq, len(f.Text))
		// L4: a new inbound bumped this edge's recv_24h + last_seen — refresh the
		// operator snapshot (throttled to ≥30s).
		s.fleetStateChanged()
	} else {
		s.fleetLog.logf("edge=%s alias=%q dir=in kind=fleet_msg cid=%q duplicate: re-ack (already journaled)", tag, alias, f.CID)
	}
	// Commit-before-ack: the durable CommitInbound already fsynced + advanced the
	// cursor, so the ack now is honest. Inject is idempotent (InboundDelivered).
	s.ackInbound(f.CID, write)
	// F1: the validated kind drives the framing switch (directive vs untrusted peer).
	s.injectInbound(ctx, edge, from, f.CID, f.Text, kind)
	return inKeepSession
}

// resolveFleetFrom picks the sender identity for the journal frame, bounding
// every field. A peer-supplied from wins per field (that is the peer's honest
// self-report); missing fields fall back to the edge's stored peer identity.
func (s *Server) resolveFleetFrom(supplied *fleetFrom, edge FleetEdge) fleetFrom {
	from := fleetFrom{Box: edge.PeerBoxName, KeyFP: edge.PeerKeyFP}
	if supplied != nil {
		if supplied.Box != "" {
			from.Box = supplied.Box
		}
		if supplied.KeyFP != "" {
			from.KeyFP = supplied.KeyFP
		}
		from.Harness = supplied.Harness
		from.Model = supplied.Model
	}
	from.Box = clampIdent(from.Box)
	from.KeyFP = clampIdent(from.KeyFP)
	from.Harness = clampIdent(from.Harness)
	from.Model = clampIdent(from.Model)
	return from
}

func clampIdent(s string) string {
	if len(s) > fleetMaxIdentField {
		return s[:fleetMaxIdentField]
	}
	return s
}

// liveEdge is the authoritative fresh-from-disk edge lookup (B2): ok=false for a
// missing/tombstoned edge or a corrupt registry (fail-closed).
func (s *Server) liveEdge(edgeID string) (FleetEdge, bool) {
	if s.fleetStore == nil {
		return FleetEdge{}, false
	}
	return s.fleetStore.LiveEdge(edgeID)
}

func (s *Server) fleetTouch(edgeID string) {
	if s.fleetStore != nil {
		s.fleetStore.TouchLastSeen(edgeID)
	}
}

// fleetBoxName is the box display identity for welcome_fleet (§4: relayState.Name).
func (s *Server) fleetBoxName() string {
	if s.store == nil {
		return ""
	}
	name, _ := s.store.IdentityName()
	return name
}

// fleetKeyFP is the box-key.json RFC 7638 JWK thumbprint for welcome_fleet (§4,
// should-fix 2). The box identity key is loaded-or-created (like core mode), so a
// serving box always has one; a real filesystem failure returns an error and the
// caller refuses to serve rather than omitting the fp.
func (s *Server) fleetKeyFP() (string, error) {
	if s.cfg == nil || s.cfg.StateDir == "" {
		return "", fmt.Errorf("no state dir for box identity key")
	}
	priv, err := loadOrCreateBoxKey(s.cfg.StateDir)
	if err != nil {
		return "", err
	}
	jwk := publicJWKFor(priv)
	return jwkThumbprintP256(jwk["x"], jwk["y"]), nil
}

func (s *Server) fleetHarness() string { return s.currentAgentInfo().Harness }
func (s *Server) fleetModel() string   { return s.currentAgentInfo().Model }

// jwkThumbprintP256 computes the RFC 7638 JWK thumbprint of a P-256 public key:
// base64url(SHA-256) over the canonical JSON with lexicographically-ordered
// members {crv, kty, x, y} and no whitespace. Full length (never truncated).
func jwkThumbprintP256(x, y string) string {
	canonical := `{"crv":"P-256","kty":"EC","x":"` + x + `","y":"` + y + `"}`
	sum := sha256.Sum256([]byte(canonical))
	return base64.RawURLEncoding.EncodeToString(sum[:])
}

func shortEdgeID(id string) string {
	if len(id) > 8 {
		return id[:8]
	}
	return id
}

// openFleetLogger returns the fleet.log logger under stateDir (§6), reusing the
// connLogger template (connlog.go). Nil-safe; "" stateDir yields a nil no-op.
func openFleetLogger(stateDir string) *connLogger {
	if stateDir == "" {
		return nil
	}
	return &connLogger{path: filepath.Join(stateDir, fleetLogFile)}
}

const fleetLogFile = "fleet.log"
