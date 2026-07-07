package notify

import (
	"os"
	"path/filepath"
	"sync"
	"testing"
)

func spoolTmp(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "spool.json")
}

func TestLoadSpoolMissing(t *testing.T) {
	d, err := LoadSpool(spoolTmp(t))
	if err != nil {
		t.Fatal(err)
	}
	if d.Pending == nil || len(d.Pending) != 0 || d.State == nil {
		t.Errorf("missing file should yield empty non-nil doc, got %+v", d)
	}
}

func TestLoadSpoolCorrupt(t *testing.T) {
	path := spoolTmp(t)
	if err := os.WriteFile(path, []byte("{broken"), 0o600); err != nil {
		t.Fatal(err)
	}
	d, err := LoadSpool(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Pending) != 0 {
		t.Error("corrupt file should yield empty doc")
	}
	if _, err := os.Stat(path + ".corrupt"); err != nil {
		t.Errorf("corrupt file should be moved aside: %v", err)
	}
}

func TestSpoolSaveLoadRoundTrip(t *testing.T) {
	path := spoolTmp(t)
	in := &SpoolDoc{
		Pending: []Entry{{ID: "abc123", Label: "s", Level: LevelNormal, Message: "hi", Hash: "h", Status: statusReady, Count: 1}},
		State:   map[string]*SourceState{"s": {Tokens: 3.5, Delivered: 2}},
	}
	if err := SaveSpool(in, path); err != nil {
		t.Fatal(err)
	}
	out, err := LoadSpool(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(out.Pending) != 1 || out.Pending[0].ID != "abc123" || out.Pending[0].Message != "hi" {
		t.Errorf("pending round-trip mismatch: %+v", out.Pending)
	}
	if out.State["s"].Tokens != 3.5 || out.State["s"].Delivered != 2 {
		t.Errorf("state round-trip mismatch: %+v", out.State["s"])
	}
}

// TestConcurrentMutateSpoolNoLostUpdates hammers the flock: N goroutines each
// append a distinct entry; all must survive.
func TestConcurrentMutateSpoolNoLostUpdates(t *testing.T) {
	path := spoolTmp(t)
	const n = 20
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			err := MutateSpool(path, func(d *SpoolDoc) error {
				d.Pending = append(d.Pending, Entry{ID: string(rune('A' + i)), Status: statusReady})
				return nil
			})
			if err != nil {
				t.Errorf("concurrent mutate %d: %v", i, err)
			}
		}(i)
	}
	wg.Wait()
	d, _ := LoadSpool(path)
	if len(d.Pending) != n {
		t.Errorf("concurrent mutates lost updates: got %d, want %d", len(d.Pending), n)
	}
}

func TestNewEntryIDUnique(t *testing.T) {
	d := &SpoolDoc{}
	d.normalize()
	seen := map[string]bool{}
	for i := 0; i < 100; i++ {
		id := newEntryID(d)
		if len(id) != 6 {
			t.Fatalf("id should be 6 hex chars, got %q", id)
		}
		if seen[id] {
			t.Fatalf("duplicate id %q", id)
		}
		seen[id] = true
		d.Pending = append(d.Pending, Entry{ID: id})
	}
}
