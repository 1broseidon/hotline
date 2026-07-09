package telegram

import (
	"context"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

// Inbound coalescing. People text in bursts — "ok so" / "the auth thing" /
// "actually just the login fn?" — three messages in two seconds. Without
// buffering, each arrives as its own <channel> notification and (when Claude is
// idle between them) spawns its own turn, so Claude races the operator and
// replies to half-finished thoughts. This is the inbound mirror of pace.go's
// outbound bubble pacing: hold a short window after each message and flush the
// burst as ONE turn once they stop typing.
//
// The window is adaptive: a message that looks complete (terminal punctuation
// or long) gets only a short grace hold, so a fast follow-up still merges while
// single messages stay snappy; genuine fragments pay the full hold. Hard caps
// (message count, total wait) and a flush-on-shutdown drain bound the worst
// case so nothing is stranded.
const (
	defaultCoalesceWindow  = 1200 * time.Millisecond
	defaultCoalesceMaxWait = 8 * time.Second
	// defaultGraceWindow is the brief hold a complete-looking message gets
	// instead of an instant flush, so a fast follow-up still coalesces into the
	// same burst rather than racing its own turn.
	defaultGraceWindow = 500 * time.Millisecond
	// coalesceMaxMsgs flushes a burst once this many messages accumulate, so a
	// fast typist never starves behind an ever-re-armed window.
	coalesceMaxMsgs = 6
	// coalesceLongRune treats a message at least this long as a complete thought
	// worth flushing immediately.
	coalesceLongRune = 80
)

// pendingMsg is one buffered inbound message: the content and meta exactly as
// handleMessage built them.
type pendingMsg struct {
	content string
	meta    map[string]string
}

// chatBuffer accumulates a burst for a single chat.
type chatBuffer struct {
	msgs    []pendingMsg
	timer   *time.Timer
	firstAt time.Time
}

// enqueue buffers an inbound message and (re)arms the coalescing window. When
// the window elapses with no new message — or a flush condition trips — the
// accumulated burst is delivered as one coalesced turn.
func (h *Handler) enqueue(ctx context.Context, content string, meta map[string]string) {
	chatID := meta["chat_id"]

	h.coalMu.Lock()
	if h.buffers == nil {
		h.buffers = make(map[string]*chatBuffer)
	}
	buf := h.buffers[chatID]
	if buf == nil {
		buf = &chatBuffer{firstAt: time.Now()}
		h.buffers[chatID] = buf
	}
	buf.msgs = append(buf.msgs, pendingMsg{content: content, meta: meta})
	if buf.timer != nil {
		buf.timer.Stop()
		buf.timer = nil
	}

	// Hard caps still flush right now. A complete-looking message no longer
	// flushes instantly; it takes the short grace hold below so a fast follow-up
	// still merges into the same burst.
	if len(buf.msgs) >= coalesceMaxMsgs ||
		time.Since(buf.firstAt) >= h.coalesceMaxWait {
		msgs := buf.msgs
		delete(h.buffers, chatID)
		h.coalMu.Unlock()
		h.deliver(ctx, msgs)
		return
	}

	// Arm the window by the just-arrived message's completeness: a complete
	// thought gets the short grace hold, a fragment the full window.
	window := h.coalesceWindow
	if looksComplete(content) {
		window = h.graceWindow
	}
	buf.timer = time.AfterFunc(window, func() {
		h.coalMu.Lock()
		b := h.buffers[chatID]
		if b == nil {
			h.coalMu.Unlock()
			return
		}
		msgs := b.msgs
		delete(h.buffers, chatID)
		h.coalMu.Unlock()
		// The dispatch ctx may be gone by now; use a fresh one so a flush that
		// fires near shutdown still reaches Claude.
		h.deliver(context.Background(), msgs)
	})
	h.coalMu.Unlock()
}

// FlushAll drains every pending buffer immediately. Called at shutdown so a
// burst caught mid-window isn't lost when polling stops.
func (h *Handler) FlushAll(ctx context.Context) {
	h.coalMu.Lock()
	var pending [][]pendingMsg
	for id, b := range h.buffers {
		if b.timer != nil {
			b.timer.Stop()
		}
		pending = append(pending, b.msgs)
		delete(h.buffers, id)
	}
	h.coalMu.Unlock()
	for _, msgs := range pending {
		h.deliver(ctx, msgs)
	}
}

// deliver routes a flushed burst. Tests swap coalDeliver to capture bursts
// without a live notifier; production leaves it nil and uses flush.
func (h *Handler) deliver(ctx context.Context, msgs []pendingMsg) {
	if h.coalDeliver != nil {
		h.coalDeliver(ctx, msgs)
		return
	}
	h.flush(ctx, msgs)
}

// flush logs each real message to the transcript (keeping the durable record
// granular, message by message) and delivers the burst to Claude as a single
// coalesced notification.
func (h *Handler) flush(ctx context.Context, msgs []pendingMsg) {
	if len(msgs) == 0 {
		return
	}
	for _, m := range msgs {
		h.Log.Append(transcript.Record{
			Dir:       "in",
			ChatID:    m.meta["chat_id"],
			User:      m.meta["user"],
			UserID:    m.meta["user_id"],
			Kind:      inboundKind(m.meta),
			MessageID: m.meta["message_id"],
			Text:      m.content,
		})
	}
	content, meta := coalesce(msgs)
	if h.Notifier == nil {
		return
	}
	if err := h.Notifier.SendChannel(ctx, content, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: deliver inbound failed: %v\n", err)
	}
}

// coalesce merges a burst into one (content, meta). A single message is
// returned unchanged — same content, same attribute-form meta — so the common
// case is byte-identical to the pre-coalescing behavior. Two or more messages
// are joined newline-per-bubble with attachments rendered as inline markers
// (the <channel> attribute form can only carry one attachment), carrying the
// last message_id and last reply-to context, with a bubbles=N count.
func coalesce(msgs []pendingMsg) (string, map[string]string) {
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
		parts = append(parts, renderPart(m))
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

// renderPart renders one buffered message as text plus an inline attachment
// marker. A bare media message (synthetic "(photo)" content) becomes just the
// marker; a captioned attachment keeps its caption with the marker on the next
// line.
func renderPart(m pendingMsg) string {
	text := strings.TrimSpace(m.content)
	marker := attachmentMarker(m.meta)
	if marker == "" {
		return text
	}
	if looksSynthetic(text) {
		return marker
	}
	return text + "\n" + marker
}

// attachmentMarker builds the inline marker for whatever attachment a message's
// meta carries: a ready-to-Read path for eager-downloaded photos, or a file_id
// (passed to download_attachment) for lazy documents and other media.
func attachmentMarker(meta map[string]string) string {
	if p := meta["image_path"]; p != "" {
		return "[image: " + p + "]"
	}
	if id := meta["attachment_file_id"]; id != "" {
		name := nonEmpty(meta["attachment_name"], meta["attachment_kind"])
		return fmt.Sprintf("[attachment: %s id=%s kind=%s]", nonEmpty(name, "file"), id, meta["attachment_kind"])
	}
	return ""
}

// looksSynthetic reports whether content is one of media.go's parenthesized
// placeholders ("(photo)", "(document: x)", …) rather than a real caption.
func looksSynthetic(s string) bool {
	return strings.HasPrefix(s, "(") && strings.HasSuffix(s, ")") && !strings.Contains(s, "\n")
}

// looksComplete reports whether a message reads as a finished thought worth
// flushing immediately, rather than a fragment to hold the window open for.
func looksComplete(text string) bool {
	t := strings.TrimSpace(text)
	if t == "" {
		return false
	}
	// Trailing ellipsis signals more is coming — keep holding.
	if strings.HasSuffix(t, "...") || strings.HasSuffix(t, "…") {
		return false
	}
	if len([]rune(t)) >= coalesceLongRune {
		return true
	}
	switch []rune(t)[len([]rune(t))-1] {
	case '.', '?', '!', '。', '？', '！':
		return true
	}
	return false
}
