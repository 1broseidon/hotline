package notify

import (
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"testing"
	"time"
)

func sourcesTmp(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "sources.json")
}

func TestLoadRegistryMissing(t *testing.T) {
	r, err := LoadRegistry(sourcesTmp(t))
	if err != nil {
		t.Fatal(err)
	}
	if r.Sources == nil || len(r.Sources) != 0 {
		t.Errorf("missing file should yield empty non-nil slice, got %+v", r.Sources)
	}
}

func TestLoadRegistryCorrupt(t *testing.T) {
	path := sourcesTmp(t)
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Sources) != 0 {
		t.Error("corrupt file should yield empty registry")
	}
	if _, err := os.Stat(path + ".corrupt"); err != nil {
		t.Errorf("corrupt file should be moved aside: %v", err)
	}
}

var uuidRe = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)

func TestAddSourceMintsKeyAndDefaults(t *testing.T) {
	path := sourcesTmp(t)
	now := time.Date(2026, 7, 7, 10, 0, 0, 0, time.UTC)

	// No cap → defaults to normal.
	s, err := AddSource(path, "email-sentry", "", Rate{}, "", now)
	if err != nil {
		t.Fatal(err)
	}
	if !uuidRe.MatchString(s.Key) {
		t.Errorf("key is not a canonical UUIDv4: %q", s.Key)
	}
	if s.LevelCap != LevelNormal {
		t.Errorf("default cap = %q, want normal", s.LevelCap)
	}
	if s.CreatedAt != now.Format(time.RFC3339) {
		t.Errorf("CreatedAt = %q", s.CreatedAt)
	}

	// Explicit cap + rate + chat.
	s2, err := AddSource(path, "backups", LevelUrgent, Rate{Burst: 3, RefillMins: 10}, "555", now)
	if err != nil {
		t.Fatal(err)
	}
	if s2.LevelCap != LevelUrgent || s2.Rate.Burst != 3 || s2.ChatID != "555" {
		t.Errorf("stored source wrong: %+v", s2)
	}

	// Keys are distinct.
	if s.Key == s2.Key {
		t.Error("two sources minted the same key")
	}

	// Duplicate label rejected.
	if _, err := AddSource(path, "backups", "", Rate{}, "", now); err == nil {
		t.Error("duplicate label should error")
	}

	// Empty label rejected.
	if _, err := AddSource(path, "  ", "", Rate{}, "", now); err == nil {
		t.Error("empty label should error")
	}
}

func TestRevokeSource(t *testing.T) {
	path := sourcesTmp(t)
	now := time.Now()
	if _, err := AddSource(path, "a", "", Rate{}, "", now); err != nil {
		t.Fatal(err)
	}
	if _, err := AddSource(path, "b", "", Rate{}, "", now); err != nil {
		t.Fatal(err)
	}
	if _, err := RevokeSource(path, "a"); err != nil {
		t.Fatalf("revoke: %v", err)
	}
	r, _ := LoadRegistry(path)
	if len(r.Sources) != 1 || r.Sources[0].Label != "b" {
		t.Errorf("after revoke: %+v", r.Sources)
	}
	if _, err := RevokeSource(path, "nope"); !errors.Is(err, ErrSourceNotFound) {
		t.Errorf("revoke missing: want ErrSourceNotFound, got %v", err)
	}
}

func TestFindByKey(t *testing.T) {
	path := sourcesTmp(t)
	s, _ := AddSource(path, "email-sentry", LevelUrgent, Rate{}, "", time.Now())
	r, _ := LoadRegistry(path)

	got, ok := r.FindByKey(s.Key)
	if !ok || got.Label != "email-sentry" {
		t.Errorf("FindByKey(valid) = %+v, %v", got, ok)
	}
	if _, ok := r.FindByKey("9f2c6a1e-8b4d-4f3a-9c7e-2d5b8e1f4a6c"); ok {
		t.Error("FindByKey(unknown) should miss")
	}
	if _, ok := r.FindByKey(""); ok {
		t.Error("FindByKey(empty) should miss")
	}
	// A revoked key immediately misses on a fresh read.
	if _, err := RevokeSource(path, "email-sentry"); err != nil {
		t.Fatal(err)
	}
	r2, _ := LoadRegistry(path)
	if _, ok := r2.FindByKey(s.Key); ok {
		t.Error("revoked key should miss after fresh load")
	}
}

func TestConcurrentAddSourceNoLostUpdates(t *testing.T) {
	path := sourcesTmp(t)
	now := time.Now()
	const n = 20
	var wg sync.WaitGroup
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			label := "src-" + string(rune('a'+i))
			if _, err := AddSource(path, label, "", Rate{}, "", now); err != nil {
				t.Errorf("concurrent add %d: %v", i, err)
			}
		}(i)
	}
	wg.Wait()
	r, _ := LoadRegistry(path)
	if len(r.Sources) != n {
		t.Errorf("concurrent adds lost updates: got %d, want %d", len(r.Sources), n)
	}
	seen := map[string]bool{}
	for _, s := range r.Sources {
		if seen[s.Key] {
			t.Errorf("duplicate key %q", s.Key)
		}
		seen[s.Key] = true
	}
}
