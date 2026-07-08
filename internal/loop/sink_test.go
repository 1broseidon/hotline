package loop

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/notify"
)

func notifyTestSource(root, label string) (notify.Source, error) {
	return notify.AddSource(notify.SourcesPath(root), label, notify.LevelNormal, notify.Rate{}, "", time.Now())
}

func TestNotifySinkEmptyStdoutNoEnqueue(t *testing.T) {
	root := t.TempDir()
	src, err := notifyTestSource(root, "watch")
	if err != nil {
		t.Fatal(err)
	}
	s := notifySink{
		spoolPath:   notify.SpoolPath(root),
		sourcesPath: notify.SourcesPath(root),
		rejectsPath: filepath.Join(root, "notify", "rejects.log"),
		now:         func() time.Time { return time.Date(2026, 7, 8, 12, 0, 0, 0, time.UTC) },
	}
	if err := s.Route(context.Background(), Loop{Label: "l", Source: src.Label, Level: "low"}, " \n\t", 0); err != nil {
		t.Fatal(err)
	}
	sp, err := notify.LoadSpool(notify.SpoolPath(root))
	if err != nil {
		t.Fatal(err)
	}
	if len(sp.Pending) != 0 {
		t.Errorf("empty stdout enqueued: %+v", sp.Pending)
	}
}

func TestNotifySinkRoutesThroughEnqueue(t *testing.T) {
	root := t.TempDir()
	src, err := notifyTestSource(root, "watch")
	if err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 7, 8, 12, 0, 0, 0, time.UTC)
	s := notifySink{
		spoolPath:   notify.SpoolPath(root),
		sourcesPath: notify.SourcesPath(root),
		rejectsPath: notify.RejectsPath(root),
		now:         func() time.Time { return now },
	}
	if err := s.Route(context.Background(), Loop{Label: "l", Source: src.Label, Level: "low"}, "new reddit hit", 0); err != nil {
		t.Fatal(err)
	}
	sp, err := notify.LoadSpool(notify.SpoolPath(root))
	if err != nil {
		t.Fatal(err)
	}
	if len(sp.Pending) != 1 {
		t.Fatalf("pending = %d, want 1", len(sp.Pending))
	}
	got := sp.Pending[0]
	if got.Label != src.Label || got.Level != notify.LevelLow || got.Message != "new reddit hit" {
		t.Errorf("pending wrong: %+v", got)
	}
}
