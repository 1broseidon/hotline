package app

import (
	"context"
	"encoding/json"
	"fmt"
	"math/rand"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/gorilla/websocket"
)

const (
	defaultRelayBackoffMin  = time.Second
	defaultRelayBackoffMax  = 60 * time.Second
	defaultRelayStableAfter = 60 * time.Second
	relayRoomPollInterval   = 2 * time.Second
	writeWait               = 10 * time.Second
	// maxBadFrames is the malformed-frame abuse limit shared by the gap and
	// steady-state input loops: the 3rd bad frame closes with 4002 "bad_frame".
	maxBadFrames = 3
)

type relayDialFunc func(context.Context, string, http.Header) (*websocket.Conn, *http.Response, error)

func defaultRelayDial(ctx context.Context, url string, header http.Header) (*websocket.Conn, *http.Response, error) {
	return websocket.DefaultDialer.DialContext(ctx, url, header)
}

// roomLoopHandle is a running per-room serving goroutine (SPEC §3). The manager
// cancels ctx and awaits done to reap it when the room leaves the served set.
type roomLoopHandle struct {
	cancel context.CancelFunc
	done   chan struct{}
}

// dialStagger spaces the initial dials of rooms spawned in the same poll to
// avoid a thundering herd on box restart (SPEC §3).
const dialStagger = 250 * time.Millisecond

// runRoomManager serves every non-dead room concurrently (SPEC §3, MD2). Every
// relayRoomPollInterval it diffs store.ServedRooms() against the running
// per-room loops: a newly appeared room spawns a roomLoop (a live ADD with no
// harness recycle), a room that left the set (revoked → dead, or replaced by
// --rotate-all) has its loop cancelled and reaped. A --rotate-all — detected as
// the whole previously-served set leaving in one poll while a new room appears —
// still fires the onRoomRotate harness-recycle hook exactly once (the preserved
// soak-bug fix); adds and removals never recycle.
func (s *Server) runRoomManager(ctx context.Context) error {
	loops := map[string]*roomLoopHandle{}
	var prevServed map[string]bool

	reap := func(id string) {
		h := loops[id]
		h.cancel()
		<-h.done
		delete(loops, id)
	}

	diff := func() {
		served := s.store.ServedRooms()
		servedSet := make(map[string]RoomRecord, len(served))
		curSet := make(map[string]bool, len(served))
		for _, r := range served {
			servedSet[r.ID] = r
			curSet[r.ID] = true
		}
		// Rotation detection: the entire previous served set left in one poll and
		// a new room appeared ⇒ a --rotate-all. Fire the recycle hook once.
		if rotatedAll(prevServed, curSet) {
			for id := range curSet {
				if !prevServed[id] {
					s.onRoomRotate(id)
					break
				}
			}
		}
		prevServed = curSet
		// Spawn new rooms (staggered).
		newCount := 0
		for id, r := range servedSet {
			if _, running := loops[id]; running {
				continue
			}
			lctx, cancel := context.WithCancel(ctx)
			h := &roomLoopHandle{cancel: cancel, done: make(chan struct{})}
			loops[id] = h
			delay := time.Duration(newCount) * dialStagger
			newCount++
			go func(r RoomRecord, h *roomLoopHandle, delay time.Duration) {
				defer close(h.done)
				s.roomLoop(lctx, r, delay)
			}(r, h, delay)
		}
		// Reap rooms that left the served set.
		for id := range loops {
			if _, ok := servedSet[id]; !ok {
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

// rotatedAll reports whether the served set was wholly replaced in one poll —
// both the previous and current sets are non-empty and share no room id. This is
// the --rotate-all signature (every prior room went dead, one fresh room
// appeared). An initial bind (empty prev), a live add, and a revoke all share at
// least one id and so are not rotations.
func rotatedAll(prev, cur map[string]bool) bool {
	if len(prev) == 0 || len(cur) == 0 {
		return false
	}
	for id := range cur {
		if prev[id] {
			return false
		}
	}
	return true
}

// roomLoop serves a single room until its ctx is cancelled (SPEC §3): the old
// runConnector body minus the current_room polling. Per-room backoff, per-room
// core registration, per-room envelope codec — nothing is shared but the Server,
// so one room's relay flapping only spins this goroutine's backoff.
func (s *Server) roomLoop(ctx context.Context, room RoomRecord, startDelay time.Duration) {
	if startDelay > 0 {
		select {
		case <-ctx.Done():
			return
		case <-time.After(startDelay):
		}
	}
	tag := room.ID
	if len(tag) > 8 {
		tag = tag[:8]
	}
	backoff := defaultRelayBackoffMin
	for ctx.Err() == nil {
		s.ensureRegistered(ctx, room)
		endpoint := strings.TrimRight(room.URL, "/") + "/r/" + room.ID + "/c"
		s.connLog.logf("room=%s /c dialing %s", tag, endpoint)
		conn, resp, err := s.dial(ctx, endpoint, nil)
		if resp != nil && resp.Body != nil {
			_ = resp.Body.Close()
		}
		if err == nil {
			connected := time.Now()
			s.connLog.logf("room=%s /c connected", tag)
			s.serveV2ConnForRoom(ctx, conn, room)
			_ = conn.Close()
			held := time.Since(connected)
			if ctx.Err() != nil {
				s.connLog.logf("room=%s /c session ended after %s (ctx cancelled — room reaped/box shutdown)", tag, held.Round(time.Millisecond))
				return
			}
			s.connLog.logf("room=%s /c session ended after %s (stable=%t)", tag, held.Round(time.Millisecond), held >= defaultRelayStableAfter)
			if held >= defaultRelayStableAfter {
				backoff = defaultRelayBackoffMin
			}
		} else {
			fmt.Fprintf(os.Stderr, "hotline: room=%s dial failed: %v\n", tag, err)
			s.connLog.logf("room=%s /c dial failed: %v", tag, err)
		}
		delay := jitterBackoff(backoff, defaultRelayBackoffMax)
		s.connLog.logf("room=%s /c reconnect in %s (backoff=%s)", tag, delay.Round(time.Millisecond), backoff)
		select {
		case <-ctx.Done():
			return
		case <-time.After(delay):
		}
		backoff = nextBackoff(backoff, defaultRelayBackoffMax)
	}
}

func nextBackoff(current, max time.Duration) time.Duration {
	if current >= max || current > max/2 {
		return max
	}
	return current * 2
}

func jitterBackoff(base, max time.Duration) time.Duration {
	if base >= max {
		return max
	}
	spread := base / 2
	d := base + time.Duration(rand.Int63n(int64(spread)+1))
	if d > max {
		return max
	}
	return d
}

// ensureRegistered registers the current room with the core once per room id
// (core-v1 SPEC §2.2), on the first connect and after a new-link room hop. It is
// best-effort: a failure is logged and retried on the next reconnect. Inert when
// core mode is off. auth_hash is derived from the room's stored pairing secret;
// a room minted before core mode has no stored secret and is skipped (soak/cutover
// always uses `new-link`, which mints an envelope room with the secret).
func (s *Server) ensureRegistered(ctx context.Context, room RoomRecord) {
	if !s.coreMode || s.coreClient == nil {
		return
	}
	s.regMu.Lock()
	done := s.registered[room.ID]
	s.regMu.Unlock()
	if done {
		return
	}
	if room.Secret == "" {
		fmt.Fprintf(os.Stderr, "hotline: core register skipped for room %s: no stored secret (mint a new-link)\n", room.ID)
		return
	}
	authHash, err := deriveRoomAuthHash(room.Secret)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: core register auth_hash derive failed room=%s: %v\n", room.ID, err)
		return
	}
	// FB21: the box identity is the one true assistant name; room records can
	// carry stale pre-identity names (old link placeholders like "Pegasus"),
	// and boot-time re-registration would otherwise stamp those onto the relay
	// (= push titles reviving dead names).
	name := room.Name
	if idName, ok := s.store.IdentityName(); ok && idName != "" {
		name = idName
	}
	rctx, cancel := withTimeout(ctx, pushTimeout)
	defer cancel()
	if err := s.coreClient.register(rctx, room.ID, name, authHash); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: core register failed room=%s: %v\n", room.ID, err)
		return
	}
	fmt.Fprintf(os.Stderr, "hotline: core register ok room=%s name=%q\n", room.ID, name)
	s.regMu.Lock()
	s.registered[room.ID] = true
	s.regMu.Unlock()
}

// reregisterRoomsForRename re-registers every live room with the core after a
// rename (FB21 set_name) so the relay's per-room reg.name — used verbatim as the
// push-notification title on the core path — refreshes from the box's frozen
// first-register value to the new name. The relay's handleRegister is idempotent
// and overwrites the stored name on a same-key re-register, so re-registering is
// the entire relay-side fix; there is no dedicated name-update endpoint. It
// clears the one-shot ensureRegistered gates so each served room re-sends with
// the freshly restamped Name read back from the store. Best-effort: per-room
// failures are logged in ensureRegistered and never crash or block the rename.
// Runs off the ws handler (its own goroutine) so relay I/O never stalls the ack.
func (s *Server) reregisterRoomsForRename() {
	if !s.coreMode || s.coreClient == nil {
		return
	}
	s.regMu.Lock()
	for id := range s.registered {
		delete(s.registered, id)
	}
	s.regMu.Unlock()
	ctx := context.Background()
	for _, room := range s.store.ServedRooms() {
		s.ensureRegistered(ctx, room)
	}
}

type sessionInput struct {
	raw []byte
	err error
}

type sessionResult struct {
	restart     []byte
	closeCode   int
	closeReason string
	awaitHello  bool
}

// serveV2ConnForRoom runs one relay session for room and returns when the
// session ends or the room's ctx is cancelled (the manager reaping the room, or
// box shutdown). Room lifecycle is now the manager's job via ctx — this only has
// to unblock the blocked reader by closing the socket on ctx.Done().
func (s *Server) serveV2ConnForRoom(ctx context.Context, conn *websocket.Conn, room RoomRecord) {
	serveDone := make(chan struct{})
	go func() {
		s.serveV2Conn(ctx, conn, room)
		close(serveDone)
	}()
	select {
	case <-ctx.Done():
		_ = conn.Close()
		<-serveDone
	case <-serveDone:
	}
}

func (s *Server) serveV2Conn(ctx context.Context, conn *websocket.Conn, room RoomRecord) {
	// Envelope (core-v1 SPEC §1): for an e1 room, derive the directional content
	// keys once and wrap/unwrap every v2 frame at this socket boundary. The inner
	// v2 protocol below is untouched. A codec-derivation failure on an e1 room is
	// fatal to this connection (we must not fall back to plaintext on an E2E room).
	// codec == nil for every plaintext room ⇒ the write/read paths below are the
	// exact pre-core byte path.
	var codec *envelopeCodec
	if room.Envelope {
		c, err := newEnvelopeCodec(room)
		if err != nil {
			fmt.Fprintf(os.Stderr, "hotline: envelope codec init failed room=%s: %v\n", room.ID, err)
			return
		}
		codec = c
	}

	var writeMu sync.Mutex
	write := func(raw []byte) error {
		out := raw
		if codec != nil {
			sealed, err := codec.wrap(raw)
			if err != nil {
				return err
			}
			out = sealed
		}
		writeMu.Lock()
		defer writeMu.Unlock()
		_ = conn.SetWriteDeadline(time.Now().Add(writeWait))
		return conn.WriteMessage(websocket.TextMessage, out)
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
		readSessionInputs(conn, codec, inputs, readerDone, s.connLog, shortRoomID(room.ID))
	}()
	defer func() {
		close(readerDone)
		_ = conn.Close()
		<-readerStopped
	}()

	var pendingHello []byte
	for ctx.Err() == nil {
		hello, ok := readHello(ctx, inputs, pendingHello, write)
		pendingHello = nil
		if !ok {
			return
		}
		result := s.serveV2Session(ctx, room, hello, inputs, write)
		if len(result.restart) != 0 {
			pendingHello = result.restart
			continue
		}
		if result.awaitHello {
			continue
		}
		if result.closeCode != 0 {
			closeWith(result.closeCode, result.closeReason)
		}
		return
	}
}

func readSessionInputs(conn *websocket.Conn, codec *envelopeCodec, inputs chan<- sessionInput, done <-chan struct{}, clog *connLogger, tag string) {
	for {
		messageType, raw, err := conn.ReadMessage()
		if err != nil {
			// Capture the /c close code — the single most diagnostic signal for a
			// device-bounce flap. 1013 "frame rate exceeded" = the box flooded the
			// per-room frame bucket writing to an absent app and the relay closed the
			// SENDER (see hotline-core src/index.ts webSocketMessage: consumeFrame runs
			// BEFORE the peer-present check); 4001 "replaced" = a second /c claimed the
			// room; 1000/1006 = a normal/abnormal peer close.
			if ce, ok := err.(*websocket.CloseError); ok {
				clog.logf("room=%s /c closed code=%d reason=%q", tag, ce.Code, ce.Text)
			} else {
				clog.logf("room=%s /c read error: %v", tag, err)
			}
			select {
			case inputs <- sessionInput{err: err}:
			case <-done:
			}
			return
		}
		if messageType != websocket.TextMessage {
			continue
		}
		if codec != nil {
			// Envelope room (SPEC §1): unwrap the e1 frame to the inner v2 bytes.
			// A frame that fails to decrypt (tampered, wrong key, or a plaintext
			// frame on an e-room) is dropped — the inner v2 layer never sees it and
			// so a plaintext hello on an e-room gets no reply.
			inner, derr := codec.unwrap(raw)
			if derr != nil {
				continue
			}
			raw = inner
		}
		select {
		case inputs <- sessionInput{raw: raw}:
		case <-done:
			return
		}
	}
}

func readHello(ctx context.Context, inputs <-chan sessionInput, pending []byte, write func([]byte) error) (helloFrame, bool) {
	for {
		var in sessionInput
		if len(pending) != 0 {
			in.raw = pending
			pending = nil
		} else {
			select {
			case <-ctx.Done():
				return helloFrame{}, false
			case in = <-inputs:
			}
		}
		if in.err != nil {
			return helloFrame{}, false
		}
		t, exact := exactType(in.raw)
		if !exact {
			continue
		}
		if t != "hello" {
			_ = write(errorFrame("bad_frame", "first frame must be hello"))
			return helloFrame{}, false
		}
		var hello helloFrame
		if json.Unmarshal(in.raw, &hello) != nil || hello.V != ProtocolVersion || !deviceLookupRE.MatchString(hello.DeviceID) || hello.Secret == "" || !validDecimal(hello.ResumeFrom) {
			_ = write(errorFrame("bad_frame", "invalid hello"))
			return helloFrame{}, false
		}
		return hello, true
	}
}

func (s *Server) serveV2Session(ctx context.Context, room RoomRecord, hello helloFrame, inputs <-chan sessionInput, write func([]byte) error) sessionResult {
	// A1 capability gate: record for this session's lifetime whether the device can
	// render fleet_state, so the post-drain snapshot + change broadcast emit it ONLY to
	// a device that advertised support (no shipped client does → not emitted at all).
	if helloAdvertisesFleetState(hello.Caps) {
		defer s.markFleetStateCap(hello.DeviceID)()
	}
	result, linked, err := s.store.VerifyAndLink(room.ID, hello.DeviceID, hello.Secret)
	if err != nil {
		_ = write(errorFrame("bad_frame", "state update failed"))
		return sessionResult{}
	}
	switch result {
	case VerifyUnauthorized:
		_ = write(errorFrame("unauthorized", ""))
		return sessionResult{closeCode: 4003, closeReason: "unauthorized"}
	case VerifyRevoked:
		_ = write(errorFrame("revoked", ""))
		return sessionResult{closeCode: 4003, closeReason: "revoked"}
	}
	// Ensure the device's mailbox exists on EVERY successful attach, not only on
	// the first link (B1). provision() is idempotent — a no-op when the mailbox is
	// already present — so this re-seeds nothing for a live device but recreates a
	// missing mailbox for a device that is active in the relay store yet lost its
	// mailbox record (e.g. a mailbox file reset/desynced from the relay-state
	// across a box restart). Without this, a reconnecting device (VerifyActive,
	// linked=false) skipped provisioning, hit "mailbox unavailable" below, and
	// could never self-heal — it flapped with zero catch-up on every re-attach.
	if err := s.provisionMailbox(hello.DeviceID); err != nil {
		_ = write(errorFrame("bad_frame", "mailbox provisioning failed"))
		return sessionResult{}
	}
	// Reconcile this device against the authoritative outbox before it participates
	// in W: record a Hole for every durable seq it is missing above its contiguous
	// head (crash gap, reactivation gap, unbound-window gap), THEN backfill re-
	// delivers what it can so its CHead — and thus W — can advance. A frame that
	// can't be delivered yet (mailbox still Full) stays a recorded Hole and holds W
	// below it. Idempotent for a live/caught-up device (nothing above CHead).
	s.reconcileDevice(hello.DeviceID)
	if linked {
		if s.onLinked != nil {
			s.onLinked(hello.DeviceID)
		} else {
			fmt.Fprintln(os.Stderr, "hotline: device linked")
		}
	}
	if hello.Push != nil && s.acceptPushToken(hello.Push.Token) && (hello.Push.Platform == "ios" || hello.Push.Platform == "android") {
		_ = s.store.SetPush(hello.DeviceID, hello.Push.Token, hello.Push.Platform)
		// Core mode: forward the token to the core registry (SPEC §3.3, full
		// replace). The box keeps its local copy too (self-host mode unchanged).
		if s.coreMode && s.coreClient != nil {
			s.forwardDeviceToken(room.ID, hello.DeviceID, hello.Push.Platform, hello.Push.Token)
		}
		// Gateway mode: a token with no credential yet needs registration. This
		// no-ops when a registration is already in flight or a key_id exists.
		if s.gatewayEnabled() {
			if _, keyID, _, ok := s.store.ActivePushTarget(hello.DeviceID); ok && keyID == "" {
				s.kickRegister(hello.DeviceID, hello.Push.Token)
			}
		}
	}
	floor, head, snapshot, sub, err := s.mailbox.stateAndSubscribe(hello.DeviceID)
	if err != nil {
		_ = write(errorFrame("bad_frame", "mailbox unavailable"))
		return sessionResult{}
	}
	defer s.mailbox.unsubscribe(hello.DeviceID, sub)

	if decimalCmp(hello.ResumeFrom, head) > 0 {
		_ = write(errorFrame("cursor_ahead", ""))
		return sessionResult{awaitHello: true}
	}
	if err := write(welcomeFrame(room, hello.DeviceID, floor, head, s.currentAgentInfo())); err != nil {
		return sessionResult{}
	}
	cursor := hello.ResumeFrom
	// A device parked in full resync MUST re-adopt the current floor and re-drain
	// before it can rejoin W (BLOCKER 2). markResyncLocked reset its mailbox (Floor=
	// Head, Full=true), so drive the mailbox-gap adoption even when resume_from ==
	// floor — the equality case a non-resync device legitimately skips. Without this,
	// a resync device reconnecting exactly at floor would never clear Full, so
	// markParticipating's guard would keep it excluded forever (a liveness gap).
	resync := s.mailbox.deviceResync(hello.DeviceID)
	if decimalCmp(cursor, floor) < 0 || (resync && decimalCmp(cursor, floor) == 0) {
		if err := write(mailboxGapFrame(floor)); err != nil {
			return sessionResult{}
		}
		result, proceed := waitGapAck(ctx, inputs, write, s, sub, hello.DeviceID, floor)
		if !proceed {
			return result
		}
		cursor = floor
		// Atomically clear Full WITH gap re-derivation (BLOCKER 1). The gap-ack
		// deliberately left Full set (ack skips clearing it for a resync device); here
		// we compute the undelivered durable range from the outbox and, under m.mu in
		// one critical section, record those holes THEN clear Full — so no concurrent
		// emit-fold can leap the parked gap in between. Only after this does reconcile/
		// backfill run to deliver what it can; anything still missing is a recorded Hole
		// that holds W below it.
		s.clearResyncFull(hello.DeviceID)
		// Full is now cleared with every gap recorded: re-run the reconcile so holes
		// that couldn't be filled while the mailbox was Full are delivered now, before
		// the device is admitted to W.
		s.reconcileDevice(hello.DeviceID)
	}
	// The device has completed reconcile/backfill and adopted the floor: it now
	// participates in W (its CHead honestly reflects delivery; any residual gap is a
	// recorded Hole that correctly holds W below it). markParticipating clears Resync
	// ONLY if the device is genuinely caught up (floor re-adopted / drained / no
	// outstanding holes) — otherwise a not-yet-adopted resync device stays excluded.
	s.mailbox.markParticipating(hello.DeviceID, sub, s.durableHead())
	return s.serveSessionStream(ctx, room, hello.DeviceID, sub, itemsAfter(snapshot, cursor, head), inputs, write)
}

// waitGapAck drains input during the mailbox-gap phase until the device acks at
// the floor. It returns a sessionResult and whether to proceed to the stream:
// (result, true) means the gap ack succeeded and the caller should stream from
// the floor; (result, false) means the caller should return result directly (a
// restart hello, a disconnect, or a 4002 abuse close). It applies the same
// 3-bad-frame → close(4002,"bad_frame") accounting as steady state.
func waitGapAck(ctx context.Context, inputs <-chan sessionInput, write func([]byte) error, s *Server, sub *mailboxSubscriber, deviceID, floor string) (sessionResult, bool) {
	badFrames := 0
	// recordBad reports the bad frame and returns true once the abuse limit is
	// reached, matching serveSessionStream's steady-state accounting.
	recordBad := func(detail string) bool {
		_ = write(errorFrame("bad_frame", detail))
		badFrames++
		return badFrames >= maxBadFrames
	}
	for {
		var in sessionInput
		select {
		case <-ctx.Done():
			return sessionResult{}, false
		case in = <-inputs:
		}
		if in.err != nil {
			return sessionResult{}, false
		}
		t, ok := exactType(in.raw)
		if !ok {
			continue
		}
		if t == "hello" {
			return sessionResult{restart: in.raw}, false
		}
		if t == "ping" {
			n, ok := parsePing(in.raw)
			if !ok {
				if recordBad("ping.n must be a non-negative integer") {
					return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, false
				}
				continue
			}
			s.mailbox.touchPresence(sub)
			_ = write(pongFrame(n))
			continue
		}
		// Presence is accepted during the gap phase without disturbing the gap
		// ack/drain: it only mutates subscriber presence and never advances the
		// cursor. This keeps the app free to send presence:foreground on a fresh
		// socket even if it lands before the gap ack completes.
		if t == "presence" {
			if !s.applyPresence(sub, in.raw) {
				if recordBad("presence.state must be foreground or background") {
					return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, false
				}
			}
			continue
		}
		// ActivityKit registration is authenticated by the post-hello session and
		// is accepted during gap recovery without disturbing the required ack.
		if t == "live_activity_token" {
			if !s.applyLiveActivityToken(deviceID, in.raw) {
				if recordBad("invalid live_activity_token") {
					return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, false
				}
			}
			continue
		}
		// Read-state is INERT during gap recovery (SF1), regardless of the
		// HOTLINE_READ_SYNC gate: the device has not yet adopted the floor, so any
		// read it sends refers to a pre-gap view and must not advance anything —
		// and, like an unknown type, it must never count toward the 4002 abuse
		// limit (a client that always emits reads would otherwise trip it after 3).
		// The device reconverges via the post-drain read snapshot.
		if t == "read" {
			continue
		}
		if t != "mailbox_ack" {
			if recordBad("mailbox_gap requires an ack at floor") {
				return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, false
			}
			continue
		}
		var a struct {
			M string `json:"m"`
		}
		if json.Unmarshal(in.raw, &a) != nil || a.M != floor {
			if recordBad("mailbox_ack.m must equal floor") {
				return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, false
			}
			continue
		}
		return sessionResult{}, s.mailbox.ack(deviceID, a.M) == nil
	}
}

func (s *Server) serveSessionStream(ctx context.Context, room RoomRecord, deviceID string, sub *mailboxSubscriber, drain []MailboxItem, inputs <-chan sessionInput, write func([]byte) error) sessionResult {
	roomTick := time.NewTicker(relayRoomPollInterval)
	defer roomTick.Stop()
	badFrames := 0
	snapshotSent := false
	handleInput := func(in sessionInput) (sessionResult, bool) {
		if in.err != nil {
			return sessionResult{}, true
		}
		if typ, exact := exactType(in.raw); exact && typ == "hello" {
			return sessionResult{restart: in.raw}, true
		}
		bad, fatal := s.handleSessionInput(ctx, deviceID, sub, in.raw, write)
		if bad {
			badFrames++
		}
		if fatal {
			return sessionResult{}, true
		}
		if badFrames >= maxBadFrames {
			return sessionResult{closeCode: 4002, closeReason: "bad_frame"}, true
		}
		return sessionResult{}, false
	}
	// checkRoom closes the session when this device stops being active-and-bound
	// here (a `relay revoke` bans the device and deads its room within one 2s
	// poll → 4003 "revoked"). The current_room clause is gone: room lifecycle is
	// the room manager's job (it cancels this loop's ctx when the room leaves the
	// served set), so a session only self-closes on the device-binding change.
	checkRoom := func() (sessionResult, bool) {
		device, deviceOK := s.store.Device(deviceID)
		if !deviceOK || device.State != DeviceActive || device.Room != room.ID {
			_ = write(errorFrame("revoked", ""))
			return sessionResult{closeCode: 4003, closeReason: "revoked"}, true
		}
		return sessionResult{}, false
	}

	for {
		// Prefer already-read app input so a queued hello cancels the old drain
		// before another item is emitted.
		select {
		case in := <-inputs:
			if result, stop := handleInput(in); stop {
				return result
			}
			continue
		default:
		}
		if len(drain) != 0 {
			if err := s.writeItemWithBlobs(write, drain[0]); err != nil {
				return sessionResult{}
			}
			drain = drain[1:]
			continue
		}
		// The device is now caught up (welcome + gap + drain all delivered):
		// hand it the current agent_state snapshot once, over the transient
		// path (SPEC §1.2 "full snapshot on device subscribe, after drain").
		if !snapshotSent {
			snapshotSent = true
			s.snapshotAgentStateTo(deviceID)
			// The model catalog rides the same moment (model catalog amendment
			// 2026-07-20): it is per-device push-once, so a device that attaches
			// long after the harness reported still gets the list without asking.
			// A box with no catalog sends nothing and the app keeps its curated
			// fallback — the pre-amendment behavior, unchanged.
			s.snapshotAgentCatalogTo(deviceID)
			// Read-state converges here too (§4.1): an offline device that missed
			// intermediate read transients gets the current shared cursor once,
			// after its mailbox drained — so the snapshot never races ahead of the
			// messages it refers to.
			s.snapshotReadTo(deviceID)
			// Fleet awareness converges here too (a2a-design-v2 §6, Lane L4): the
			// full fleet snapshot is pushed once, post-drain, exactly like the
			// agent_state snapshot. A box with no fleet store sends nothing.
			s.snapshotFleetStateTo(deviceID)
		}
		select {
		case <-ctx.Done():
			return sessionResult{}
		case <-sub.overflow:
			return sessionResult{}
		case code := <-sub.controls:
			if write(errorFrame(code, "")) != nil {
				return sessionResult{}
			}
		case <-sub.readmit:
			// A resync device's CHead advanced via a fan-fold (BLOCKER 2): re-attempt
			// admission now instead of waiting for a reconnect. durableHead() takes the
			// OUTBOX lock and markParticipating then takes m.mu — the correct outbox→m.mu
			// order (this runs off the deliveryMu/m.mu path). markParticipating is an
			// idempotent no-op when the device is still genuinely behind (Full / holes /
			// CHead<durableHead), so a spurious wake costs nothing.
			s.mailbox.markParticipating(deviceID, sub, s.durableHead())
		case item := <-sub.items:
			if err := s.writeItemWithBlobs(write, item); err != nil {
				return sessionResult{}
			}
		case transient := <-sub.transients:
			// Ordering guarantee (§4 / B2): a transient must never overtake a
			// mailbox_item already queued for this device. A read.j in particular
			// must reach the wire only AFTER every mailbox_item.j' with j' <= j —
			// otherwise a sibling could see read.j before the item it refers to
			// (docs/app-channel.md). The read fan is published only after item j is
			// in sub.items (the read watermark only advances once item j is delivered to every device), so flushing every ready item
			// before this transient makes the wire guarantee hold both post-drain
			// and in steady state. (Other transients — typing/agent_state — are
			// floor-class and equally fine to trail the items they accompany.)
			if err := s.flushReadyItems(write, sub); err != nil {
				return sessionResult{}
			}
			if err := write(transient); err != nil {
				return sessionResult{}
			}
		case in := <-inputs:
			if result, stop := handleInput(in); stop {
				return result
			}
		case <-roomTick.C:
			if result, stop := checkRoom(); stop {
				return result
			}
		}
	}
}

// parsePing validates a ping frame shared by the gap and steady-state loops. It
// requires the n field to be PRESENT and non-negative: a missing n (which would
// otherwise decode to 0) or a negative n is a bad frame. It returns the echo
// value for the pong and whether the ping is valid. An invalid ping must not be
// ponged and must not refresh the foreground presence lease.
func parsePing(raw []byte) (int64, bool) {
	var p struct {
		N *int64 `json:"n"`
	}
	if json.Unmarshal(raw, &p) != nil || p.N == nil || *p.N < 0 {
		return 0, false
	}
	return *p.N, true
}

// applyPresence records a presence control frame against the subscriber. It
// returns false for a malformed frame (an unknown/missing state), which the
// caller reports as bad_frame and counts toward abuse handling. Unknown extra
// fields are ignored. The frame is last-write-wins and idempotent per
// subscriber.
func (s *Server) applyPresence(sub *mailboxSubscriber, raw []byte) bool {
	var p struct {
		State string `json:"state"`
	}
	if json.Unmarshal(raw, &p) != nil || (p.State != "foreground" && p.State != "background") {
		return false
	}
	s.mailbox.setPresence(sub, p.State == "foreground")
	return true
}

func (s *Server) handleSessionInput(ctx context.Context, deviceID string, sub *mailboxSubscriber, raw []byte, write func([]byte) error) (bad, fatal bool) {
	t, ok := exactType(raw)
	if !ok {
		return false, false
	}
	switch t {
	case "live_activity_token":
		// Direct ActivityKit registration is a post-hello control. Device and room
		// identity come only from this authenticated session; the frame carries no
		// client-asserted identity. Unknown/finished jobs are valid no-ops.
		if !s.applyLiveActivityToken(deviceID, raw) {
			_ = write(errorFrame("bad_frame", "invalid live_activity_token"))
			return true, false
		}
	case "presence":
		// A post-hello control frame reporting foreground/background. Identity is
		// the authenticated session's subscriber (no device_id). It has no
		// response, no cursor, no ack, no persistence, and never becomes a
		// mailbox item; it never enters content/transcript parsing.
		if !s.applyPresence(sub, raw) {
			_ = write(errorFrame("bad_frame", "presence.state must be foreground or background"))
			return true, false
		}
	case "typing":
		// Ephemeral hold hint (typing-signal design §2), on the same fire-and-forget
		// rail as presence/read. No response, no ack, no persistence, never a mailbox
		// item, never content, never a wake, never delivered to the harness — it only
		// feeds the per-device typing gate. A malformed/missing state fails OPEN
		// (state=false): a garbled frame must never penalize the connection or wedge
		// the gate. Identity is the authenticated session's device, same as presence.
		var f struct {
			State bool `json:"state"`
		}
		_ = json.Unmarshal(raw, &f)
		s.typing.set(deviceID, f.State)
		// Re-evaluate any pending burst: a state:false may release the hold now.
		s.inbound.pokeTyping(ctx)
	case "mailbox_ack":
		var a struct {
			M string `json:"m"`
		}
		if json.Unmarshal(raw, &a) != nil || s.mailbox.ack(deviceID, a.M) != nil {
			_ = write(errorFrame("bad_frame", "mailbox_ack.m must be a decimal string"))
			return true, false
		}
	case "read":
		// Read-state sync (§4). When disabled the frame is inert — treated exactly
		// like an unknown type (ignored, not a bad_frame), so a client that always
		// sends it against a soak-gated box is never penalized.
		if !s.readSync {
			return false, false
		}
		var r struct {
			J string `json:"j"`
		}
		// The cursor is GLOBAL journal-seq space, bounded to W — the hole-free
		// watermark W = min over active devices of each device's highest CONTIGUOUS
		// durable-delivered seq — NOT the raw outbox cursor. Bounding to W closes:
		// (blocker #1) a read is never accepted while item j is missing from ANY
		// active device, including behind an interior hole that a monotone-max scalar
		// would have jumped; (trailing) a transient seq reserved ABOVE the durable
		// head is never a valid target (W only counts durable, delivered frames).
		// Interior transient gaps (below W) are snapped down to a durable seq just
		// after, ring-independently.
		if json.Unmarshal(raw, &r) != nil || !validDecimal(r.J) || decimalCmp(r.J, journalString(s.durableWatermark())) > 0 {
			_ = write(errorFrame("bad_frame", "read.j must be a decimal journal seq within the durable journal cursor"))
			return true, false
		}
		// Interior transient gap (blocker #2): read.j may land on a transient seq
		// reserved BETWEEN two durable seqs (durable 1, transient 2, durable 3 —
		// read.j=2 is below W=3 yet not durable). Snap DOWN to the highest durable
		// seq <= read.j so a transient seq is NEVER persisted as the read cursor. The
		// oracle is RING-INDEPENDENT and authoritative: it consults the append-only
		// outbox.jsonl when the target predates the hot ring window, so a transient
		// seq evicted from the ring can no longer slip through and persist verbatim.
		// ok=false means no durable seq <= read.j exists at all (below the very first
		// durable frame): there is nothing durable to mark read, so the frame is
		// dropped rather than persisted as a possibly-transient value.
		seq, err := strconv.ParseUint(r.J, 10, 64)
		if err != nil {
			return true, false
		}
		durable, ok := s.outbox.highestDurableSeqAtMost(seq)
		if !ok {
			// No durable frame ≤ read.j is recorded anywhere. W is itself a durable
			// seq (a contiguous head only folds durable frames) and read.j is bounded
			// to W above, so read.j == W here means read.j is durable — accept W.
			// Otherwise read.j sits below the first recorded durable frame with nothing
			// durable to snap to (a possibly-transient value): drop it rather than
			// persist it verbatim (blocker #2).
			if w := s.durableWatermark(); w > 0 && seq >= w {
				s.setRead(w)
			}
			return false, false
		}
		s.setRead(durable)
	case "device_send":
		// FB21 §4 rename control: the app cannot emit a top-level websocket frame
		// (its connection primitive is fenced), so a rename rides as a NORMAL
		// device_send whose text payload is the serialized `{"t":"set_name",...}`
		// line — the same text-payload mechanism as `/el` element-action taps. It
		// is consumed SILENTLY here (BEFORE handleDeviceSend): never delivered to
		// the harness, never echoed to the transcript, no sent frame. Broadcasting
		// from here (not inside handleDeviceSend) preserves the asMu→deliveryMu
		// lock order — handleDeviceSend holds deliveryMu, and the broadcast path
		// re-enters it.
		if name, ok := setNameFromDeviceSend(raw); ok {
			// Any active paired device may rename (single-operator). Validate
			// (trim, reject empty/>64); invalid rides are dropped silently (this
			// path has no error convention). Skip the no-op rename so a retried
			// control frame is idempotent and never re-broadcasts.
			if name != "" && utf8.RuneCountInString(name) <= 64 {
				if cur, _ := s.store.IdentityName(); cur != name {
					if err := s.store.SetIdentityName(name); err == nil {
						// Refresh the direct/self-host push title (push.go reads
						// getBotName), which was otherwise frozen at construction.
						s.setBotName(name)
						s.broadcastAgentStateNow()
						// Re-register every live room with the relay so its
						// per-room reg.name (the push title on the core path) picks
						// up the restamped name. handleRegister is idempotent and
						// overwrites the stored name on same-key re-register, so
						// this is the whole relay-side fix. Async + best-effort: it
						// does relay network I/O and must not stall the rename ack,
						// and it stays off this ws handler to respect the
						// asMu→deliveryMu lock order noted above.
						go s.reregisterRoomsForRename()
					}
				}
			}
			return false, false
		}
		// FB23 push-preview control: a device sets its OWN push privacy via a
		// `{"t":"set_push_preview","clear":<bool>}` text payload riding the same
		// mechanism as set_name. Consumed SILENTLY here (never reaches the harness,
		// no transcript echo). Malformed clear is ignored but still consumed.
		if clear, valid, isControl := setPushPreviewFromDeviceSend(raw); isControl {
			if valid {
				_ = s.store.SetDevicePushPreview(deviceID, clear)
			}
			return false, false
		}
		// FB44 successful-job notification preference: the adjacent nested text
		// control is likewise device-local and silently consumed. A malformed or
		// missing enabled value leaves the prior preference unchanged.
		if enabled, valid, isControl := setJobCompletionPushFromDeviceSend(raw); isControl {
			if valid {
				if err := s.store.SetDeviceJobCompletionPush(deviceID, enabled); err != nil {
					fmt.Fprintf(os.Stderr, "hotline: job completion push preference persist failed device=%s: %v\n", deviceID, err)
				}
			}
			return false, false
		}
		// SDK-settings control (sdk-config.json): a claude-sdk box's model/effort
		// knobs set from the app's bot settings panel. Consumed silently like the
		// controls above — never an error frame on this path; every outcome
		// (including refusal on a non-sdk box) rides the transient per-device
		// sdk_config_result instead. Same handler-context transient emit as
		// snapshotAgentStateTo — no new lock ordering.
		if req, isControl := setSDKConfigFromDeviceSend(raw); isControl {
			s.handleSetSDKConfig(deviceID, req)
			return false, false
		}
		var f deviceSendFrame
		if json.Unmarshal(raw, &f) != nil || s.handleDeviceSend(ctx, deviceID, f) != nil {
			_ = write(errorFrame("bad_frame", "invalid device_send"))
			return true, false
		}
	case "push_challenge":
		// The app relays the {challenge_id, nonce} carried by the gateway's silent
		// registration push so a pending register() can complete. It has no
		// response and never becomes a mailbox item; ignored outside gateway mode.
		var c struct {
			ChallengeID string `json:"challenge_id"`
			Nonce       string `json:"nonce"`
		}
		if json.Unmarshal(raw, &c) != nil || c.Nonce == "" {
			_ = write(errorFrame("bad_frame", "invalid push_challenge"))
			return true, false
		}
		if s.pushSigner != nil {
			s.pushSigner.deliverChallenge(deviceID, challengeRelay{challengeID: c.ChallengeID, nonce: c.Nonce})
		}
	case "ping":
		n, ok := parsePing(raw)
		if !ok {
			_ = write(errorFrame("bad_frame", "ping.n must be a non-negative integer"))
			return true, false
		}
		// The existing app ping IS the liveness signal: a valid ping refreshes
		// the subscriber's foreground lease (unless it is explicitly
		// background-latched — an ack/ping never reactivates a backgrounded app).
		s.mailbox.touchPresence(sub)
		_ = write(pongFrame(n))
	case "blob_begin":
		var b struct {
			Xfer   string `json:"xfer"`
			Mime   string `json:"mime"`
			Size   int64  `json:"size"`
			Chunks int    `json:"chunks"`
		}
		if json.Unmarshal(raw, &b) != nil || s.blobs.begin(b.Xfer, b.Mime, b.Size, b.Chunks) != nil {
			_ = write(errorFrame("bad_frame", "invalid blob_begin"))
			return true, false
		}
	case "blob_chunk":
		var b struct {
			Xfer string `json:"xfer"`
			I    int    `json:"i"`
			Data string `json:"data"`
		}
		if json.Unmarshal(raw, &b) != nil || s.blobs.chunk(b.Xfer, b.I, b.Data) != nil {
			_ = write(errorFrame("bad_frame", "invalid blob_chunk"))
			return true, false
		}
	case "blob_end":
		var b struct {
			Xfer string `json:"xfer"`
		}
		if json.Unmarshal(raw, &b) != nil {
			return true, false
		}
		if _, err := s.blobs.end(b.Xfer); err != nil {
			_ = write(errorFrame("bad_frame", err.Error()))
			return true, false
		}
	case "blob_req":
		var b struct {
			Xfer string `json:"xfer"`
		}
		if json.Unmarshal(raw, &b) != nil {
			return true, false
		}
		frames, err := s.blobs.frames(b.Xfer)
		if err != nil {
			_ = write(errorFrame("bad_frame", "unknown blob transfer"))
			return true, false
		}
		for _, frame := range frames {
			if write(frame) != nil {
				return false, true
			}
		}
	default:
		// Unknown types are ignored for forward compatibility.
	}
	return false, false
}

// flushReadyItems drains and writes every mailbox_item currently ready on the
// subscription, non-blocking, before the caller writes a pending transient. This
// is the B2 ordering guarantee: because the read fan is published only once its
// item is already on sub.items (the read watermark advances only once item j is delivered to every device),
// flushing ready items here means a read.j transient can never reach the wire
// ahead of a mailbox_item.j' it refers to. It returns a non-nil error only when a
// write fails (the session should end); an empty channel is success.
func (s *Server) flushReadyItems(write func([]byte) error, sub *mailboxSubscriber) error {
	for {
		select {
		case item := <-sub.items:
			if err := s.writeItemWithBlobs(write, item); err != nil {
				return err
			}
		default:
			return nil
		}
	}
}

func (s *Server) writeItemWithBlobs(write func([]byte) error, item MailboxItem) error {
	var walk any
	_ = json.Unmarshal(item.Payload, &walk)
	seen := map[string]bool{}
	var xfers []string
	var visit func(any)
	visit = func(v any) {
		switch x := v.(type) {
		case map[string]any:
			if id, ok := x["xfer"].(string); ok && !seen[id] {
				seen[id] = true
				xfers = append(xfers, id)
			}
			for _, child := range x {
				visit(child)
			}
		case []any:
			for _, child := range x {
				visit(child)
			}
		}
	}
	visit(walk)
	for _, xfer := range xfers {
		frames, err := s.blobs.frames(xfer)
		if err != nil {
			continue
		}
		for _, frame := range frames {
			if err := write(frame); err != nil {
				return err
			}
		}
	}
	return write(mustMarshal(item))
}
