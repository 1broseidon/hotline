package notify

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// captureSink records everything the dispatcher injects (scheduler_test recipe).
type captureSink struct {
	mu       sync.Mutex
	channels []capturedChannel
	err      error
	onSend   func(content string, meta map[string]string)
}

type capturedChannel struct {
	content string
	meta    map[string]string
}

func (c *captureSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.onSend != nil {
		c.onSend(content, meta)
	}
	c.channels = append(c.channels, capturedChannel{content: content, meta: meta})
	return c.err
}

func (c *captureSink) len() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return len(c.channels)
}

func (c *captureSink) at(i int) capturedChannel {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.channels[i]
}

// newTestDispatcher builds a Dispatcher over temp spool/sources paths with a
// fixed clock and UTC zone. accessFile is empty unless a test sets it.
func newTestDispatcher(t *testing.T, now time.Time, sources []string) *Dispatcher {
	t.Helper()
	dir := t.TempDir()
	d := NewDispatcher(
		filepath.Join(dir, "spool.json"),
		filepath.Join(dir, "sources.json"),
		"", sources, nil,
	)
	d.now = func() time.Time { return now }
	d.loc = time.UTC
	d.tick = 5 * time.Millisecond
	return d
}

func seedSpool(t *testing.T, d *Dispatcher, sp *SpoolDoc) {
	t.Helper()
	if err := SaveSpool(sp, d.spoolPath); err != nil {
		t.Fatal(err)
	}
}

func seedReg(t *testing.T, d *Dispatcher, reg *Registry) {
	t.Helper()
	if err := SaveRegistry(reg, d.sourcesPath); err != nil {
		t.Fatal(err)
	}
}

func readyEntry(id, label string, lvl Level) Entry {
	return Entry{ID: id, Label: label, Level: lvl, Message: "the payload", Hash: "h" + id,
		Status: statusReady, Count: 1, FirstAt: "2026-07-07T03:00:00Z", LastAt: "2026-07-07T03:00:00Z"}
}

func TestDispatchDeliversReadyContentAndMeta(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	seedReg(t, d, &Registry{Sources: []Source{{Label: "email-sentry", Key: "k", ChatID: "555"}}})
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{readyEntry("aaaaaa", "email-sentry", LevelUrgent)}})

	sink := &captureSink{}
	d.dispatch(context.Background(), sink)

	if sink.len() != 1 {
		t.Fatalf("want 1 injection, got %d", sink.len())
	}
	got := sink.at(0)
	if !strings.HasPrefix(got.content, `📟 Machine event from source "email-sentry" (level urgent`) {
		t.Errorf("content prefix wrong: %q", got.content)
	}
	if !strings.Contains(got.content, "untrusted data") || !strings.Contains(got.content, "silence is a valid outcome") {
		t.Errorf("content missing untrusted framing: %q", got.content)
	}
	if !strings.Contains(got.content, "--- report from email-sentry ---") || !strings.Contains(got.content, "the payload") {
		t.Errorf("content missing report block: %q", got.content)
	}
	for k, want := range map[string]string{
		"source": "telegram", "chat_id": "555", "kind": "notify",
		"notify_source": "email-sentry", "level": "urgent",
	} {
		if got.meta[k] != want {
			t.Errorf("meta[%q] = %q, want %q", k, got.meta[k], want)
		}
	}
	if got.meta["ts"] == "" {
		t.Error("meta missing ts")
	}
	// Delivered entry removed; delivered counter bumped.
	sp := loadSpoolT(t, d.spoolPath)
	if len(sp.Pending) != 0 {
		t.Errorf("delivered entry should be removed, got %+v", sp.Pending)
	}
	if sp.State["email-sentry"].Delivered != 1 {
		t.Errorf("delivered counter not bumped: %+v", sp.State["email-sentry"])
	}
}

// TestPersistBeforeInject asserts the entry is removed BEFORE the sink is called.
func TestPersistBeforeInject(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	seedReg(t, d, &Registry{Sources: []Source{{Label: "s", Key: "k"}}})
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{readyEntry("aaaaaa", "s", LevelNormal)}})

	var observed error
	sink := &captureSink{}
	sink.onSend = func(_ string, _ map[string]string) {
		// Re-entrant load must not deadlock and must see the post-remove state.
		sp, err := LoadSpool(d.spoolPath)
		if err != nil {
			observed = err
			return
		}
		if len(sp.Pending) != 0 {
			observed = errPending
		}
	}
	d.dispatch(context.Background(), sink)
	if observed != nil {
		t.Fatal(observed)
	}
	if sink.len() != 1 {
		t.Fatalf("want 1 injection, got %d", sink.len())
	}
}

var errPending = &dispatchTestErr{"entry still present at inject time"}

type dispatchTestErr struct{ s string }

func (e *dispatchTestErr) Error() string { return e.s }

func TestSinkErrorNotRetried(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	seedReg(t, d, &Registry{Sources: []Source{{Label: "s", Key: "k"}}})
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{readyEntry("aaaaaa", "s", LevelNormal)}})

	sink := &captureSink{err: errPending}
	d.dispatch(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatalf("want 1 attempted injection, got %d", sink.len())
	}
	// Entry already removed, so a second scan does not re-fire.
	d.dispatch(context.Background(), sink)
	if sink.len() != 1 {
		t.Errorf("failed injection re-fired: got %d attempts", sink.len())
	}
}

func TestQuietHoursHeldThenReleased(t *testing.T) {
	inWindow := time.Date(2026, 7, 7, 3, 0, 0, 0, time.UTC) // inside 23:00-08:00
	d := newTestDispatcher(t, inWindow, []string{"telegram"})
	seedReg(t, d, &Registry{QuietHours: "23:00-08:00", Sources: []Source{{Label: "s", Key: "k"}}})
	queued := readyEntry("aaaaaa", "s", LevelNormal)
	queued.Status = statusQueued
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{queued}})

	sink := &captureSink{}
	d.dispatch(context.Background(), sink)
	if sink.len() != 0 {
		t.Fatal("queued entry should not release inside quiet hours")
	}
	if loadSpoolT(t, d.spoolPath).Pending[0].Status != statusQueued {
		t.Error("held entry should remain queued")
	}

	// Move the clock past the window: it releases.
	d.now = func() time.Time { return time.Date(2026, 7, 7, 9, 0, 0, 0, time.UTC) }
	d.dispatch(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatalf("queued entry should release outside quiet hours, got %d", sink.len())
	}
	if loadSpoolT(t, d.spoolPath).Pending != nil && len(loadSpoolT(t, d.spoolPath).Pending) != 0 {
		t.Error("released entry should be removed")
	}
}

func TestQuietHoursDigest(t *testing.T) {
	outside := time.Date(2026, 7, 7, 9, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, outside, []string{"telegram"})
	seedReg(t, d, &Registry{QuietHours: "23:00-08:00", Sources: []Source{{Label: "a", Key: "ka"}, {Label: "b", Key: "kb"}}})
	e1 := readyEntry("aaaaaa", "a", LevelNormal)
	e1.Status = statusQueued
	e1.Message = "event A"
	e2 := readyEntry("bbbbbb", "b", LevelLow)
	e2.Status = statusQueued
	e2.Message = "event B"
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{e1, e2}})

	sink := &captureSink{}
	d.dispatch(context.Background(), sink)

	if sink.len() != 1 {
		t.Fatalf("two queued entries should release as ONE digest turn, got %d", sink.len())
	}
	got := sink.at(0)
	if got.meta["notify_source"] != "digest" || got.meta["count"] != "2" {
		t.Errorf("digest meta wrong: %+v", got.meta)
	}
	if !strings.Contains(got.content, "2 machine events held during quiet hours") {
		t.Errorf("digest header wrong: %q", got.content)
	}
	if !strings.Contains(got.content, "report from a (level normal") || !strings.Contains(got.content, "report from b (level low") {
		t.Errorf("digest missing per-source blocks: %q", got.content)
	}
	if !strings.Contains(got.content, "event A") || !strings.Contains(got.content, "event B") {
		t.Errorf("digest missing payloads: %q", got.content)
	}
}

func TestSuppressedRidesNextDelivery(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	seedReg(t, d, &Registry{Sources: []Source{{Label: "s", Key: "k"}}})
	seedSpool(t, d, &SpoolDoc{
		Pending: []Entry{readyEntry("aaaaaa", "s", LevelNormal)},
		State:   map[string]*SourceState{"s": {Suppressed: 12, SuppressedSince: "2026-07-07T08:59:00Z"}},
	})

	sink := &captureSink{}
	d.dispatch(context.Background(), sink)

	got := sink.at(0)
	if !strings.HasPrefix(got.meta["suppressed"], "12 since ") {
		t.Errorf("meta suppressed = %q, want '12 since …'", got.meta["suppressed"])
	}
	if !strings.Contains(got.content, "12 earlier events from this source were rate-limited") {
		t.Errorf("content missing suppression note: %q", got.content)
	}
	// Counter reset after it rode a delivery.
	if loadSpoolT(t, d.spoolPath).State["s"].Suppressed != 0 {
		t.Error("suppressed counter should reset after delivery")
	}
}

func TestResolveChatIDFallbacks(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)

	// 1. per-source chatId wins.
	d := newTestDispatcher(t, now, []string{"telegram"})
	reg := &Registry{DefaultChatID: "default", Sources: []Source{{Label: "s", Key: "k", ChatID: "persrc"}}}
	if got := d.resolveChatID(reg, "s"); got != "persrc" {
		t.Errorf("per-source chatId = %q, want persrc", got)
	}
	// 2. registry default when no per-source.
	reg.Sources[0].ChatID = ""
	if got := d.resolveChatID(reg, "s"); got != "default" {
		t.Errorf("default chatId = %q, want default", got)
	}
	// 3. access.json AllowFrom[0] when neither.
	reg.DefaultChatID = ""
	accessPath := filepath.Join(t.TempDir(), "access.json")
	if err := os.WriteFile(accessPath, []byte(`{"dmPolicy":"pairing","allowFrom":["999"]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	d.accessFile = accessPath
	if got := d.resolveChatID(reg, "s"); got != "999" {
		t.Errorf("access fallback chatId = %q, want 999", got)
	}
	// 4. empty when nothing resolves.
	d.accessFile = ""
	if got := d.resolveChatID(reg, "s"); got != "" {
		t.Errorf("no source of chat_id should yield empty, got %q", got)
	}
}

func TestRunEagerScanFires(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	seedReg(t, d, &Registry{Sources: []Source{{Label: "s", Key: "k"}}})
	seedSpool(t, d, &SpoolDoc{Pending: []Entry{readyEntry("aaaaaa", "s", LevelNormal)}})

	sink := &captureSink{}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = d.Run(ctx, sink) }()

	deadline := time.After(2 * time.Second)
	for sink.len() == 0 {
		select {
		case <-deadline:
			t.Fatal("eager scan did not inject")
		case <-time.After(2 * time.Millisecond):
		}
	}
}

func TestRunStopsOnCancel(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	d := newTestDispatcher(t, now, []string{"telegram"})
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- d.Run(ctx, &captureSink{}) }()
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("Run returned %v, want nil on cancel", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("Run did not return promptly on cancel")
	}
}
