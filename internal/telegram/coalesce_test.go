package telegram

import (
	"context"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestLooksComplete(t *testing.T) {
	cases := []struct {
		in   string
		want bool
	}{
		{"can you check auth.go?", true},
		{"ship it.", true},
		{"no!", true},
		{"ok so", false},
		{"wait", false},
		{"hold on...", false}, // trailing ellipsis = more coming
		{"hold on…", false},
		{"", false},
		{"(photo)", false}, // bare media placeholder is a fragment, hold it
		{strings.Repeat("x", coalesceLongRune), true}, // long = complete
	}
	for _, c := range cases {
		if got := looksComplete(c.in); got != c.want {
			t.Errorf("looksComplete(%q) = %v, want %v", c.in, got, c.want)
		}
	}
}

func TestAttachmentMarker(t *testing.T) {
	if got := attachmentMarker(map[string]string{"image_path": "/inbox/p.jpg"}); got != "[image: /inbox/p.jpg]" {
		t.Errorf("photo marker = %q", got)
	}
	got := attachmentMarker(map[string]string{
		"attachment_file_id": "BQACabc",
		"attachment_kind":    "document",
		"attachment_name":    "report.pdf",
	})
	if got != "[attachment: report.pdf id=BQACabc kind=document]" {
		t.Errorf("doc marker = %q", got)
	}
	// No name falls back to kind, then "file".
	if got := attachmentMarker(map[string]string{"attachment_file_id": "X", "attachment_kind": "voice"}); got != "[attachment: voice id=X kind=voice]" {
		t.Errorf("voice marker = %q", got)
	}
	if got := attachmentMarker(map[string]string{}); got != "" {
		t.Errorf("no attachment should be empty, got %q", got)
	}
}

func TestRenderPart(t *testing.T) {
	// Plain text, no attachment.
	if got := renderPart(pendingMsg{content: "hey there"}); got != "hey there" {
		t.Errorf("text part = %q", got)
	}
	// Bare photo (synthetic caption) -> marker only, no "(photo)".
	bare := renderPart(pendingMsg{content: "(photo)", meta: map[string]string{"image_path": "/i/p.jpg"}})
	if bare != "[image: /i/p.jpg]" {
		t.Errorf("bare photo part = %q", bare)
	}
	// Captioned photo -> caption then marker.
	cap := renderPart(pendingMsg{content: "look at this", meta: map[string]string{"image_path": "/i/p.jpg"}})
	if cap != "look at this\n[image: /i/p.jpg]" {
		t.Errorf("captioned photo part = %q", cap)
	}
}

func TestCoalesceSingleUnchanged(t *testing.T) {
	meta := map[string]string{"chat_id": "5", "message_id": "9", "image_path": "/i/p.jpg"}
	content, gotMeta := coalesce([]pendingMsg{{content: "hi", meta: meta}})
	if content != "hi" {
		t.Errorf("single content = %q, want unchanged", content)
	}
	// Single message must keep the attribute-form meta untouched (the proven path).
	if gotMeta["image_path"] != "/i/p.jpg" {
		t.Errorf("single meta should be untouched, got %v", gotMeta)
	}
	if _, ok := gotMeta["bubbles"]; ok {
		t.Error("single message should not carry a bubbles count")
	}
}

func TestCoalesceBurst(t *testing.T) {
	msgs := []pendingMsg{
		{content: "ok so", meta: map[string]string{"chat_id": "5", "user": "sam", "user_id": "1", "ts": "t1", "message_id": "10"}},
		{content: "(photo)", meta: map[string]string{"chat_id": "5", "ts": "t2", "message_id": "11", "image_path": "/i/p.jpg"}},
		{content: "what's broken here?", meta: map[string]string{"chat_id": "5", "ts": "t3", "message_id": "12", "reply_to_message_id": "4", "reply_to_from": "you", "reply_to_text": "the build"}},
	}
	content, meta := coalesce(msgs)

	wantContent := "ok so\n[image: /i/p.jpg]\nwhat's broken here?"
	if content != wantContent {
		t.Errorf("burst content =\n%q\nwant\n%q", content, wantContent)
	}
	if meta["bubbles"] != "3" {
		t.Errorf("bubbles = %q, want 3", meta["bubbles"])
	}
	if meta["message_id"] != "12" {
		t.Errorf("message_id = %q, want last (12)", meta["message_id"])
	}
	if meta["ts"] != "t3" {
		t.Errorf("ts = %q, want last (t3)", meta["ts"])
	}
	if meta["user"] != "sam" || meta["user_id"] != "1" {
		t.Errorf("identity not carried from first: %v", meta)
	}
	// Reply-to from the last message that had it.
	if meta["reply_to_message_id"] != "4" || meta["reply_to_from"] != "you" {
		t.Errorf("reply-to context not carried: %v", meta)
	}
	// Attachment attributes must NOT leak — they're inline now.
	if meta["image_path"] != "" {
		t.Errorf("image_path should be inline-only in a burst, got %q", meta["image_path"])
	}
}

// testHandler builds a Handler wired to capture bursts instead of sending them.
// window drives the fragment hold; grace drives the complete-message hold — both
// injectable so tests can pick small, deterministic durations.
func testHandler(window, grace time.Duration) (*Handler, *burstSink) {
	sink := &burstSink{done: make(chan struct{}, 16)}
	h := &Handler{
		buffers:         make(map[string]*chatBuffer),
		coalesceWindow:  window,
		graceWindow:     grace,
		coalesceMaxWait: time.Second,
		coalDeliver: func(_ context.Context, msgs []pendingMsg) {
			sink.add(msgs)
		},
	}
	return h, sink
}

type burstSink struct {
	mu     sync.Mutex
	bursts [][]pendingMsg
	done   chan struct{}
}

func (s *burstSink) add(msgs []pendingMsg) {
	s.mu.Lock()
	s.bursts = append(s.bursts, msgs)
	s.mu.Unlock()
	s.done <- struct{}{}
}

func (s *burstSink) count() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.bursts)
}

func msg(chatID, text string) (context.Context, string, map[string]string) {
	return context.Background(), text, map[string]string{"chat_id": chatID}
}

func TestEnqueueCoalescesFragments(t *testing.T) {
	h, sink := testHandler(40*time.Millisecond, 20*time.Millisecond)
	// Three fragments in quick succession (none "complete").
	h.enqueue(msg("5", "ok so"))
	h.enqueue(msg("5", "the auth thing"))
	h.enqueue(msg("5", "lemme think"))
	// Nothing should fire before the window elapses.
	if sink.count() != 0 {
		t.Fatalf("flushed early: %d bursts", sink.count())
	}
	<-sink.done
	if sink.count() != 1 {
		t.Fatalf("want 1 coalesced burst, got %d", sink.count())
	}
	if n := len(sink.bursts[0]); n != 3 {
		t.Fatalf("burst should hold all 3 fragments, got %d", n)
	}
}

// A complete-looking message no longer flushes synchronously inside enqueue; it
// takes the grace hold, so a fast follow-up still merges into the same burst.
func TestEnqueueCompleteTakesGraceHold(t *testing.T) {
	h, sink := testHandler(time.Hour, 40*time.Millisecond) // full window can't fire; only grace can
	h.enqueue(msg("5", "wait"))                            // fragment, held
	h.enqueue(msg("5", "fix auth.go?"))                    // complete -> grace hold, not an instant flush
	if sink.count() != 0 {
		t.Fatalf("complete message flushed instantly instead of taking the grace hold: %d bursts", sink.count())
	}
	select {
	case <-sink.done:
	case <-time.After(time.Second):
		t.Fatal("complete message did not flush after the grace window")
	}
	if len(sink.bursts[0]) != 2 {
		t.Fatalf("want both messages coalesced in the burst, got %d", len(sink.bursts[0]))
	}
}

// Two complete-looking messages arriving within the grace window coalesce into
// one burst (bubbles=2) — the bug this change fixes.
func TestEnqueueTwoCompleteCoalesceUnderGrace(t *testing.T) {
	h, sink := testHandler(time.Hour, 60*time.Millisecond)
	h.enqueue(msg("5", "check auth.go please.")) // complete
	h.enqueue(msg("5", "the login path is off.")) // complete, within grace
	if sink.count() != 0 {
		t.Fatalf("flushed before the grace window elapsed: %d bursts", sink.count())
	}
	<-sink.done
	if sink.count() != 1 {
		t.Fatalf("want 1 coalesced burst, got %d", sink.count())
	}
	if n := len(sink.bursts[0]); n != 2 {
		t.Fatalf("burst should hold both complete messages, got %d", n)
	}
}

// A single complete message flushes on its own after the grace window — not
// synchronously, not held the full fragment window — delivering once with the
// original single-message content preserved.
func TestEnqueueSingleCompleteFlushesAfterGrace(t *testing.T) {
	h, sink := testHandler(time.Hour, 30*time.Millisecond) // full window can't fire; only grace can
	h.enqueue(msg("5", "ship it."))
	if sink.count() != 0 {
		t.Fatalf("single complete message flushed instantly instead of after grace: %d bursts", sink.count())
	}
	<-sink.done
	if sink.count() != 1 {
		t.Fatalf("want 1 delivery, got %d", sink.count())
	}
	if n := len(sink.bursts[0]); n != 1 {
		t.Fatalf("single message should deliver alone, got %d in burst", n)
	}
	if got := sink.bursts[0][0].content; got != "ship it." {
		t.Fatalf("single-message content changed: %q", got)
	}
}

func TestEnqueueSeparateChatsDontMix(t *testing.T) {
	h, sink := testHandler(30*time.Millisecond, 15*time.Millisecond)
	h.enqueue(msg("5", "hi from five"))
	h.enqueue(msg("9", "hi from nine"))
	<-sink.done
	<-sink.done
	if sink.count() != 2 {
		t.Fatalf("want 2 separate bursts, got %d", sink.count())
	}
	for _, b := range sink.bursts {
		if len(b) != 1 {
			t.Fatalf("chats should not mix; burst len = %d", len(b))
		}
	}
}

func TestEnqueueMaxMsgsCap(t *testing.T) {
	h, sink := testHandler(time.Hour, time.Hour) // only the count cap can fire
	for range coalesceMaxMsgs {
		h.enqueue(msg("5", "x")) // all fragments
	}
	select {
	case <-sink.done:
	case <-time.After(time.Second):
		t.Fatal("count cap did not flush the burst")
	}
	if len(sink.bursts[0]) != coalesceMaxMsgs {
		t.Fatalf("want %d messages at the cap, got %d", coalesceMaxMsgs, len(sink.bursts[0]))
	}
}

func TestFlushAllDrains(t *testing.T) {
	h, sink := testHandler(time.Hour, time.Hour)
	h.enqueue(msg("5", "buffered"))
	h.enqueue(msg("9", "also buffered"))
	if sink.count() != 0 {
		t.Fatal("should still be buffered")
	}
	h.FlushAll(context.Background())
	if sink.count() != 2 {
		t.Fatalf("FlushAll should drain both chats, got %d", sink.count())
	}
}
