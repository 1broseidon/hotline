package notify

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// gateFixture returns a spool path, a rejects-log path, and a registry with one
// source (key "k-<label>") for the gate tests.
func gateFixture(t *testing.T, src Source, quietHours string) (spoolPath, rejectsPath string, reg *Registry) {
	t.Helper()
	dir := t.TempDir()
	if src.Key == "" {
		src.Key = "key-" + src.Label
	}
	reg = &Registry{QuietHours: quietHours, Sources: []Source{src}}
	return filepath.Join(dir, "spool.json"), filepath.Join(dir, "rejects.log"), reg
}

func loadSpoolT(t *testing.T, path string) *SpoolDoc {
	t.Helper()
	d, err := LoadSpool(path)
	if err != nil {
		t.Fatal(err)
	}
	return d
}

func TestEnqueueAccepted(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "email-sentry", LevelCap: LevelNormal}, "")

	out, err := Enqueue(sp, rj, reg, "key-email-sentry", LevelNormal, "backup ok", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Accepted || out.Clamped {
		t.Fatalf("out = %+v, want Accepted un-clamped", out)
	}
	d := loadSpoolT(t, sp)
	if len(d.Pending) != 1 {
		t.Fatalf("want 1 pending, got %d", len(d.Pending))
	}
	e := d.Pending[0]
	if e.Status != statusReady || e.Level != LevelNormal || e.Message != "backup ok" || e.Count != 1 {
		t.Errorf("entry wrong: %+v", e)
	}
	if d.State["email-sentry"].Tokens != defaultBurst-1 {
		t.Errorf("token not spent: %v", d.State["email-sentry"].Tokens)
	}
}

func TestEnqueueLevelClamp(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelNormal}, "")

	out, err := Enqueue(sp, rj, reg, "key-s", LevelUrgent, "escalate me", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Accepted || !out.Clamped || out.Level != LevelNormal || out.ClampedTo != LevelNormal {
		t.Fatalf("out = %+v, want clamped to normal", out)
	}
	e := loadSpoolT(t, sp).Pending[0]
	if e.Level != LevelNormal || !e.Clamped {
		t.Errorf("stored entry not clamped: %+v", e)
	}
}

func TestEnqueueDedupCoalesces(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelNormal}, "")

	if _, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, "same line", now); err != nil {
		t.Fatal(err)
	}
	out, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, "same line", now.Add(time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Duplicate || out.Count != 2 {
		t.Fatalf("second identical = %+v, want Duplicate count 2", out)
	}
	d := loadSpoolT(t, sp)
	if len(d.Pending) != 1 || d.Pending[0].Count != 2 {
		t.Errorf("duplicate should coalesce into one entry: %+v", d.Pending)
	}
	// The dedup did not spend a second token.
	if d.State["s"].Tokens != defaultBurst-1 {
		t.Errorf("dedup spent a token: %v", d.State["s"].Tokens)
	}

	// Outside the window, the identical message is a fresh event again.
	out, err = Enqueue(sp, rj, reg, "key-s", LevelNormal, "same line", now.Add(11*time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Accepted {
		t.Errorf("identical message past the window should be a new event, got %+v", out)
	}
	if len(loadSpoolT(t, sp).Pending) != 2 {
		t.Error("post-window identical should append a second entry")
	}
}

func TestEnqueueRateLimit(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "flood", LevelCap: LevelNormal}, "")

	// Burst 5: five distinct messages at the same instant all land.
	for i := 0; i < defaultBurst; i++ {
		out, err := Enqueue(sp, rj, reg, "key-flood", LevelNormal, fmt.Sprintf("event %d", i), now)
		if err != nil {
			t.Fatal(err)
		}
		if out.Status != Accepted {
			t.Fatalf("burst event %d = %+v, want Accepted", i, out)
		}
	}
	// The sixth is suppressed.
	out, err := Enqueue(sp, rj, reg, "key-flood", LevelNormal, "event over", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != RejectedRate || out.Suppressed != 1 {
		t.Fatalf("sixth = %+v, want RejectedRate suppressed 1", out)
	}
	// Suppressed but not enqueued.
	if len(loadSpoolT(t, sp).Pending) != defaultBurst {
		t.Errorf("suppressed event should not enqueue")
	}

	// After a refill interval, one more token is available.
	out, err = Enqueue(sp, rj, reg, "key-flood", LevelNormal, "event refilled", now.Add(defaultRefillMins*time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Accepted {
		t.Errorf("after refill = %+v, want Accepted", out)
	}
}

func TestEnqueueUnknownKeyLogged(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelNormal}, "")

	out, err := Enqueue(sp, rj, reg, "9f2c6a1e-dead-beef-cafe-2d5b8e1f4a6c", LevelNormal, "secret payload text", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != RejectedUnknown {
		t.Fatalf("out = %+v, want RejectedUnknown", out)
	}
	// Nothing enqueued.
	if len(loadSpoolT(t, sp).Pending) != 0 {
		t.Error("unknown key should not enqueue")
	}
	// A rejects.log line was written; it carries an 8-char key prefix and byte
	// count but NOT the message content.
	raw, err := os.ReadFile(rj)
	if err != nil {
		t.Fatalf("rejects.log not written: %v", err)
	}
	line := string(raw)
	if !strings.Contains(line, "key=9f2c6a1e") || !strings.Contains(line, "msg-bytes=") {
		t.Errorf("rejects line malformed: %q", line)
	}
	if strings.Contains(line, "secret payload text") {
		t.Errorf("rejects line must not log message content: %q", line)
	}
}

func TestEnqueueQuietHours(t *testing.T) {
	// 03:00 UTC is inside 23:00-08:00.
	now := time.Date(2026, 7, 7, 3, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelUrgent}, "23:00-08:00")

	// normal → queued.
	out, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, "nightly report", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Queued || out.QueuedUntil != "08:00" {
		t.Fatalf("normal in quiet hours = %+v, want Queued until 08:00", out)
	}
	if loadSpoolT(t, sp).Pending[0].Status != statusQueued {
		t.Error("entry should be stored queued")
	}

	// urgent → passes through as ready.
	out, err = Enqueue(sp, rj, reg, "key-s", LevelUrgent, "urgent thing", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != Accepted {
		t.Fatalf("urgent in quiet hours = %+v, want Accepted", out)
	}
	d := loadSpoolT(t, sp)
	if d.Pending[1].Status != statusReady {
		t.Error("urgent entry should be ready during quiet hours")
	}
}

func TestEnqueueInvalidQuietHoursErrors(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelNormal}, "not-a-window")
	if _, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, "x", now); err == nil {
		t.Error("invalid quiet-hours config should fail the call (exit 1)")
	}
}

func TestEnqueueSpoolFull(t *testing.T) {
	now := time.Date(2026, 7, 7, 12, 0, 0, 0, time.UTC)
	// High burst so the rate limiter never fires before the spool fills.
	sp, rj, reg := gateFixture(t, Source{Label: "s", LevelCap: LevelNormal, Rate: Rate{Burst: 1000, RefillMins: 1}}, "")

	for i := 0; i < maxPending; i++ {
		out, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, fmt.Sprintf("msg %d", i), now)
		if err != nil {
			t.Fatal(err)
		}
		if out.Status != Accepted {
			t.Fatalf("fill %d = %+v, want Accepted", i, out)
		}
	}
	out, err := Enqueue(sp, rj, reg, "key-s", LevelNormal, "one too many", now)
	if err != nil {
		t.Fatal(err)
	}
	if out.Status != RejectedSpoolFull {
		t.Errorf("over-capacity = %+v, want RejectedSpoolFull", out)
	}
}
