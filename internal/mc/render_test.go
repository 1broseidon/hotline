package mc

import (
	"fmt"
	"strings"
	"testing"
	"time"
	"unicode/utf8"
)

func TestRenderIndexSectionsAndOrder(t *testing.T) {
	s := newTestStore(t)
	s.Apply(Input{Action: "note", Text: "deploys go through make ship"})
	s.Apply(Input{Action: "update", Thread: "relay-cors", Status: "active",
		Summary: "CORS fix for web redeem", Next: "verify header on prod"})
	s.Apply(Input{Action: "handoff", State: "pingDecision done, 12 tests green", Next: "wire NetInfo redial"})

	out := s.RenderIndex(4096)
	if !strings.HasPrefix(out, "<mc-index>") || !strings.HasSuffix(out, "</mc-index>") {
		t.Fatalf("index not fenced:\n%s", out)
	}
	for _, want := range []string{
		"Standing notes:",
		"- 2026-07-16: deploys go through make ship",
		"Threads (1 active):",
		"- relay-cors [active] CORS fix for web redeem → verify header on prod (upd 07-16)",
		"Handoff (2026-07-16 14:02, manual):",
		"pingDecision done, 12 tests green",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("render missing %q\n%s", want, out)
		}
	}
	// Section order: standing before threads before handoff.
	iS := strings.Index(out, "Standing notes:")
	iT := strings.Index(out, "Threads (")
	iH := strings.Index(out, "Handoff (")
	if !(iS < iT && iT < iH) {
		t.Errorf("section order wrong: standing=%d threads=%d handoff=%d", iS, iT, iH)
	}
}

func TestRenderIndexBudgetDropsThreadRows(t *testing.T) {
	s := newTestStore(t)
	for i := 0; i < 10; i++ {
		s.Apply(Input{Action: "update", Thread: fmt.Sprintf("thread-%02d", i),
			Summary: strings.Repeat("x", 60), Next: strings.Repeat("y", 40)})
	}
	full := s.RenderIndex(4096)
	fullRows := strings.Count(full, "\n- thread-")
	if fullRows != 10 {
		t.Fatalf("expected 10 rows unbudgeted, got %d\n%s", fullRows, full)
	}

	tight := s.RenderIndex(400)
	if len(tight) > 400 {
		t.Errorf("render exceeded budget: %d > 400", len(tight))
	}
	if !strings.Contains(tight, "more threads — read INDEX.md") {
		t.Errorf("budgeted render missing overflow note:\n%s", tight)
	}
	tightRows := strings.Count(tight, "\n- thread-")
	if tightRows >= 10 {
		t.Errorf("budget did not drop rows: %d", tightRows)
	}
}

func TestRenderIndexDormantThreadsCountedNotRowed(t *testing.T) {
	s := NewStore(t.TempDir() + "/mc")
	now := time.Date(2026, 7, 16, 12, 0, 0, 0, time.UTC)
	s.SetClock(fixedClock(now))
	s.Seed()
	// A fresh thread and one last touched 40 days ago.
	s.Apply(Input{Action: "update", Thread: "fresh", Summary: "live"})
	s.SetClock(fixedClock(now.Add(-40 * 24 * time.Hour)))
	s.Apply(Input{Action: "update", Thread: "old", Summary: "stale"})
	s.SetClock(fixedClock(now))

	out := s.RenderIndex(4096)
	if !strings.Contains(out, "- fresh [active]") {
		t.Errorf("fresh thread should render as a row:\n%s", out)
	}
	if strings.Contains(out, "- old [active]") {
		t.Errorf("dormant thread should not render as a row:\n%s", out)
	}
	if !strings.Contains(out, "dormant — INDEX.md") {
		t.Errorf("dormant count missing:\n%s", out)
	}
}

func TestRenderHandoffStaleTag(t *testing.T) {
	s := NewStore(t.TempDir() + "/mc")
	now := time.Date(2026, 7, 16, 12, 0, 0, 0, time.UTC)
	s.SetClock(fixedClock(now.Add(-9 * 24 * time.Hour)))
	s.Seed()
	s.Apply(Input{Action: "handoff", State: "old work", Next: "resume"})
	s.SetClock(fixedClock(now))

	out := s.RenderIndex(4096)
	if !strings.Contains(out, "(stale — 9d old)") {
		t.Errorf("stale handoff tag missing/wrong format:\n%s", out)
	}
}

// TestRenderIndexBudgetBindsWithStandingNotes is the P0-1 repro: 25 standing
// notes of ~1KB each blow the budget when only thread rows are droppable. The
// packer must drop standing notes too and guarantee the block is ≤ budget.
func TestRenderIndexBudgetBindsWithStandingNotes(t *testing.T) {
	s := newTestStore(t)
	for i := 0; i < 25; i++ {
		s.Apply(Input{Action: "note", Text: strings.Repeat("z", 1024)})
	}
	out := s.RenderIndex(4096)
	if len(out) > 4096 {
		t.Fatalf("RenderIndex(4096) returned %d bytes, must be ≤ 4096", len(out))
	}
	if !strings.HasPrefix(out, "<mc-index>") {
		t.Errorf("render not fenced:\n%s", out)
	}
}

// TestRenderIndexBudgetHardFloor guarantees the ≤budget invariant even at a
// pathologically small budget (backstop clamp).
func TestRenderIndexBudgetHardFloor(t *testing.T) {
	s := newTestStore(t)
	for i := 0; i < 5; i++ {
		s.Apply(Input{Action: "note", Text: strings.Repeat("z", 200)})
		s.Apply(Input{Action: "update", Thread: fmt.Sprintf("t-%02d", i), Summary: strings.Repeat("y", 80)})
	}
	s.Apply(Input{Action: "handoff", State: strings.Repeat("s", 400), Next: "go"})
	for _, budget := range []int{2000, 500, 120, 40} {
		if out := s.RenderIndex(budget); len(out) > budget {
			t.Errorf("RenderIndex(%d) = %d bytes, must be ≤ budget", budget, len(out))
		}
	}
}

// TestRenderIndexRowRuneSafe is the P1-1 repro: a 200×'é' summary must clamp on
// a rune boundary so the row is valid UTF-8, never sliced mid-rune.
func TestRenderIndexRowRuneSafe(t *testing.T) {
	s := newTestStore(t)
	s.Apply(Input{Action: "update", Thread: "wide", Summary: strings.Repeat("é", 200)})
	out := s.RenderIndex(4096)
	if !utf8.ValidString(out) {
		t.Fatalf("render is not valid UTF-8 (row clamp sliced a rune):\n%q", out)
	}
	if !strings.Contains(out, "…") {
		t.Errorf("over-long row should be ellipsized:\n%s", out)
	}
}

func TestStandingNotesBoundRollsOldest(t *testing.T) {
	s := newTestStore(t)
	// Seed already added 1 standing note; add 35 more to force overflow past 30.
	for i := 0; i < 35; i++ {
		s.Apply(Input{Action: "note", Text: fmt.Sprintf("note-%02d", i)})
	}
	notes := s.standingNotes()
	if len(notes) > maxStandingNotes {
		t.Errorf("standing notes not bounded: %d > %d", len(notes), maxStandingNotes)
	}
	// Oldest (the seed init line) should have rolled into the archive.
	arc := s.archiveDir() + "/standing-2026-07.md"
	if b := readFile(t, arc); !strings.Contains(b, "mission control initialized") {
		t.Errorf("oldest note not rolled to archive:\n%s", b)
	}
}

// A negative budget must clamp to empty output, never panic — RenderIndex is
// exported, so programmatic callers can pass what config would reject.
func TestRenderIndexNegativeBudget(t *testing.T) {
	s := newTestStore(t)
	s.Apply(Input{Action: "handoff", State: "mid-flight", Next: "go"})
	if got := s.RenderIndex(-1); got != "" {
		t.Fatalf("RenderIndex(-1) = %q, want empty", got)
	}
}
