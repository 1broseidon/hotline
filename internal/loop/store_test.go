package loop

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func tmpLoopPath(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "loops.json")
}

func mkLoop() Loop {
	return Loop{
		Label: "watch",
		Every: "30s",
		Cmd:   "echo hi",
	}
}

func TestLoadMissing(t *testing.T) {
	d, err := Load(tmpLoopPath(t))
	if err != nil {
		t.Fatal(err)
	}
	if d.Loops == nil || len(d.Loops) != 0 {
		t.Errorf("missing file should yield empty non-nil slice, got %+v", d.Loops)
	}
}

func TestLoadCorrupt(t *testing.T) {
	path := tmpLoopPath(t)
	if err := os.WriteFile(path, []byte("{broken"), 0o600); err != nil {
		t.Fatal(err)
	}
	d, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Loops) != 0 {
		t.Error("corrupt file should yield empty doc")
	}
	if _, err := os.Stat(path + ".corrupt"); err != nil {
		t.Errorf("corrupt file should be moved aside: %v", err)
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	path := tmpLoopPath(t)
	in := &Doc{Loops: []Loop{{Label: "a", Every: "1m", Cmd: "echo a", CreatedAt: "2026-07-08T12:00:00Z"}}}
	if err := Save(in, path); err != nil {
		t.Fatal(err)
	}
	out, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(out.Loops) != 1 || out.Loops[0].Label != "a" || out.Loops[0].Cmd != "echo a" {
		t.Errorf("round-trip mismatch: %+v", out.Loops)
	}
}

func TestAddValidatesAndFloorsEvery(t *testing.T) {
	path := tmpLoopPath(t)
	now := time.Date(2026, 7, 8, 12, 0, 0, 0, time.UTC)
	l := mkLoop()
	l.Every = "1s"
	stored, err := Add(path, l, now)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Every != "10s" {
		t.Errorf("Every = %q, want floor 10s", stored.Every)
	}
	if stored.Timeout != "2m0s" || stored.Sink != "notify" {
		t.Errorf("defaults not stored: %+v", stored)
	}
	if stored.CreatedAt != now.Format(time.RFC3339) {
		t.Errorf("CreatedAt = %q", stored.CreatedAt)
	}
	if _, err := Add(path, stored, now); err == nil {
		t.Error("duplicate label should error")
	}
}

func TestAddMaxLoops(t *testing.T) {
	path := tmpLoopPath(t)
	now := time.Now()
	for i := 0; i < maxLoops; i++ {
		l := mkLoop()
		l.Label = string(rune('a'+i/26)) + string(rune('a'+i%26))
		if _, err := Add(path, l, now); err != nil {
			t.Fatalf("add %d: %v", i, err)
		}
	}
	if _, err := Add(path, Loop{Label: "overflow", Every: "1m", Cmd: "true"}, now); err == nil {
		t.Errorf("adding beyond max (%d) should error", maxLoops)
	}
}

func TestRemoveSetPausedAndRecordRun(t *testing.T) {
	path := tmpLoopPath(t)
	stored, err := Add(path, mkLoop(), time.Now())
	if err != nil {
		t.Fatal(err)
	}
	paused, err := SetPaused(path, stored.Label, true)
	if err != nil {
		t.Fatal(err)
	}
	if !paused.Paused {
		t.Error("SetPaused(true) did not pause")
	}
	now := time.Date(2026, 7, 8, 13, 0, 0, 0, time.UTC)
	if err := RecordRun(path, stored.Label, now, 7, 1500*time.Millisecond); err != nil {
		t.Fatal(err)
	}
	d, _ := Load(path)
	got := d.Loops[0]
	if got.LastRunAt != now.Format(time.RFC3339) || got.LastExit != 7 || got.LastDurationMs != 1500 || got.Runs != 1 {
		t.Errorf("advisory status wrong: %+v", got)
	}
	if _, err := Remove(path, stored.Label); err != nil {
		t.Fatal(err)
	}
	if _, err := Remove(path, stored.Label); !errors.Is(err, ErrNotFound) {
		t.Errorf("remove missing: want ErrNotFound, got %v", err)
	}
}
