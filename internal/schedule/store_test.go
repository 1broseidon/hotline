package schedule

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func tmpPath(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "schedules.json")
}

func mkSchedule() Schedule {
	return Schedule{
		Prompt:     "check the deploy",
		Source:     "telegram",
		ChatID:     "123",
		CreatedBy:  "agent",
		Recurrence: Recurrence{Kind: KindDaily, TimeOfDay: "09:00"},
		NextFire:   time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
	}
}

func TestLoadMissing(t *testing.T) {
	d, err := Load(tmpPath(t))
	if err != nil {
		t.Fatal(err)
	}
	if d.Schedules == nil || len(d.Schedules) != 0 {
		t.Errorf("missing file should yield empty non-nil slice, got %+v", d.Schedules)
	}
}

func TestLoadCorrupt(t *testing.T) {
	path := tmpPath(t)
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	d, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Schedules) != 0 {
		t.Error("corrupt file should yield empty doc")
	}
	if _, err := os.Stat(path + ".corrupt"); err != nil {
		t.Errorf("corrupt file should be moved aside: %v", err)
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	path := tmpPath(t)
	in := &Doc{Schedules: []Schedule{{ID: "abc123", Prompt: "hi", ChatID: "1", Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-08T09:00:00Z"}}}
	if err := Save(in, path); err != nil {
		t.Fatal(err)
	}
	out, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(out.Schedules) != 1 || out.Schedules[0].ID != "abc123" || out.Schedules[0].Prompt != "hi" {
		t.Errorf("round-trip mismatch: %+v", out.Schedules)
	}
}

func TestAddAssignsIDAndCreatedAt(t *testing.T) {
	path := tmpPath(t)
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, time.UTC)
	stored, err := Add(path, mkSchedule(), now)
	if err != nil {
		t.Fatal(err)
	}
	if len(stored.ID) != 6 {
		t.Errorf("ID should be 6 hex chars, got %q", stored.ID)
	}
	if stored.CreatedAt != now.Format(time.RFC3339) {
		t.Errorf("CreatedAt = %q, want %q", stored.CreatedAt, now.Format(time.RFC3339))
	}
	d, _ := Load(path)
	if len(d.Schedules) != 1 {
		t.Fatalf("want 1 persisted schedule, got %d", len(d.Schedules))
	}
}

func TestAddValidation(t *testing.T) {
	path := tmpPath(t)
	now := time.Now()

	noPrompt := mkSchedule()
	noPrompt.Prompt = "  "
	if _, err := Add(path, noPrompt, now); err == nil {
		t.Error("empty prompt should error")
	}
	longPrompt := mkSchedule()
	longPrompt.Prompt = strings.Repeat("x", maxPromptLen+1)
	if _, err := Add(path, longPrompt, now); err == nil {
		t.Error("over-length prompt should error")
	}
	noChat := mkSchedule()
	noChat.ChatID = ""
	if _, err := Add(path, noChat, now); err == nil {
		t.Error("empty chat_id should error")
	}
	badRec := mkSchedule()
	badRec.Recurrence = Recurrence{Kind: "cron"}
	if _, err := Add(path, badRec, now); err == nil {
		t.Error("invalid recurrence should error")
	}
	badNext := mkSchedule()
	badNext.NextFire = "not-a-time"
	if _, err := Add(path, badNext, now); err == nil {
		t.Error("invalid nextFire should error")
	}
}

func TestAddMaxSchedules(t *testing.T) {
	path := tmpPath(t)
	now := time.Now()
	for i := 0; i < maxSchedules; i++ {
		if _, err := Add(path, mkSchedule(), now); err != nil {
			t.Fatalf("add %d: %v", i, err)
		}
	}
	if _, err := Add(path, mkSchedule(), now); err == nil {
		t.Errorf("adding beyond max (%d) should error", maxSchedules)
	}
}

func TestNewIDUniqueness(t *testing.T) {
	// Force a collision: taken returns true for the first N candidates via a
	// counter, ensuring the loop keeps trying.
	calls := 0
	got := newID(func(string) bool {
		calls++
		return calls <= 3 // reject the first three ids
	})
	if got == "" || calls < 4 {
		t.Errorf("newID should retry on collision; calls=%d id=%q", calls, got)
	}
}

func TestRemove(t *testing.T) {
	path := tmpPath(t)
	now := time.Now()
	a, _ := Add(path, mkSchedule(), now)
	b, _ := Add(path, mkSchedule(), now)

	// Exact id.
	if _, err := Remove(path, a.ID); err != nil {
		t.Fatalf("remove exact: %v", err)
	}
	d, _ := Load(path)
	if len(d.Schedules) != 1 || d.Schedules[0].ID != b.ID {
		t.Errorf("after remove: %+v", d.Schedules)
	}
	// Unique prefix.
	if _, err := Remove(path, b.ID[:3]); err != nil {
		t.Fatalf("remove prefix: %v", err)
	}
	// Not found.
	if _, err := Remove(path, "zzzzzz"); !errors.Is(err, ErrNotFound) {
		t.Errorf("remove missing: want ErrNotFound, got %v", err)
	}
}

func TestRemoveAmbiguous(t *testing.T) {
	path := tmpPath(t)
	// Seed two schedules with a shared prefix directly.
	d := &Doc{Schedules: []Schedule{
		{ID: "aa0001", Prompt: "p", ChatID: "1", Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-08T09:00:00Z"},
		{ID: "aa0002", Prompt: "p", ChatID: "1", Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-08T09:00:00Z"},
	}}
	if err := Save(d, path); err != nil {
		t.Fatal(err)
	}
	_, err := Remove(path, "aa")
	if err == nil || !strings.Contains(err.Error(), "ambiguous") {
		t.Errorf("ambiguous prefix should error with 'ambiguous', got %v", err)
	}
}

func TestSetPausedResumeRecompute(t *testing.T) {
	path := tmpPath(t)
	loc := time.UTC
	now := time.Date(2026, 7, 6, 12, 0, 0, 0, loc)

	// aa0001: a daily with a stale (past) NextFire, paused.
	// bb0001: a once, paused, with a past NextFire.
	d := &Doc{Schedules: []Schedule{
		{
			ID: "aa0001", Prompt: "p", ChatID: "1", Paused: true,
			Recurrence: Recurrence{Kind: KindDaily, TimeOfDay: "09:00"},
			NextFire:   "2026-07-01T09:00:00Z", // stale
		},
		{
			ID: "bb0001", Prompt: "p", ChatID: "1", Paused: true,
			Recurrence: Recurrence{Kind: KindOnce}, NextFire: "2026-07-01T09:00:00Z",
		},
	}}
	if err := Save(d, path); err != nil {
		t.Fatal(err)
	}
	updated, err := SetPaused(path, "aa0001", false, now, loc)
	if err != nil {
		t.Fatal(err)
	}
	nf, _ := time.Parse(time.RFC3339, updated.NextFire)
	if !nf.After(now) {
		t.Errorf("resume recurring should recompute NextFire to future, got %v", nf)
	}

	// A once keeps its stored time on resume.
	once, err := SetPaused(path, "bb0001", false, now, loc)
	if err != nil {
		t.Fatal(err)
	}
	if once.NextFire != "2026-07-01T09:00:00Z" {
		t.Errorf("resume once should keep stored time, got %q", once.NextFire)
	}

	// Pausing never touches NextFire.
	paused, err := SetPaused(path, "aa0001", true, now, loc)
	if err != nil {
		t.Fatal(err)
	}
	if !paused.Paused || paused.NextFire != updated.NextFire {
		t.Errorf("pause changed NextFire: %q -> %q", updated.NextFire, paused.NextFire)
	}
}

func TestConcurrentMutateNoLostUpdates(t *testing.T) {
	path := tmpPath(t)
	now := time.Now()
	const n = 20
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := Add(path, mkSchedule(), now); err != nil {
				t.Errorf("concurrent add: %v", err)
			}
		}()
	}
	wg.Wait()
	d, _ := Load(path)
	if len(d.Schedules) != n {
		t.Errorf("concurrent adds lost updates: got %d, want %d", len(d.Schedules), n)
	}
	// All ids unique.
	seen := map[string]bool{}
	for _, s := range d.Schedules {
		if seen[s.ID] {
			t.Errorf("duplicate id %q", s.ID)
		}
		seen[s.ID] = true
	}
}
