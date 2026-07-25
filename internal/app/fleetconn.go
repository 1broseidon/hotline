package app

import (
	"context"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

// This file wires serve-side fleet rooms into the box's serving runtime (A2A v2
// §3.1), mirroring the operator room manager (connector.go runRoomManager /
// roomLoop) but with the MINIMAL fleet session handler and no --rotate-all hook.
// Fleet rooms are a disjoint served set (FleetStore.ServedFleetRooms), so a fleet
// room flapping only spins its own goroutine's backoff and never perturbs the
// operator path.

// runFleetRoomManager serves every non-tombstoned serve-direction fleet room
// concurrently, diffing the registry each poll like runRoomManager. It is a
// no-op when the fleet store is absent (bare-Server tests) so it never touches
// the operator serving path.
func (s *Server) runFleetRoomManager(ctx context.Context) error {
	if s.fleetStore == nil {
		<-ctx.Done()
		return nil
	}
	loops := map[string]*roomLoopHandle{}
	// roomEdge maps a served room id → its edge id, so the reap path can address the
	// live serve session (registered by edge id) to send a revoked (review B5b).
	roomEdge := map[string]string{}

	reap := func(id string) {
		h := loops[id]
		h.cancel()
		<-h.done
		delete(loops, id)
		delete(roomEdge, id)
	}

	diff := func() {
		served, err := s.fleetStore.ServedFleetRooms()
		if err != nil {
			// A corrupt/invalid registry must not silently serve a malformed edge:
			// log and keep the currently-running loops as-is until it is repaired.
			s.fleetLog.logf("fleet room manager: %v", err)
			fmt.Fprintf(os.Stderr, "hotline: fleet registry unusable: %v\n", err)
			return
		}
		servedSet := make(map[string]FleetEdge, len(served))
		for _, e := range served {
			servedSet[e.Room] = e
		}
		newCount := 0
		for id, e := range servedSet {
			if _, running := loops[id]; running {
				continue
			}
			lctx, cancel := context.WithCancel(ctx)
			h := &roomLoopHandle{cancel: cancel, done: make(chan struct{})}
			loops[id] = h
			roomEdge[id] = e.EdgeID
			delay := time.Duration(newCount) * dialStagger
			newCount++
			go func(e FleetEdge, h *roomLoopHandle, delay time.Duration) {
				defer close(h.done)
				s.fleetRoomLoop(lctx, e, delay)
			}(e, h, delay)
		}
		for id := range loops {
			if _, ok := servedSet[id]; !ok {
				// This serve edge left the served set (a `fleet rm` tombstoned it):
				// send an authenticated revoked to the live session BEFORE terminating
				// it (review B5b), so the dialing peer marks the edge dead promptly
				// instead of retrying a now-empty room. loops is keyed by ROOM id, so
				// resolve the edge id to address the session (registered by edge id).
				if edgeID := roomEdge[id]; edgeID != "" {
					if w, live := s.fleetSessionWriter(edgeID); live {
						_ = w(errorFrame("revoked", ""))
						s.fleetLog.logf("edge=%s serve session revoked: edge removed", shortEdgeID(edgeID))
					}
				}
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

// fleetRoomLoop serves a single fleet room until its ctx is cancelled (the
// manager reaping a removed edge, or box shutdown). It mirrors roomLoop:
// best-effort core registration (reusing ensureRegistered — the synthesized
// RoomRecord carries the secret), dial the /c leg, serve the fleet session,
// per-room backoff.
func (s *Server) fleetRoomLoop(ctx context.Context, edge FleetEdge, startDelay time.Duration) {
	if startDelay > 0 {
		select {
		case <-ctx.Done():
			return
		case <-time.After(startDelay):
		}
	}
	room := edge.roomRecordFor()
	tag := shortEdgeID(edge.EdgeID)
	backoff := defaultRelayBackoffMin
	for ctx.Err() == nil {
		s.ensureFleetRegistered(ctx, room)
		endpoint := strings.TrimRight(room.URL, "/") + "/r/" + room.ID + "/c"
		s.fleetLog.logf("edge=%s /c dialing %s", tag, endpoint)
		conn, resp, err := s.dial(ctx, endpoint, nil)
		if resp != nil && resp.Body != nil {
			_ = resp.Body.Close()
		}
		if err == nil {
			connected := time.Now()
			s.fleetLog.logf("edge=%s /c connected", tag)
			s.serveFleetConnForRoom(ctx, conn, edge)
			_ = conn.Close()
			held := time.Since(connected)
			if ctx.Err() != nil {
				s.fleetLog.logf("edge=%s /c session ended after %s (ctx cancelled)", tag, held.Round(time.Millisecond))
				return
			}
			s.fleetLog.logf("edge=%s /c session ended after %s (stable=%t)", tag, held.Round(time.Millisecond), held >= defaultRelayStableAfter)
			if held >= defaultRelayStableAfter {
				backoff = defaultRelayBackoffMin
			}
		} else {
			fmt.Fprintf(os.Stderr, "hotline: fleet edge=%s dial failed: %v\n", tag, err)
			s.fleetLog.logf("edge=%s /c dial failed: %v", tag, err)
		}
		delay := jitterBackoff(backoff, defaultRelayBackoffMax)
		s.fleetLog.logf("edge=%s /c reconnect in %s (backoff=%s)", tag, delay.Round(time.Millisecond), backoff)
		select {
		case <-ctx.Done():
			return
		case <-time.After(delay):
		}
		backoff = nextBackoff(backoff, defaultRelayBackoffMax)
	}
}

// serveFleetConnForRoom runs one fleet relay session and returns when it ends or
// the edge's ctx is cancelled (mirrors serveV2ConnForRoom).
func (s *Server) serveFleetConnForRoom(ctx context.Context, conn *websocket.Conn, edge FleetEdge) {
	serveDone := make(chan struct{})
	go func() {
		s.serveFleetConn(ctx, conn, edge)
		close(serveDone)
	}()
	select {
	case <-ctx.Done():
		_ = conn.Close()
		<-serveDone
	case <-serveDone:
	}
}

// serveFleetConn wraps the fleet socket in the mandatory e1 envelope codec (a
// fleet room is ALWAYS an envelope room — F1), drives readSessionInputs (the
// shared reader that drops any non-e1 frame, so a plaintext hello never reaches
// the handler), then reads the hello and runs the fleet session.
func (s *Server) serveFleetConn(ctx context.Context, conn *websocket.Conn, edge FleetEdge) {
	room := edge.roomRecordFor()
	codec, err := newEnvelopeCodec(room)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: fleet envelope codec init failed edge=%s: %v\n", shortEdgeID(edge.EdgeID), err)
		s.fleetLog.logf("edge=%s codec init failed: %v", shortEdgeID(edge.EdgeID), err)
		return
	}

	// Outer transport bound (B3): refuse any frame larger than the fleet cap
	// before the reader allocates it. gorilla closes the conn with 1009 when the
	// peer exceeds this.
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
	closeWith := func(code int, reason string) {
		writeMu.Lock()
		defer writeMu.Unlock()
		_ = conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(code, reason), time.Now().Add(writeWait))
	}

	inputs := make(chan sessionInput, 1)
	readerDone := make(chan struct{})
	readerStopped := make(chan struct{})
	go func() {
		defer close(readerStopped)
		readSessionInputs(conn, codec, inputs, readerDone, s.fleetLog, shortEdgeID(edge.EdgeID))
	}()
	defer func() {
		close(readerDone)
		_ = conn.Close()
		<-readerStopped
	}()

	hello, ok := s.readFleetHello(ctx, shortEdgeID(edge.EdgeID), inputs, write)
	if !ok {
		return
	}
	result := s.serveFleetSession(ctx, edge.EdgeID, hello, inputs, write)
	if result.closeCode != 0 {
		closeWith(result.closeCode, result.closeReason)
	}
}

// ensureFleetRegistered registers a fleet room with the core, keyed separately
// from operator rooms in the shared registered map ("fleet:"+roomID) so the two
// namespaces never collide (should-fix 4). Best-effort and inert outside core
// mode, mirroring ensureRegistered.
func (s *Server) ensureFleetRegistered(ctx context.Context, room RoomRecord) {
	if !s.coreMode || s.coreClient == nil {
		return
	}
	key := "fleet:" + room.ID
	s.regMu.Lock()
	done := s.registered[key]
	s.regMu.Unlock()
	if done {
		return
	}
	if room.Secret == "" {
		fmt.Fprintf(os.Stderr, "hotline: fleet core register skipped for room %s: no stored secret\n", room.ID)
		return
	}
	authHash, err := deriveRoomAuthHash(room.Secret)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: fleet core register auth_hash derive failed room=%s: %v\n", room.ID, err)
		return
	}
	name := room.Name
	if idName, ok := s.store.IdentityName(); ok && idName != "" {
		name = idName
	}
	rctx, cancel := withTimeout(ctx, pushTimeout)
	defer cancel()
	if err := s.coreClient.register(rctx, room.ID, name, authHash); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: fleet core register failed room=%s: %v\n", room.ID, err)
		return
	}
	s.regMu.Lock()
	s.registered[key] = true
	s.regMu.Unlock()
}
