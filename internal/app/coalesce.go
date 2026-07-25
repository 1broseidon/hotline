package app

import (
	"context"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"
)

// Inbound coalescing for the app channel. Until now the app channel — alone
// among the channels (telegram/signal/discord each have one) — delivered one
// SendChannel per bubble, so a texting burst raced its own turns and the MCP
// instruction's "bubbles=N" promise was false here. This adds the missing
// coalescer, modeled on internal/telegram/coalesce.go, and layers the app-only
// typing gate on top as an extra hold condition: while any device asserts it is
// typing, the buffer holds regardless of the arrival window.
//
// Two hold signals, one flush:
//   - The typing gate is the PRIMARY hold on the app channel (a live signal that
//     "more is coming").
//   - The adaptive arrival window is the FALLBACK for typing-less clients (web,
//     old app versions), so they still get burst coalescing with no signal at
//     all.
//
// Tuning (product-owner override, 2026-07-15 field feedback: "telegram batching
// never really worked — 500ms is not enough for a human to type"): the base
// quiet window and the complete-looking grace are UNIFIED at 3s (George, live
// tuning 2026-07-15: 500ms then 1.5s both split natural bursts): one number —
// 3s of quiet ships, typing within it holds. Hard caps 15s / 6 msgs unchanged.
const (
	// appCoalesceWindow is the base quiet window: a fragment holds this long after
	// the last message before flushing, absent a typing hold.
	appCoalesceWindow = 3 * time.Second
	// appCoalesceGrace once fast-tracked complete-looking messages (500ms, then
	// 1.5s) — live tuning 2026-07-15 showed BOTH split natural bursts: the gap
	// between a send and the next draft's first typing frame is routinely 1.5-3s.
	// Unified with the window at 3s; the constant survives only so the
	// looksCompleteApp branch stays wired for future re-tuning.
	appCoalesceGrace = 3 * time.Second
	// appCoalesceMaxMsgs flushes a burst once this many messages accumulate.
	appCoalesceMaxMsgs = 6
	// appCoalesceHardCap bounds a single burst: a distracted typist (or a stuck
	// typing hold on another device) never wedges the harness longer than this. It
	// overrides the typing hold. Larger than telegram's 8s because the app channel
	// has a live typing signal actively earning the extra leash.
	appCoalesceHardCap = 15 * time.Second
	// appCoalesceLongRune treats a message at least this long as a complete thought.
	appCoalesceLongRune = 80
)

// pendingMsg is one buffered inbound message: the harness content and meta
// exactly as handleDeviceSend built them at accept time.
type pendingMsg struct {
	content string
	meta    map[string]string
}

// chatBuffer accumulates a burst for a single chat_id.
type chatBuffer struct {
	msgs    []pendingMsg
	timer   *time.Timer
	firstAt time.Time
	lastAt  time.Time
}

// inboundCoalescer buffers app-channel inbound per chat_id and flushes a burst as
// ONE SendChannel, holding while any device's typing gate is live. Delivery
// (flush) always runs off the buffering goroutine (a timer, poke, or shutdown
// drain), never under the caller's deliveryMu — so SendChannel stays lock-free,
// matching the telegram coalescer's discipline.
type inboundCoalescer struct {
	mu      sync.Mutex
	buffers map[string]*chatBuffer
	typing  *typingGate

	// flush delivers a coalesced burst (SendChannel + per-message MarkDelivered).
	// Tests swap it to capture bursts without a live sink.
	flush func(ctx context.Context, msgs []pendingMsg)

	// enabled gates whether handleDeviceSend routes through the coalescer. Off ⇒
	// the legacy synchronous delivery path is used and this coalescer is inert.
	enabled bool

	clock     func() time.Time                        // injectable for tests; nil ⇒ time.Now
	afterFunc func(time.Duration, func()) *time.Timer // injectable for tests; nil ⇒ time.AfterFunc

	window  time.Duration
	grace   time.Duration
	maxMsgs int
	hardCap time.Duration
}

func newInboundCoalescer(typing *typingGate, flush func(context.Context, []pendingMsg), enabled bool) *inboundCoalescer {
	return &inboundCoalescer{
		buffers:   make(map[string]*chatBuffer),
		typing:    typing,
		flush:     flush,
		enabled:   enabled,
		clock:     time.Now,
		afterFunc: time.AfterFunc,
		window:    appCoalesceWindow,
		grace:     appCoalesceGrace,
		maxMsgs:   appCoalesceMaxMsgs,
		hardCap:   appCoalesceHardCap,
	}
}

func (c *inboundCoalescer) now() time.Time {
	if c.clock != nil {
		return c.clock()
	}
	return time.Now()
}

// enqueue buffers an accepted inbound message and (re)arms the buffer's flush
// timer. It NEVER flushes inline: the caller (handleDeviceSend) holds
// deliveryMu, and flush must run lock-free. Even a hard-cap-tripping message
// arms a zero-delay timer so the actual delivery happens on the timer goroutine.
func (c *inboundCoalescer) enqueue(_ context.Context, content string, meta map[string]string) {
	chatID := meta["chat_id"]
	c.mu.Lock()
	defer c.mu.Unlock()
	buf := c.buffers[chatID]
	now := c.now()
	if buf == nil {
		buf = &chatBuffer{firstAt: now}
		c.buffers[chatID] = buf
	}
	buf.msgs = append(buf.msgs, pendingMsg{content: content, meta: meta})
	buf.lastAt = now
	c.scheduleLocked(chatID, buf)
}

// pokeTyping re-evaluates every pending buffer after a typing-state change. It
// runs OUTSIDE deliveryMu (called from the connector's typing case), so a buffer
// that is now releasable (e.g. a state:false dropped the last hold and the
// window has elapsed) is flushed synchronously here, giving the <100ms release
// the explicit state:false frame exists for. Buffers not yet ready are re-armed.
func (c *inboundCoalescer) pokeTyping(ctx context.Context) {
	c.mu.Lock()
	var ready [][]pendingMsg
	for chatID, buf := range c.buffers {
		if c.delayLocked(buf) <= 0 {
			ready = append(ready, buf.msgs)
			if buf.timer != nil {
				buf.timer.Stop()
			}
			delete(c.buffers, chatID)
			continue
		}
		c.scheduleLocked(chatID, buf)
	}
	c.mu.Unlock()
	for _, msgs := range ready {
		c.deliver(ctx, msgs)
	}
}

// FlushAll drains every pending buffer immediately. Called at shutdown so a
// burst caught mid-hold isn't lost when the connector stops.
func (c *inboundCoalescer) FlushAll(ctx context.Context) {
	c.mu.Lock()
	var pending [][]pendingMsg
	for id, b := range c.buffers {
		if b.timer != nil {
			b.timer.Stop()
		}
		pending = append(pending, b.msgs)
		delete(c.buffers, id)
	}
	c.mu.Unlock()
	for _, msgs := range pending {
		c.deliver(ctx, msgs)
	}
}

// deliver routes a flushed burst to the flush callback (nil in a misconfigured
// build is a no-op).
func (c *inboundCoalescer) deliver(ctx context.Context, msgs []pendingMsg) {
	if len(msgs) == 0 || c.flush == nil {
		return
	}
	c.flush(ctx, msgs)
}

// scheduleLocked (re)arms buf.timer to the delay until it should next be
// evaluated. A zero delay flushes as soon as the timer goroutine runs.
func (c *inboundCoalescer) scheduleLocked(chatID string, buf *chatBuffer) {
	d := c.delayLocked(buf)
	if buf.timer != nil {
		buf.timer.Stop()
	}
	buf.timer = c.afterFunc(d, func() { c.fire(chatID) })
}

// delayLocked returns how long until buf should flush. Zero means "now".
//
//	flush iff  len(msgs) >= maxMsgs                         (6, telegram parity)
//	        OR since(firstAt) >= hardCap                    (15s — overrides typing)
//	        OR ( !typing.active() AND since(lastAt) >= window )
//
// While a typing hold is live the arrival window cannot release the buffer; the
// next wake is the earlier of {earliest typing expiry, hard-cap deadline} so the
// buffer re-checks the moment a hold might lapse (a refresh re-arms it later).
func (c *inboundCoalescer) delayLocked(buf *chatBuffer) time.Duration {
	if len(buf.msgs) == 0 {
		return c.window
	}
	now := c.now()
	if len(buf.msgs) >= c.maxMsgs || now.Sub(buf.firstAt) >= c.hardCap {
		return 0
	}
	deadline := buf.firstAt.Add(c.hardCap) // hard-cap ceiling always applies
	if c.typing != nil && c.typing.active() {
		if exp, ok := c.typing.earliestExpiry(); ok && exp.Before(deadline) {
			deadline = exp
		}
	} else {
		window := c.window
		if looksCompleteApp(buf.msgs[len(buf.msgs)-1].content) {
			window = c.grace
		}
		if sd := buf.lastAt.Add(window); sd.Before(deadline) {
			deadline = sd
		}
	}
	if d := deadline.Sub(now); d > 0 {
		return d
	}
	return 0
}

// fire is the timer callback: flush if ready, otherwise re-arm (a typing hold is
// still live, or the window has not elapsed).
func (c *inboundCoalescer) fire(chatID string) {
	c.mu.Lock()
	buf := c.buffers[chatID]
	if buf == nil {
		c.mu.Unlock()
		return
	}
	if c.delayLocked(buf) > 0 {
		c.scheduleLocked(chatID, buf)
		c.mu.Unlock()
		return
	}
	msgs := buf.msgs
	if buf.timer != nil {
		buf.timer.Stop()
	}
	delete(c.buffers, chatID)
	c.mu.Unlock()
	// The accepting ctx is long gone; use a fresh one so a flush near shutdown
	// still reaches the harness.
	c.deliver(context.Background(), msgs)
}

// coalesceApp merges a burst into one (content, meta). A single message is
// returned unchanged — byte-identical passthrough, exactly like the telegram
// coalescer. Two or more are joined newline-per-bubble with attachments
// rendered as inline markers (the <channel> attribute form carries only one),
// carrying the last message_id and last reply-to context, with a bubbles=N
// count. App uploads use image_path for photos and attachment_file_id for files.
func coalesceApp(msgs []pendingMsg) (string, map[string]string) {
	if len(msgs) == 1 {
		return msgs[0].content, msgs[0].meta
	}
	meta := map[string]string{
		"chat_id": msgs[0].meta["chat_id"],
		"user":    msgs[0].meta["user"],
		"user_id": msgs[0].meta["user_id"],
	}
	parts := make([]string, 0, len(msgs))
	var lastReply map[string]string
	for _, m := range msgs {
		parts = append(parts, renderPartApp(m))
		if v := m.meta["ts"]; v != "" {
			meta["ts"] = v
		}
		if v := m.meta["message_id"]; v != "" {
			meta["message_id"] = v
		}
		if m.meta["reply_to_message_id"] != "" {
			lastReply = m.meta
		}
	}
	if lastReply != nil {
		for _, k := range []string{"reply_to_message_id", "reply_to_from", "reply_to_text"} {
			if v := lastReply[k]; v != "" {
				meta[k] = v
			}
		}
	}
	meta["bubbles"] = strconv.Itoa(len(msgs))
	return strings.Join(parts, "\n"), meta
}

// renderPartApp renders one buffered message as text plus an inline attachment marker.
func renderPartApp(m pendingMsg) string {
	text := strings.TrimSpace(m.content)
	marker := ""
	if p := m.meta["image_path"]; p != "" {
		marker = "[image: " + p + "]"
	} else if id := m.meta["attachment_file_id"]; id != "" {
		name := m.meta["attachment_name"]
		if name == "" {
			name = "file"
		}
		kind := m.meta["attachment_kind"]
		if kind == "" {
			kind = "document"
		}
		marker = fmt.Sprintf("[attachment: %s id=%s kind=%s]", name, id, kind)
	}
	if marker == "" {
		return text
	}
	if text == "" || looksSyntheticApp(text) {
		return marker
	}
	return text + "\n" + marker
}

// looksSyntheticApp reports whether content is a parenthesized placeholder
// ("(photo)") rather than a real caption.
func looksSyntheticApp(s string) bool {
	return strings.HasPrefix(s, "(") && strings.HasSuffix(s, ")") && !strings.Contains(s, "\n")
}

// looksCompleteApp reports whether a message reads as a finished thought worth
// the short grace hold instead of the full window.
func looksCompleteApp(text string) bool {
	t := strings.TrimSpace(text)
	if t == "" {
		return false
	}
	if strings.HasSuffix(t, "...") || strings.HasSuffix(t, "…") {
		return false
	}
	r := []rune(t)
	if len(r) >= appCoalesceLongRune {
		return true
	}
	switch r[len(r)-1] {
	case '.', '?', '!', '。', '？', '！':
		return true
	}
	return false
}
