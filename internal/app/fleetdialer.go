package app

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

// This file is Lane L3 — the fleet DIALER (a2a-design-v2 §3.2). For every
// direction=dial edge the box holds device creds for, it dials the peer's fleet
// room on the DEVICE leg (<relay>/r/<room>/a, F1), speaks e=1 as the device
// (newDeviceEnvelopeCodec — seal K_a2b / open K_b2a), and runs the fleet device
// protocol against the peer's serve-side fleet handler (fleetsession.go).
//
// It reuses the shared inbound/outbound machinery (deliverInbound → the single
// durable CommitInbound tx with complete journal-cid dedup; drainPendingConcurrent
// + fleet_resume for bounded, ack-concurrent redelivery; recordPeerAck → dir=ack
// WAL prune). L3-specific here: the device envelope leg, the welcome handshake as a
// session precondition (valid first key_fp pin; mismatch/re-welcome terminate), a
// welcome timeout, dial-time origin/creds revalidation, and the dead-with-flag /
// dead-on-revoked lifecycle.
//
// FLEET LEG PROTOCOL, v1.1 (review B1-B5 fixes): the leg remains cid-idempotent (no
// wire mailbox cursor — SPEC §3.1-3.8's resume_from/gap/full is NOT spoken), but
// exactly-once is now DURABLE, not ring-bounded: the per-edge journal is the single
// source of truth for both inbound dedup (every committed cid, full history) and the
// outbound WAL (dir=out minus dir=ack), and a compact fleet_resume {have} handshake
// prunes already-delivered outbound without a whole-queue body replay.

// dialOutcome is the result of one dial session.
type dialOutcome struct {
	dead   bool   // the peer revoked us (unauthorized/revoked) → mark dead
	reason string // tombstone reason when dead
	// welcomed reports that the peer's welcome_fleet was accepted, i.e. a REAL
	// handshake completed. F2 keys both the streak reset and the unreachable-revive on
	// it: before, any socket error while awaiting welcome reset the streak as if the
	// edge had connected, so a relay that accepts the TCP dial and then EOFs (the 1006
	// pattern) looked healthy to the counter.
	welcomed bool
}

// runFleetDialManager drives every non-tombstoned direction=dial edge, diffing the
// registry each poll like runFleetRoomManager. It is a no-op when the fleet store
// is absent. One fleetDialLoop goroutine per dial edge; completed loops are reaped
// and respawned if the edge is still live (review SF1).
func (s *Server) runFleetDialManager(ctx context.Context) error {
	if s.fleetStore == nil {
		<-ctx.Done()
		return nil
	}
	loops := map[string]*roomLoopHandle{}

	reap := func(id string) {
		h := loops[id]
		h.cancel()
		<-h.done
		delete(loops, id)
	}

	diff := func() {
		// Reap any loop that exited on its own (dead-marked, unrecoverable creds) so a
		// still-live edge respawns and a dead one simply drops out of the set (SF1).
		for id, h := range loops {
			select {
			case <-h.done:
				delete(loops, id)
			default:
			}
		}
		dials, err := s.fleetStore.DialEdges()
		if err != nil {
			s.fleetLog.logf("fleet dial manager: %v", err)
			return
		}
		set := make(map[string]FleetEdge, len(dials))
		for _, e := range dials {
			set[e.EdgeID] = e
		}
		newCount := 0
		for id, e := range set {
			if _, running := loops[id]; running {
				continue
			}
			lctx, cancel := context.WithCancel(ctx)
			h := &roomLoopHandle{cancel: cancel, done: make(chan struct{})}
			loops[id] = h
			delay := time.Duration(newCount) * dialStagger
			newCount++
			go func(e FleetEdge, h *roomLoopHandle, delay time.Duration) {
				defer close(h.done)
				s.fleetDialLoop(lctx, e, delay)
			}(e, h, delay)
		}
		for id := range loops {
			if _, ok := set[id]; !ok {
				reap(id)
			}
		}
	}

	diff()
	tick := time.NewTicker(relayRoomPollInterval)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			for id := range loops {
				loops[id].cancel()
			}
			for id := range loops {
				<-loops[id].done
			}
			return nil
		case <-tick.C:
			diff()
		}
	}
}

// markDeadRetry tombstones a dial edge, RETRYING on failure (review SF1) so a
// transient state-write error never leaves a live edge un-dialed-but-not-dead.
func (s *Server) markDeadRetry(ctx context.Context, edgeID, reason string) {
	tag := shortEdgeID(edgeID)
	for ctx.Err() == nil {
		if err := s.fleetStore.MarkDialEdgeDead(edgeID, reason); err == nil {
			s.fleetLog.logf("edge=%s dial edge marked DEAD: %s", tag, reason)
			return
		} else {
			s.fleetLog.logf("edge=%s mark-dead failed, retrying: %v", tag, err)
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Second):
		}
	}
}

// markUnreachableRetry records the F2 recoverable dead-mark, retrying a transient
// state-write failure the same way markDeadRetry does — but it never tombstones and
// never zeroes creds, so the edge stays revivable. It also refreshes the operator
// snapshot so a cold edge is visible without tailing the log.
func (s *Server) markUnreachableRetry(ctx context.Context, edgeID string, attempts int) {
	tag := shortEdgeID(edgeID)
	for ctx.Err() == nil {
		if err := s.fleetStore.MarkEdgeUnreachable(edgeID, attempts); err == nil {
			s.fleetStateChanged()
			return
		} else {
			s.fleetLog.logf("edge=%s mark-unreachable failed, retrying: %v", tag, err)
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Second):
		}
	}
}

// reviveUnreachable clears the cold flag after a successful handshake (F2) and says so
// once, so the operator sees the recovery in the same log the death appeared in.
func (s *Server) reviveUnreachable(edgeID string) {
	cleared, err := s.fleetStore.ClearEdgeUnreachable(edgeID)
	if err != nil {
		s.fleetLog.logf("edge=%s revive (clear unreachable) failed: %v", shortEdgeID(edgeID), err)
		return
	}
	if cleared {
		s.fleetLog.logf("edge=%s REVIVED: handshake succeeded, unreachable flag cleared", shortEdgeID(edgeID))
		s.fleetStateChanged()
	}
}

// validateDialCreds re-verifies a dial edge's creds every attempt (review SF3): the
// creds room/relay must match the registry edge, the origin must match the operator-
// approved RelayOrigin, and the device id must be well-formed. A mismatch means the
// persisted state was tampered — the caller marks the edge dead rather than dialing
// an unvetted origin.
func validateDialCreds(edge FleetEdge) error {
	c := edge.DeviceCreds
	if c == nil || c.Secret == "" {
		return fmt.Errorf("no device creds")
	}
	if c.Room != edge.Room {
		return fmt.Errorf("creds room %q != edge room %q", c.Room, edge.Room)
	}
	norm, err := normalizeRendezvous(c.RelayURL)
	if err != nil {
		return fmt.Errorf("bad creds relay url: %w", err)
	}
	if norm != edge.RelayURL {
		return fmt.Errorf("creds relay url %q != edge relay url %q", norm, edge.RelayURL)
	}
	origin, err := rendezvousOrigin(norm)
	if err != nil {
		return err
	}
	if edge.RelayOrigin != "" && origin != edge.RelayOrigin {
		return fmt.Errorf("creds origin %s != approved %s", origin, edge.RelayOrigin)
	}
	if !fleetDeviceRE.MatchString(c.DeviceID) {
		return fmt.Errorf("bad device id %q", c.DeviceID)
	}
	return nil
}

// fleetDialLoop dials one edge's peer room until ctx is cancelled, the edge is
// removed, or the peer revokes us. It reloads the edge fresh each attempt,
// revalidates creds/origin (SF3), and mirrors roomLoop backoff. A dead outcome (or
// N consecutive failed handshakes on a known-good edge — review B5c) tombstones the
// edge and stops.
func (s *Server) fleetDialLoop(ctx context.Context, edge FleetEdge, startDelay time.Duration) {
	if startDelay > 0 {
		select {
		case <-ctx.Done():
			return
		case <-time.After(startDelay):
		}
	}
	tag := shortEdgeID(edge.EdgeID)
	backoff := defaultRelayBackoffMin
	noWelcomeStreak := 0
	everConnected := edge.PeerKeyFP != "" // a pinned edge is known-good already
	// F2: cold is the recoverable dead state, not an exit. A box that starts with the
	// flag already on disk resumes in the cold tier rather than hammering.
	cold := edge.Unreachable != nil

	for ctx.Err() == nil {
		live, ok := s.fleetStore.LiveEdge(edge.EdgeID)
		if !ok {
			s.fleetLog.logf("edge=%s dial stop: edge no longer live", tag)
			return
		}
		edge = live
		if err := validateDialCreds(edge); err != nil {
			s.fleetLog.logf("edge=%s dial stop: creds invalid: %v", tag, err)
			s.markDeadRetry(ctx, edge.EdgeID, "bad_creds")
			return
		}
		codec, err := newDeviceEnvelopeCodec(edge.DeviceCreds.Room, edge.DeviceCreds.Secret)
		if err != nil {
			s.fleetLog.logf("edge=%s dial stop: device codec init failed: %v", tag, err)
			s.markDeadRetry(ctx, edge.EdgeID, "bad_creds")
			return
		}
		// Endpoint from VALIDATED registry fields, never the raw creds string (SF3).
		endpoint := strings.TrimRight(edge.RelayURL, "/") + "/r/" + edge.Room + "/a"
		s.fleetLog.logf("edge=%s /a dialing %s", tag, endpoint)
		conn, resp, derr := s.dial(ctx, endpoint, nil)
		if resp != nil && resp.Body != nil {
			_ = resp.Body.Close()
		}
		if derr == nil {
			connected := time.Now()
			s.fleetLog.logf("edge=%s /a connected", tag)
			outcome := s.dialFleetSession(ctx, conn, edge, codec)
			_ = conn.Close()
			if outcome.dead {
				s.markDeadRetry(ctx, edge.EdgeID, outcome.reason)
				return
			}
			held := time.Since(connected)
			if ctx.Err() != nil {
				return
			}
			if outcome.welcomed {
				// A completed handshake clears the streak and revives the edge (F2).
				noWelcomeStreak = 0
				everConnected = true
				if cold {
					cold = false
					s.reviveUnreachable(edge.EdgeID)
				}
			} else {
				noWelcomeStreak++
			}
			if held >= defaultRelayStableAfter {
				backoff = defaultRelayBackoffMin
			}
		} else {
			s.fleetLog.logf("edge=%s /a dial failed: %v", tag, derr)
			noWelcomeStreak++
		}
		// F2 cold tier (replaces the old terminal dead-mark): a known-good edge that
		// cannot complete a handshake for many consecutive attempts stops hot-retrying
		// and is flagged Unreachable — creds retained, edge still live, still dialed on
		// a long interval — so it revives on its own when the relay comes back instead
		// of costing an operator re-pair.
		if everConnected && noWelcomeStreak >= fleetDialDeadAfter && !cold {
			cold = true
			s.fleetLog.logf("edge=%s UNREACHABLE after %d consecutive failed handshakes on a known-good edge → cold retry every ~%s (recoverable; NOT removed, creds retained)", tag, noWelcomeStreak, fleetDialColdRetry)
			s.markUnreachableRetry(ctx, edge.EdgeID, noWelcomeStreak)
		}
		delay := jitterBackoff(backoff, defaultRelayBackoffMax)
		if cold {
			delay = jitterBackoff(fleetDialColdRetry, fleetDialColdRetryMax)
		}
		s.fleetLog.logf("edge=%s /a reconnect in %s (backoff=%s, streak=%d, cold=%t)", tag, delay.Round(time.Millisecond), backoff, noWelcomeStreak, cold)
		select {
		case <-ctx.Done():
			return
		case <-time.After(delay):
		}
		backoff = nextBackoff(backoff, defaultRelayBackoffMax)
	}
}

// dialFleetSession runs one device-leg session: reconcile inbound state, send the
// device hello, AWAIT welcome_fleet under a timeout (review B5a) with the pin as a
// precondition (review SF2), then attach (register writer + resume + concurrent
// drain) and run the shared duplex loop until the socket closes or ctx is cancelled.
func (s *Server) dialFleetSession(ctx context.Context, conn *websocket.Conn, edge FleetEdge, codec *envelopeCodec) (outcome dialOutcome) {
	tag := shortEdgeID(edge.EdgeID)
	creds := edge.DeviceCreds
	if creds == nil || creds.Secret == "" {
		return dialOutcome{}
	}

	conn.SetReadLimit(fleetMaxFrameBytes)
	var writeMu sync.Mutex
	write := func(raw []byte) error {
		sealed, err := codec.wrap(raw)
		if err != nil {
			return err
		}
		writeMu.Lock()
		defer writeMu.Unlock()
		_ = conn.SetWriteDeadline(time.Now().Add(writeWait))
		return conn.WriteMessage(websocket.TextMessage, sealed)
	}

	inputs := make(chan sessionInput, 1)
	readerDone := make(chan struct{})
	readerStopped := make(chan struct{})
	go func() {
		defer close(readerStopped)
		readSessionInputs(conn, codec, inputs, readerDone, s.fleetLog, tag)
	}()
	defer func() {
		close(readerDone)
		_ = conn.Close()
		<-readerStopped
	}()

	// Realign the inbound cursor with the durable journal before resuming (B3).
	if _, resynced, rerr := s.fleetStore.ReconcileInboundCursor(edge.EdgeID); rerr != nil {
		s.fleetLog.logf("edge=%s inbound reconcile failed: %v", tag, rerr)
	} else if resynced {
		s.fleetLog.logf("edge=%s inbound cursor reconciled to journal tail", tag)
	}

	// Device hello with stored creds.
	hello := mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": creds.DeviceID, "secret": creds.Secret,
	})
	if err := write(hello); err != nil {
		s.fleetLog.logf("edge=%s hello write failed: %v", tag, err)
		return dialOutcome{}
	}

	// Await welcome_fleet under a timeout; the pin is a precondition. Re-send the
	// hello periodically so a hello dropped before the peer's /c leg was connected
	// (the dumb-pipe race) still lands, without waiting the full timeout.
	welcomeTimer := time.NewTimer(fleetWelcomeTimeout)
	defer welcomeTimer.Stop()
	helloTick := time.NewTicker(fleetHelloResend)
	defer helloTick.Stop()
	// caps-design §2: set true when the peer's welcome_fleet carried a caps object —
	// only then may this dialer reply with the fleet_caps frame (an old serve peer,
	// whose welcome had no caps, must never receive it: it would protocol_error 4002).
	peerCapsAware := false
	for {
		var in sessionInput
		select {
		case <-ctx.Done():
			return dialOutcome{}
		case <-welcomeTimer.C:
			s.fleetLog.logf("edge=%s welcome timeout after %s", tag, fleetWelcomeTimeout)
			return dialOutcome{}
		case <-helloTick.C:
			if err := write(hello); err != nil {
				return dialOutcome{}
			}
			continue
		case in = <-inputs:
		}
		if in.err != nil {
			return dialOutcome{}
		}
		t, exact := exactType(in.raw)
		if !exact {
			continue
		}
		if t == "welcome_fleet" {
			ok, aware := s.applyWelcome(edge, in.raw)
			if !ok {
				// empty fp / mismatch / unparsable → terminate; reconnect + backoff.
				return dialOutcome{}
			}
			peerCapsAware = aware
			// F2: from here on this attempt counts as a REAL handshake, whatever ends
			// the session afterwards — the streak reset and the unreachable revive both
			// key on it (a socket that dies before welcome must not read as connected).
			defer func() { outcome.welcomed = true }()
			break
		}
		if t == "error" {
			if outcome, dead := s.dialErrorOutcome(edge, in.raw); dead {
				return outcome
			}
			return dialOutcome{}
		}
		// ignore anything else until welcomed
	}

	// Attach + resume + concurrent drain (review B2), then the shared duplex loop. This
	// dial session may push fleet_caps only when the peer advertised caps-awareness
	// (caps-design §2).
	deregister := s.attachFleetSession(edge.EdgeID, write, peerCapsAware)
	defer deregister()
	ob := newFleetOutbox()
	resumeSeen := make(chan struct{})
	var resumeOnce sync.Once
	drainCtx, drainCancel := context.WithCancel(ctx)
	s.sendFleetResume(edge.EdgeID, write)
	drainDone := make(chan struct{})
	go func() { defer close(drainDone); s.drainPendingConcurrent(drainCtx, edge.EdgeID, write, ob, resumeSeen) }()
	defer func() { drainCancel(); <-drainDone }()
	// caps-design §2: right after applyWelcome + attach, send this box's manifest to a
	// caps-aware serve peer (the serve side got ours; it hands back its own via welcome).
	// B2: cached wire — zero filesystem access on session establishment.
	if peerCapsAware {
		if capsWire, _ := s.currentCapsWireCached(); len(capsWire) > 0 {
			_ = write(fleetCapsFrame(json.RawMessage(capsWire)))
		}
	}
	s.fleetLog.logf("edge=%s dial session up device=%q", tag, creds.DeviceID)

	liveTick := time.NewTicker(fleetLiveCheckInterval)
	defer liveTick.Stop()
	for {
		var in sessionInput
		select {
		case <-ctx.Done():
			return dialOutcome{}
		case <-liveTick.C:
			if _, ok := s.liveEdge(edge.EdgeID); !ok {
				// A LOCAL rm of this dial edge → stop the session (the loop then stops).
				s.fleetLog.logf("edge=%s dial session ending: edge removed locally", tag)
				return dialOutcome{}
			}
			continue
		case in = <-inputs:
		}
		if in.err != nil {
			return dialOutcome{}
		}
		t, exact := exactType(in.raw)
		if !exact {
			s.fleetLog.logf("edge=%s dir=in dropped: malformed frame", tag)
			continue
		}
		switch t {
		case "welcome_fleet":
			// A re-welcome mid-session is a protocol violation (SF2) → terminate.
			s.fleetLog.logf("edge=%s dir=in re-welcome rejected: terminating session", tag)
			return dialOutcome{}
		case "ping":
			if n, ok := parsePing(in.raw); ok {
				_ = write(pongFrame(n))
			}
		case "pong":
			// keepalive echo
		case "fleet_ack":
			var a struct {
				CID string `json:"cid"`
			}
			if json.Unmarshal(in.raw, &a) == nil && fleetCIDRE.MatchString(a.CID) {
				s.fleetTouch(edge.EdgeID)
				s.recordPeerAck(edge.EdgeID, a.CID, ob)
			}
		case "fleet_resume":
			s.peerResume(edge.EdgeID, in.raw, ob)
			resumeOnce.Do(func() { close(resumeSeen) })
		case "fleet_msg":
			if s.deliverInbound(ctx, edge, edge.Alias, in.raw, write, false) == inCloseRevoked {
				return dialOutcome{}
			}
		case "fleet_caps":
			// caps-design §2: a mid-session caps change from the serve peer. Stored
			// latest-wins, rate-capped, no ack, never journaled.
			s.handleFleetCaps(edge, in.raw)
		case "error":
			if outcome, dead := s.dialErrorOutcome(edge, in.raw); dead {
				return outcome
			}
		default:
			// Unknown t → ignore (forward-compat, SPEC §3).
		}
	}
}

// applyWelcome enforces the welcome_fleet pin as a session PRECONDITION (review
// SF2): a valid non-empty key_fp is required; the first one is trust-on-first-use
// pinned (and the claimed box name recorded only then); a later DIFFERENT fp is a
// mismatch that flags and REJECTS (never overwrites the pin). Returns false to
// terminate the session.
func (s *Server) applyWelcome(edge FleetEdge, raw []byte) (ok bool, capsAware bool) {
	if s.fleetStore == nil {
		return false, false
	}
	var w struct {
		Box   string          `json:"box"`
		KeyFP string          `json:"key_fp"`
		Caps  json.RawMessage `json:"caps"`
	}
	tag := shortEdgeID(edge.EdgeID)
	if json.Unmarshal(raw, &w) != nil {
		s.fleetLog.logf("edge=%s welcome_fleet rejected: unparsable", tag)
		return false, false
	}
	if w.KeyFP == "" {
		s.fleetLog.logf("edge=%s welcome_fleet rejected: empty key_fp", tag)
		return false, false
	}
	pinned, mismatch, err := s.fleetStore.PinPeerKeyFP(edge.EdgeID, w.KeyFP)
	if err != nil {
		s.fleetLog.logf("edge=%s welcome_fleet pin error: %v", tag, err)
		return false, false
	}
	if mismatch {
		_ = s.fleetStore.FlagKeyFPMismatch(edge.EdgeID)
		s.cacheSetMismatch(edge.EdgeID) // B2/B4: persist in the attestation cache too
		s.fleetLog.logf("edge=%s welcome_fleet rejected: key_fp MISMATCH (peer presented %q != pinned)", tag, w.KeyFP)
		return false, false
	}
	if pinned {
		s.fleetLog.logf("edge=%s welcome_fleet pinned peer key_fp", tag)
		if err := s.fleetStore.SetPeerBoxName(edge.EdgeID, w.Box); err != nil {
			s.fleetLog.logf("edge=%s welcome_fleet set peer_box_name failed: %v", tag, err)
		}
	}
	// Keep the attestation cache's pin current (B2): first-use pin, or a re-affirmed
	// match. w.KeyFP is the just-validated session key.
	s.cacheSetPin(edge.EdgeID, w.KeyFP)
	// caps-design §2: a welcome carrying a caps object is the peer's box-attested
	// manifest AND the caps-awareness signal. Store it latest-wins (clamped + BOUND to
	// the welcome's just-pinned key_fp — review B4).
	if len(w.Caps) > 0 {
		capsAware = true
		s.storePeerCaps(edge.EdgeID, w.KeyFP, w.Caps)
	}
	return true, capsAware
}

// dialErrorOutcome maps a peer error frame to a session outcome. unauthorized /
// revoked mean the peer removed the edge on their side → the dialer stops and marks
// the edge dead. Any other code is transient.
func (s *Server) dialErrorOutcome(edge FleetEdge, raw []byte) (dialOutcome, bool) {
	var e struct {
		Code string `json:"code"`
	}
	_ = json.Unmarshal(raw, &e)
	s.fleetLog.logf("edge=%s dir=in kind=error code=%q", shortEdgeID(edge.EdgeID), e.Code)
	switch e.Code {
	case "unauthorized", "revoked":
		return dialOutcome{dead: true, reason: e.Code}, true
	}
	return dialOutcome{}, false
}
