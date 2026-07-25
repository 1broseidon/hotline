package mc

import (
	"strings"
	"testing"
	"time"
)

func newReadStore(t *testing.T) *Store {
	t.Helper()
	s := NewStore(t.TempDir() + "/mc")
	s.SetClock(func() time.Time { return time.Date(2026, 7, 16, 14, 2, 11, 0, time.UTC) })
	if err := s.Seed(); err != nil {
		t.Fatal(err)
	}
	return s
}

func TestHandoffTriggerDefaultsAndOverride(t *testing.T) {
	s := newReadStore(t)

	// No trigger ⇒ manual (the MCP tool path).
	if msg, isErr := s.Apply(Input{Action: "handoff", State: "s", Next: "n"}); isErr {
		t.Fatalf("handoff: %s", msg)
	}
	raw, ok, err := s.ReadHandoff()
	if err != nil || !ok {
		t.Fatalf("ReadHandoff: ok=%v err=%v", ok, err)
	}
	if !strings.Contains(raw, "trigger: manual") {
		t.Errorf("default trigger should be manual:\n%s", raw)
	}

	// Explicit trigger ⇒ recorded (the CLI path).
	if msg, isErr := s.Apply(Input{Action: "handoff", State: "s2", Next: "n2", Trigger: "pre-compact"}); isErr {
		t.Fatalf("handoff2: %s", msg)
	}
	raw, _, _ = s.ReadHandoff()
	if !strings.Contains(raw, "trigger: pre-compact") {
		t.Errorf("explicit trigger not recorded:\n%s", raw)
	}
}

func TestReadIndexAndThread(t *testing.T) {
	s := newReadStore(t)

	idx, ok, err := s.ReadIndex()
	if err != nil || !ok {
		t.Fatalf("ReadIndex: ok=%v err=%v", ok, err)
	}
	if !strings.Contains(idx, "mission control map") {
		t.Errorf("index missing header:\n%s", idx)
	}

	if _, ok, _ := s.ReadThread("nope"); ok {
		t.Error("unknown thread should not be found")
	}

	s.Apply(Input{Action: "update", Thread: "relay-cors", Summary: "cors", Next: "verify"})
	body, ok, err := s.ReadThread("relay-cors")
	if err != nil || !ok {
		t.Fatalf("ReadThread live: ok=%v err=%v", ok, err)
	}
	if !strings.Contains(body, "slug: relay-cors") {
		t.Errorf("thread body:\n%s", body)
	}

	// After archive, ReadThread finds it under archive/.
	s.Apply(Input{Action: "archive", Thread: "relay-cors", Outcome: "done"})
	if _, ok, _ := s.ReadThread("relay-cors"); !ok {
		t.Error("archived thread should still be readable")
	}

	slugs, err := s.ListThreadSlugs()
	if err != nil {
		t.Fatal(err)
	}
	if len(slugs) != 0 {
		t.Errorf("archived thread should not be listed as live: %v", slugs)
	}
}
