package catchup

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

type capture struct {
	content string
	meta    map[string]string
}

type recSink struct {
	got []capture
	err error
}

func (s *recSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	if s.err != nil {
		return s.err
	}
	s.got = append(s.got, capture{content: content, meta: meta})
	return nil
}

// delivered is a shorthand for the meta high-water record the live delivery
// path journals when SendChannel hands an inbound to a running harness.
func delivered(id string) transcript.Record {
	return transcript.Record{Dir: transcript.DirMeta, Kind: transcript.KindDelivered, MessageID: id}
}

// writeTranscript writes records via the real logger so TS formatting matches
// production exactly. now advances so records land in chronological order.
func writeTranscript(t *testing.T, path string, recs []transcript.Record) {
	t.Helper()
	l := transcript.New(path)
	base := time.Date(2026, 7, 14, 4, 0, 0, 0, time.UTC)
	for i, r := range recs {
		if r.TS == "" {
			r.TS = base.Add(time.Duration(i) * time.Minute).Format(tsFormat)
		}
		if err := l.Append(r); err != nil {
			t.Fatal(err)
		}
	}
}

func TestReplayUnansweredAfterLastDelivered(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "first"},
		delivered("u-1"), // u-1 reached a live harness — answered.
		{Dir: "out", ChatID: "dev-1", Kind: "reply", Text: "answered first"},
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-2", Text: "Send me a quick status update"},
	})

	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 || len(sink.got) != 1 {
		t.Fatalf("want 1 replay, got n=%d delivered=%d", n, len(sink.got))
	}
	c := sink.got[0]
	if !strings.Contains(c.content, "status update") {
		t.Errorf("replayed content missing original text: %q", c.content)
	}
	if !strings.HasPrefix(c.content, "[catch-up") {
		t.Errorf("replayed content missing catch-up marker: %q", c.content)
	}
	if c.meta["chat_id"] != "dev-1" || c.meta["message_id"] != "u-2" || c.meta["kind"] != "text" {
		t.Errorf("meta not reconstructed: %+v", c.meta)
	}
}

func TestReplayNothingWhenDelivered(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "hi"},
		delivered("u-1"),
		{Dir: "out", ChatID: "dev-1", Kind: "reply", Text: "yo"},
	})
	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 0 || len(sink.got) != 0 {
		t.Fatalf("want 0 replays when the turn was delivered, got %d", n)
	}
}

// TestOutboundDoesNotFalseAck is the B-1 regression: an inbound the harness
// never saw is followed by a notify (skipped) and its OUTBOUND reply. The old
// "any out acks all prior inbound" logic would drop the lost user turn here; the
// delivered-marker logic MUST still replay it.
func TestOutboundDoesNotFalseAck(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "status update please"},
		// George's email-triage watcher fires (skip kind) and the agent answers it:
		{Dir: "in", ChatID: "dev-1", Kind: "notify", Text: "important email arrived"},
		{Dir: "out", ChatID: "dev-1", Kind: "reply", Text: "flagged that email"},
		// A background job also writes an outbound record overnight:
		{Dir: "out", ChatID: "dev-1", Kind: "job", MessageID: "j-1", Text: "job done: loop"},
	})
	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 10, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 || len(sink.got) != 1 || !strings.Contains(sink.got[0].content, "status update please") {
		t.Fatalf("outbound false-acked the lost user turn: n=%d %+v", n, sink.got)
	}
}

// TestLiveDeliveryStopsReplay proves the delivered marker journaled once the
// harness comes back and consumes the turn stops further replay.
func TestLiveDeliveryStopsReplay(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "status?"},
	})
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)

	// First (re)start: the unanswered turn replays.
	s1 := &recSink{}
	n1, err := ReplayUnanswered(context.Background(), s1, transcript.New(path), path, now, DefaultWindow)
	if err != nil || n1 != 1 {
		t.Fatalf("first replay: n=%d err=%v", n1, err)
	}

	// The harness finally receives it live — a delivered marker lands.
	writeTranscript(t, path, []transcript.Record{delivered("u-1")})

	// Second (re)start: nothing to replay — the delivered marker acked the turn.
	s2 := &recSink{}
	n2, err := ReplayUnanswered(context.Background(), s2, transcript.New(path), path, now.Add(2*time.Minute), DefaultWindow)
	if err != nil || n2 != 0 {
		t.Fatalf("second replay must be empty after delivered marker, got n=%d err=%v", n2, err)
	}
}

// TestReplayAttemptCap is the S-1 regression: a turn the harness never answers
// (no delivered marker) is replayed at most maxReplayAttempts times across
// restarts, never crash-looping.
func TestReplayAttemptCap(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "poison turn"},
	})
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	log := transcript.New(path)

	total := 0
	// Three restarts with no answer (no delivered marker ever lands).
	for i := 0; i < 3; i++ {
		s := &recSink{}
		n, err := ReplayUnanswered(context.Background(), s, log, path, now.Add(time.Duration(i)*time.Minute), DefaultWindow)
		if err != nil {
			t.Fatal(err)
		}
		total += n
	}
	if total != maxReplayAttempts {
		t.Fatalf("want exactly %d replays across 3 restarts, got %d", maxReplayAttempts, total)
	}
}

func TestReplaySkipsAncientAndFires(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	old := time.Date(2026, 7, 13, 0, 0, 0, 0, time.UTC) // ~28h before now
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", TS: old.Format(tsFormat), Text: "ancient"},
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-2", Text: "recent"},
	})
	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 || len(sink.got) != 1 || !strings.Contains(sink.got[0].content, "recent") {
		t.Fatalf("want only the recent turn, got n=%d %+v", n, sink.got)
	}
}

func TestReplaySkipsScheduleAndNotify(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "c", Kind: "schedule", Text: "cron fire"},
		{Dir: "in", ChatID: "c", Kind: "notify", Text: "watcher fire"},
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-9", Text: "real message"},
	})
	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 || !strings.Contains(sink.got[0].content, "real message") {
		t.Fatalf("want only the user turn replayed, got n=%d %+v", n, sink.got)
	}
}

func TestReplayMissingTranscriptIsNoOp(t *testing.T) {
	sink := &recSink{}
	n, err := ReplayUnanswered(context.Background(), sink, nil, filepath.Join(t.TempDir(), "nope.jsonl"), time.Now(), DefaultWindow)
	if err != nil || n != 0 {
		t.Fatalf("missing transcript should be a no-op, got n=%d err=%v", n, err)
	}
}

func TestReplayInOrder(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "one"},
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-2", Text: "two"},
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-3", Text: "three"},
	})
	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 3 {
		t.Fatalf("want 3, got %d", n)
	}
	for i, want := range []string{"one", "two", "three"} {
		if !strings.Contains(sink.got[i].content, want) {
			t.Errorf("order wrong at %d: %q", i, sink.got[i].content)
		}
	}
}

func TestReplayToleratesCorruptTail(t *testing.T) {
	path := filepath.Join(t.TempDir(), "transcript.jsonl")
	writeTranscript(t, path, []transcript.Record{
		{Dir: "in", ChatID: "dev-1", Kind: "text", MessageID: "u-1", Text: "good"},
	})
	// Append a torn line (a crash mid-write).
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = f.WriteString(`{"dir":"in","text":"tru`)
	f.Close()

	sink := &recSink{}
	now := time.Date(2026, 7, 14, 4, 5, 0, 0, time.UTC)
	n, err := ReplayUnanswered(context.Background(), sink, nil, path, now, DefaultWindow)
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 || !strings.Contains(sink.got[0].content, "good") {
		t.Fatalf("want the one good record, got n=%d %+v", n, sink.got)
	}
}
