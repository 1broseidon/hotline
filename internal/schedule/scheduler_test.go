package schedule

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

// captureSink records everything the scheduler injects.
type captureSink struct {
	mu       sync.Mutex
	channels []capturedChannel
	err      error // returned by SendChannel when set
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

// newTestScheduler builds a Scheduler with a fixed clock, UTC zone, and a short
// tick, over schedules.json in a temp dir.
func newTestScheduler(t *testing.T, now time.Time, sources []string) *Scheduler {
	t.Helper()
	s := NewScheduler(tmpPath(t), sources, nil)
	s.now = func() time.Time { return now }
	s.loc = time.UTC
	s.tick = 5 * time.Millisecond
	return s
}

// seed writes the schedules directly to the scheduler's store.
func seed(t *testing.T, s *Scheduler, schedules ...Schedule) {
	t.Helper()
	if err := Save(&Doc{Schedules: schedules}, s.path); err != nil {
		t.Fatal(err)
	}
}

func dueDaily(id, nextFire string) Schedule {
	return Schedule{
		ID: id, Prompt: "check the deploy", Source: "telegram", ChatID: "123",
		Recurrence: Recurrence{Kind: KindDaily, TimeOfDay: "09:00"}, NextFire: nextFire,
	}
}

func TestFireDueDeliversContentAndMeta(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s, dueDaily("aaaaaa", "2026-07-06T09:00:00Z")) // due (past)

	sink := &captureSink{}
	s.fireDue(context.Background(), sink)

	if sink.len() != 1 {
		t.Fatalf("want 1 fire, got %d", sink.len())
	}
	got := sink.at(0)
	if !strings.HasPrefix(got.content, "⏰ Scheduled task aaaaaa fired (daily at 09:00).") {
		t.Errorf("content prefix wrong: %q", got.content)
	}
	if !strings.Contains(got.content, "check the deploy") {
		t.Errorf("content missing prompt: %q", got.content)
	}
	for k, want := range map[string]string{
		"source": "telegram", "chat_id": "123", "kind": "schedule", "schedule_id": "aaaaaa",
	} {
		if got.meta[k] != want {
			t.Errorf("meta[%q] = %q, want %q", k, got.meta[k], want)
		}
	}
	if got.meta["ts"] == "" {
		t.Error("meta missing ts")
	}
	// Recurring: advanced to the future, not deleted.
	d, _ := Load(s.path)
	if len(d.Schedules) != 1 {
		t.Fatalf("recurring schedule should survive, got %d", len(d.Schedules))
	}
	nf, _ := time.Parse(time.RFC3339, d.Schedules[0].NextFire)
	if !nf.After(now) {
		t.Errorf("NextFire not advanced past now: %v", nf)
	}
	if d.Schedules[0].LastFired == "" {
		t.Error("LastFired not recorded")
	}
}

func TestFireDueDeletesOnce(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s, Schedule{
		ID: "bbbbbb", Prompt: "one shot", Source: "telegram", ChatID: "1",
		Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-06T09:00:00Z",
	})
	sink := &captureSink{}
	s.fireDue(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatalf("want 1 fire, got %d", sink.len())
	}
	d, _ := Load(s.path)
	if len(d.Schedules) != 0 {
		t.Errorf("once should be deleted after firing, got %+v", d.Schedules)
	}
}

// TestPersistBeforeInject asserts the store is advanced/cleaned BEFORE the sink
// is called: the sink itself reads the store during SendChannel.
func TestPersistBeforeInject(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s,
		dueDaily("aaaaaa", "2026-07-06T09:00:00Z"),
		Schedule{ID: "bbbbbb", Prompt: "one", Source: "telegram", ChatID: "1",
			Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-06T09:00:00Z"},
	)
	var observed error
	sink := &captureSink{}
	sink.onSend = func(_ string, meta map[string]string) {
		// Re-entrant Load must NOT deadlock (lock released before inject) and must
		// see the already-persisted post-fire state.
		d, err := Load(s.path)
		if err != nil {
			observed = err
			return
		}
		switch meta["schedule_id"] {
		case "aaaaaa":
			for _, sc := range d.Schedules {
				if sc.ID == "aaaaaa" {
					nf, _ := time.Parse(time.RFC3339, sc.NextFire)
					if !nf.After(now) {
						observed = fmt.Errorf("daily NextFire not advanced at inject time: %v", nf)
					}
				}
			}
		case "bbbbbb":
			for _, sc := range d.Schedules {
				if sc.ID == "bbbbbb" {
					observed = fmt.Errorf("once still present at inject time")
				}
			}
		}
	}
	s.fireDue(context.Background(), sink)
	if observed != nil {
		t.Fatal(observed)
	}
	if sink.len() != 2 {
		t.Fatalf("want 2 fires, got %d", sink.len())
	}
}

// TestSinkErrorNotRetried: a failing SendChannel does not re-fire next scan
// (NextFire is already advanced), and the error is swallowed.
func TestSinkErrorNotRetried(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s, dueDaily("aaaaaa", "2026-07-06T09:00:00Z"))

	sink := &captureSink{err: fmt.Errorf("delivery down")}
	s.fireDue(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatalf("want 1 attempted fire, got %d", sink.len())
	}
	// Second scan at the same now: NextFire already advanced to tomorrow, so no
	// re-fire.
	s.fireDue(context.Background(), sink)
	if sink.len() != 1 {
		t.Errorf("failed fire re-fired on next scan: got %d attempts", sink.len())
	}
}

func TestPausedSkippedNotAdvanced(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	sc := dueDaily("aaaaaa", "2026-07-06T09:00:00Z")
	sc.Paused = true
	seed(t, s, sc)

	sink := &captureSink{}
	s.fireDue(context.Background(), sink)
	if sink.len() != 0 {
		t.Errorf("paused schedule should not fire")
	}
	d, _ := Load(s.path)
	if d.Schedules[0].NextFire != "2026-07-06T09:00:00Z" {
		t.Errorf("paused NextFire changed: %q", d.Schedules[0].NextFire)
	}
}

func TestInvalidAutoPausedNoSpam(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s, Schedule{
		ID: "aaaaaa", Prompt: "p", Source: "telegram", ChatID: "1",
		Recurrence: Recurrence{Kind: KindDaily, TimeOfDay: "09:00"},
		NextFire:   "garbage-not-a-time",
	})
	sink := &captureSink{}
	s.fireDue(context.Background(), sink)
	if sink.len() != 0 {
		t.Errorf("invalid schedule should not fire")
	}
	d, _ := Load(s.path)
	if !d.Schedules[0].Paused {
		t.Error("invalid schedule should be auto-paused")
	}
	// A second scan does not re-fire or unpause (no spam).
	s.fireDue(context.Background(), sink)
	if sink.len() != 0 {
		t.Error("auto-paused schedule should stay quiet on subsequent scans")
	}
}

func TestCatchUpFiresOnce(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	// Daily due 3 days ago.
	seed(t, s, dueDaily("aaaaaa", "2026-07-03T09:00:00Z"))

	sink := &captureSink{}
	s.fireDue(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatalf("catch-up should fire exactly once, got %d", sink.len())
	}
	d, _ := Load(s.path)
	nf, _ := time.Parse(time.RFC3339, d.Schedules[0].NextFire)
	if !nf.After(now) {
		t.Errorf("catch-up NextFire not in the future: %v", nf)
	}
}

func TestSourceFallback(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	// Stored source "discord" not in the single configured source.
	s := newTestScheduler(t, now, []string{"telegram"})
	sc := dueDaily("aaaaaa", "2026-07-06T09:00:00Z")
	sc.Source = "discord"
	seed(t, s, sc)

	sink := &captureSink{}
	s.fireDue(context.Background(), sink)
	if sink.len() != 1 {
		t.Fatal("want 1 fire")
	}
	if got := sink.at(0).meta["source"]; got != "telegram" {
		t.Errorf("source fallback: meta[source] = %q, want telegram", got)
	}
}

func TestRunStopsOnCancel(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- s.Run(ctx, &captureSink{}) }()
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

func TestRunEagerScanFires(t *testing.T) {
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	s := newTestScheduler(t, now, []string{"telegram"})
	seed(t, s, dueDaily("aaaaaa", "2026-07-06T09:00:00Z"))

	sink := &captureSink{}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = s.Run(ctx, sink) }()

	deadline := time.After(2 * time.Second)
	for sink.len() == 0 {
		select {
		case <-deadline:
			t.Fatal("eager scan did not fire")
		case <-time.After(2 * time.Millisecond):
		}
	}
}
