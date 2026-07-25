package app

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/catchup"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// coalHarness drives an inboundCoalescer deterministically: a mutable injected
// clock shared with the typing gate, a non-firing afterFunc (so real timers
// never interfere), and a capture flush. Evaluation is driven by advancing the
// clock and calling pokeTyping, which flushes every buffer that is now ready.
type coalHarness struct {
	c *inboundCoalescer
	g *typingGate

	tmu sync.Mutex
	now time.Time

	bmu    sync.Mutex
	bursts [][]pendingMsg
}

func newCoalHarness() *coalHarness {
	h := &coalHarness{now: time.Unix(1_000_000, 0)}
	h.g = newTypingGate()
	h.g.clock = h.clock
	h.c = newInboundCoalescer(h.g, h.capture, true)
	h.c.clock = h.clock
	// Never let a real timer fire during a test; drive evaluation via pokeTyping.
	h.c.afterFunc = func(time.Duration, func()) *time.Timer { return time.NewTimer(time.Hour) }
	return h
}

func (h *coalHarness) clock() time.Time {
	h.tmu.Lock()
	defer h.tmu.Unlock()
	return h.now
}

func (h *coalHarness) advance(d time.Duration) {
	h.tmu.Lock()
	h.now = h.now.Add(d)
	h.tmu.Unlock()
}

func (h *coalHarness) capture(_ context.Context, msgs []pendingMsg) {
	h.bmu.Lock()
	h.bursts = append(h.bursts, msgs)
	h.bmu.Unlock()
}

func (h *coalHarness) got() [][]pendingMsg {
	h.bmu.Lock()
	defer h.bmu.Unlock()
	return h.bursts
}

// eval re-evaluates every buffer (the pokeTyping path), flushing any that are
// ready at the current clock.
func (h *coalHarness) eval() { h.c.pokeTyping(context.Background()) }

func msg(chatID, text string) (string, map[string]string) {
	return text, map[string]string{"chat_id": chatID, "user": "dev-a", "user_id": "dev-a", "message_id": "u-" + text}
}

// TestInboundCoalesceSingleMessagePassthrough proves one buffered message flushes
// as a single burst whose coalesceApp form is byte-identical passthrough.
func TestInboundCoalesceSingleMessagePassthrough(t *testing.T) {
	h := newCoalHarness()
	content, meta := msg("app", "hello")
	h.c.enqueue(context.Background(), content, meta)

	h.eval() // window not elapsed
	if len(h.got()) != 0 {
		t.Fatal("must not flush before the window elapses")
	}
	h.advance(appCoalesceWindow)
	h.eval()

	bursts := h.got()
	if len(bursts) != 1 || len(bursts[0]) != 1 {
		t.Fatalf("want one burst of one message, got %v", bursts)
	}
	gc, gm := coalesceApp(bursts[0])
	if gc != "hello" || gm["bubbles"] != "" {
		t.Fatalf("passthrough broke: content=%q bubbles=%q", gc, gm["bubbles"])
	}
}

// TestInboundCoalesceBurstOneSend proves a rapid burst of fragments flushes as
// ONE burst with bubbles=N and newline-joined content.
func TestInboundCoalesceBurstOneSend(t *testing.T) {
	h := newCoalHarness()
	for _, frag := range []string{"ok so", "the auth thing", "actually just the login fn"} {
		c, m := msg("app", frag)
		h.c.enqueue(context.Background(), c, m)
		h.advance(300 * time.Millisecond) // well under the 2s window
		h.eval()
		if len(h.got()) != 0 {
			t.Fatalf("must keep holding mid-burst (fragment %q)", frag)
		}
	}
	h.advance(appCoalesceWindow)
	h.eval()

	bursts := h.got()
	if len(bursts) != 1 || len(bursts[0]) != 3 {
		t.Fatalf("want one burst of three, got %v", bursts)
	}
	content, meta := coalesceApp(bursts[0])
	if meta["bubbles"] != "3" {
		t.Fatalf("bubbles = %q, want 3", meta["bubbles"])
	}
	if content != "ok so\nthe auth thing\nactually just the login fn" {
		t.Fatalf("merged content = %q", content)
	}
}

func TestRenderPartAppIncludesGenericAttachment(t *testing.T) {
	got := renderPartApp(pendingMsg{content: "(attachment)", meta: map[string]string{
		"attachment_file_id": "x-document",
		"attachment_name":    "report.pdf",
		"attachment_kind":    "document",
	}})
	if want := "[attachment: report.pdf id=x-document kind=document]"; got != want {
		t.Fatalf("attachment marker = %q, want %q", got, want)
	}
}

// TestInboundCoalesceTypingHoldExtendsPastWindow proves a live typing hold keeps
// the buffer past the arrival window, and an explicit state:false releases it.
func TestInboundCoalesceTypingHoldExtendsPastWindow(t *testing.T) {
	h := newCoalHarness()
	h.g.set("dev-a", true)
	c, m := msg("app", "wait for it")
	h.c.enqueue(context.Background(), c, m)

	h.advance(appCoalesceWindow + time.Second) // well past the window
	h.eval()
	if len(h.got()) != 0 {
		t.Fatal("typing hold must keep the buffer past the arrival window")
	}

	h.g.set("dev-a", false) // composer cleared / send
	h.eval()
	if len(h.got()) != 1 {
		t.Fatal("state:false must release the held buffer")
	}
}

// TestInboundCoalesceHardCapOverridesTyping proves the 15s hard cap flushes even
// while a device keeps typing (a distracted typist never wedges the harness).
func TestInboundCoalesceHardCapOverridesTyping(t *testing.T) {
	h := newCoalHarness()
	h.g.set("dev-a", true)
	c, m := msg("app", "typing forever")
	h.c.enqueue(context.Background(), c, m)

	// Keep the typing hold continuously live across the whole hard-cap window.
	for elapsed := time.Duration(0); elapsed < appCoalesceHardCap; elapsed += 3 * time.Second {
		h.advance(3 * time.Second)
		h.g.set("dev-a", true) // refresh so the gate never lapses on its own
		h.eval()
	}
	if !h.g.active() {
		t.Fatal("precondition: typing must still be active at the hard cap")
	}
	if len(h.got()) != 1 {
		t.Fatalf("hard cap must flush despite the live typing hold, bursts=%v", h.got())
	}
}

// TestInboundCoalesceMaxMsgsCap proves a 6-message burst flushes on the count cap.
func TestInboundCoalesceMaxMsgsCap(t *testing.T) {
	h := newCoalHarness()
	h.g.set("dev-a", true) // even while holding, the count cap wins
	for i := 0; i < appCoalesceMaxMsgs; i++ {
		c, m := msg("app", string(rune('a'+i)))
		h.c.enqueue(context.Background(), c, m)
		h.advance(50 * time.Millisecond)
	}
	h.eval()
	bursts := h.got()
	if len(bursts) != 1 || len(bursts[0]) != appCoalesceMaxMsgs {
		t.Fatalf("count cap must flush all %d, got %v", appCoalesceMaxMsgs, bursts)
	}
}

// TestInboundCoalesceMultiDeviceHold proves the hold is an OR over devices: a
// second device typing keeps the burst held after the first releases.
func TestInboundCoalesceMultiDeviceHold(t *testing.T) {
	h := newCoalHarness()
	h.g.set("dev-a", true)
	h.g.set("dev-b", true)
	c, m := msg("app", "cross-device")
	h.c.enqueue(context.Background(), c, m)

	h.advance(appCoalesceWindow + time.Second)
	h.g.set("dev-a", false) // one device stops
	h.eval()
	if len(h.got()) != 0 {
		t.Fatal("dev-b still typing must keep the buffer held")
	}
	h.g.set("dev-b", false)
	h.eval()
	if len(h.got()) != 1 {
		t.Fatal("buffer must release once every device stopped")
	}
}

// TestInboundCoalesceTimerAutoFires proves the production timer path delivers
// with no manual poke, using real (small) durations and the real AfterFunc.
func TestInboundCoalesceTimerAutoFires(t *testing.T) {
	done := make(chan []pendingMsg, 1)
	g := newTypingGate()
	c := newInboundCoalescer(g, func(_ context.Context, msgs []pendingMsg) { done <- msgs }, true)
	c.window = 30 * time.Millisecond
	c.grace = 15 * time.Millisecond

	content, meta := msg("app", "auto")
	c.enqueue(context.Background(), content, meta)

	select {
	case burst := <-done:
		if len(burst) != 1 {
			t.Fatalf("timer flushed %d messages, want 1", len(burst))
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timer never auto-flushed the buffer")
	}
}

// --- Server-level integration ---

// enableCoalescing turns on the coalescer and swaps in a deterministic clock +
// non-firing timer so a test can drive flushes via pokeTyping. It returns an
// advance() closure.
func enableCoalescing(srv *Server) (advance func(time.Duration)) {
	var mu sync.Mutex
	now := time.Unix(1_000_000, 0)
	clock := func() time.Time { mu.Lock(); defer mu.Unlock(); return now }
	srv.inbound.enabled = true
	srv.inbound.clock = clock
	srv.inbound.afterFunc = func(time.Duration, func()) *time.Timer { return time.NewTimer(time.Hour) }
	srv.typing.clock = clock
	return func(d time.Duration) { mu.Lock(); now = now.Add(d); mu.Unlock() }
}

// TestCoalescedMultiMessageInjection proves two rapid device_sends reach the
// harness as ONE SendChannel with bubbles=N — the harness-facing shape matching
// telegram's coalesced block.
func TestCoalescedMultiMessageInjection(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	advance := enableCoalescing(srv)
	sink := newFakeSink()
	srv.bindSink(sink)
	ctx := context.Background()

	for i, text := range []string{"first bubble", "second bubble"} {
		frame := deviceSendFrame{
			T:       "device_send",
			CID:     "cid-000000000000000" + string(rune('1'+i)),
			Payload: json.RawMessage(`{"t":"send","text":"` + text + `"}`),
		}
		if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
			t.Fatalf("device_send %d: %v", i, err)
		}
	}

	// Nothing delivered yet (still inside the window).
	select {
	case c := <-sink.ch:
		t.Fatalf("delivered before the window elapsed: %+v", c)
	default:
	}

	advance(appCoalesceWindow)
	srv.inbound.pokeTyping(ctx)

	select {
	case c := <-sink.ch:
		if c.meta["bubbles"] != "2" {
			t.Fatalf("bubbles = %q, want 2", c.meta["bubbles"])
		}
		if c.content != "first bubble\nsecond bubble" {
			t.Fatalf("coalesced content = %q", c.content)
		}
	default:
		t.Fatal("coalesced burst never reached the harness")
	}
	// Exactly one SendChannel for the burst.
	select {
	case c := <-sink.ch:
		t.Fatalf("second unexpected SendChannel: %+v", c)
	default:
	}
}

// TestCoalesceAppendAtAcceptDeliverAtFlush proves transcript timing: Append at
// accept (before delivery) and MarkDelivered only after the flush.
func TestCoalesceAppendAtAcceptDeliverAtFlush(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	advance := enableCoalescing(srv)
	sink := newFakeSink()
	srv.bindSink(sink)
	ctx := context.Background()

	frame := deviceSendFrame{T: "device_send", CID: "cid-0000000000000abc", Payload: json.RawMessage(`{"t":"send","text":"journal me"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
		t.Fatalf("device_send: %v", err)
	}

	// Accepted but not yet delivered: the inbound is journaled, without a
	// delivered marker, so catch-up would replay it if the box crashed now.
	if in, delivered := transcriptCounts(t, srv.cfg.TranscriptFile); in != 1 || delivered != 0 {
		t.Fatalf("after accept: in=%d delivered=%d, want 1/0", in, delivered)
	}

	advance(appCoalesceWindow)
	srv.inbound.pokeTyping(ctx)
	<-sink.ch // ensure the flush ran

	if in, delivered := transcriptCounts(t, srv.cfg.TranscriptFile); in != 1 || delivered != 1 {
		t.Fatalf("after flush: in=%d delivered=%d, want 1/1", in, delivered)
	}
}

// TestCoalesceNilSinkReplayableViaCatchup proves a flush with no live sink drops
// the delivery but leaves the accepted message replayable: catch-up redelivers it
// (the crash-mid-hold heal).
func TestCoalesceNilSinkReplayableViaCatchup(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	advance := enableCoalescing(srv)
	// No sink bound: currentSink() is nil.
	ctx := context.Background()

	frame := deviceSendFrame{T: "device_send", CID: "cid-0000000000000def", Payload: json.RawMessage(`{"t":"send","text":"replay me"}`)}
	if err := srv.handleDeviceSend(ctx, ids[0], frame); err != nil {
		t.Fatalf("device_send: %v", err)
	}
	advance(appCoalesceWindow)
	srv.inbound.pokeTyping(ctx) // flush hits a nil sink → dropped, not MarkDelivered

	if in, delivered := transcriptCounts(t, srv.cfg.TranscriptFile); in != 1 || delivered != 0 {
		t.Fatalf("nil-sink flush must leave in=1 delivered=0, got %d/%d", in, delivered)
	}

	// Catch-up over the transcript redelivers the un-delivered turn.
	replaySink := newFakeSink()
	n, err := catchup.ReplayUnanswered(ctx, replaySink, srv.log, srv.cfg.TranscriptFile, time.Now(), catchup.DefaultWindow)
	if err != nil {
		t.Fatalf("catchup: %v", err)
	}
	if n != 1 {
		t.Fatalf("catch-up replayed %d turns, want 1", n)
	}
}

// TestTypingFrameNoMailboxNoWake proves a typing frame is consumed silently: no
// error, gate armed, no mailbox item, and zero push-intent fires (a typing frame
// is structurally incapable of a wake).
func TestTypingFrameNoMailboxNoWake(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	var wakes int
	srv.mailbox.onPushIntent = func(string, MailboxItem) { wakes++ }
	before := len(srv.mailbox.disk.Devices[ids[0]].Items)
	ctx := context.Background()

	bad, fatal := srv.handleSessionInput(ctx, ids[0], subs[0], []byte(`{"t":"typing","state":true}`), writeSink())
	if bad || fatal {
		t.Fatalf("typing frame must be clean: bad=%v fatal=%v", bad, fatal)
	}
	if !srv.typing.active() {
		t.Fatal("typing gate must be armed by state:true")
	}
	if after := len(srv.mailbox.disk.Devices[ids[0]].Items); after != before {
		t.Fatalf("typing frame created %d mailbox item(s)", after-before)
	}
	if wakes != 0 {
		t.Fatalf("typing frame triggered %d push intent(s), want 0", wakes)
	}
	if in, _ := transcriptCounts(t, srv.cfg.TranscriptFile); in != 0 {
		t.Fatalf("typing frame journaled %d inbound record(s), want 0", in)
	}
}

// TestTypingFrameParseAndUnknownCompat proves state parsing, fail-open on
// malformed state, and forward-compat silence on an unknown frame type.
func TestTypingFrameParseAndUnknownCompat(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	ctx := context.Background()

	feed := func(raw string) (bad, fatal bool) {
		return srv.handleSessionInput(ctx, ids[0], subs[0], []byte(raw), writeSink())
	}

	if bad, _ := feed(`{"t":"typing","state":true}`); bad {
		t.Fatal("valid typing frame must not be a bad_frame")
	}
	if !srv.typing.active() {
		t.Fatal("state:true must arm the gate")
	}
	if bad, _ := feed(`{"t":"typing","state":false}`); bad {
		t.Fatal("state:false must not be a bad_frame")
	}
	if srv.typing.active() {
		t.Fatal("state:false must release the gate")
	}
	// Malformed state fails OPEN (treated as false), never a bad_frame.
	if bad, _ := feed(`{"t":"typing","state":"oops"}`); bad {
		t.Fatal("malformed typing.state must fail open, not bad_frame")
	}
	if srv.typing.active() {
		t.Fatal("malformed state must leave the gate idle (fail-open)")
	}
	// An unknown frame type an old box never learned is silently ignored.
	if bad, fatal := feed(`{"t":"totally_unknown_frame"}`); bad || fatal {
		t.Fatalf("unknown frame must be ignored: bad=%v fatal=%v", bad, fatal)
	}
}

// transcriptCounts returns (inbound records, delivered markers) in a transcript.
func transcriptCounts(t *testing.T, path string) (in, delivered int) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return 0, 0
		}
		t.Fatalf("read transcript: %v", err)
	}
	for _, line := range strings.Split(string(data), "\n") {
		if line == "" {
			continue
		}
		var r struct {
			Dir  string `json:"dir"`
			Kind string `json:"kind"`
		}
		if json.Unmarshal([]byte(line), &r) != nil {
			continue
		}
		switch {
		case r.Dir == "in":
			in++
		case r.Dir == "meta" && r.Kind == "delivered":
			delivered++
		}
	}
	return in, delivered
}

// TestServerAppliesCoalesceWindowOverride verifies HOTLINE_APP_COALESCE_WINDOW
// (surfaced as cfg.AppCoalesceWindow) overrides BOTH the window and the grace,
// and that an unset (zero) config leaves the built-in defaults intact.
func TestServerAppliesCoalesceWindowOverride(t *testing.T) {
	dir := t.TempDir()

	base := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl")}
	def := NewServer(base, transcript.New(base.TranscriptFile))
	if def.initErr != nil {
		t.Fatal(def.initErr)
	}
	if def.inbound.window != appCoalesceWindow || def.inbound.grace != appCoalesceGrace {
		t.Fatalf("unset config should keep defaults: window=%v grace=%v", def.inbound.window, def.inbound.grace)
	}

	cfg := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl"), AppCoalesceWindow: 7 * time.Second}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	if srv.inbound.window != 7*time.Second || srv.inbound.grace != 7*time.Second {
		t.Fatalf("override should set both window and grace to 7s, got window=%v grace=%v", srv.inbound.window, srv.inbound.grace)
	}
}
