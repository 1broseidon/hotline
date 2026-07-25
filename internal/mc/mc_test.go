package mc

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// fixedClock returns a clock pinned at t for deterministic timestamps.
func fixedClock(t time.Time) func() time.Time { return func() time.Time { return t } }

func newTestStore(t *testing.T) *Store {
	t.Helper()
	s := NewStore(filepath.Join(t.TempDir(), "mc"))
	s.SetClock(fixedClock(time.Date(2026, 7, 16, 14, 2, 11, 0, time.UTC)))
	if err := s.Seed(); err != nil {
		t.Fatalf("seed: %v", err)
	}
	return s
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return string(b)
}

func TestSeedCreatesMapAndIsIdempotent(t *testing.T) {
	s := newTestStore(t)
	idx := readFile(t, s.indexPath())
	for _, want := range []string{
		"# INDEX — mission control map",
		standingHeader,
		"mission control initialized",
		threadsBeginMarker,
		threadsEndMarker,
	} {
		if !strings.Contains(idx, want) {
			t.Errorf("seed INDEX.md missing %q\n%s", want, idx)
		}
	}
	// Second seed must not clobber operator content.
	if err := os.WriteFile(s.indexPath(), []byte("CUSTOM\n"+threadsBeginMarker+"\n"+threadsEndMarker+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := s.Seed(); err != nil {
		t.Fatal(err)
	}
	if got := readFile(t, s.indexPath()); !strings.HasPrefix(got, "CUSTOM") {
		t.Errorf("re-seed clobbered existing INDEX.md: %s", got)
	}
}

func TestUpdateRoundTripAndIndexRegen(t *testing.T) {
	s := newTestStore(t)
	msg, isErr := s.Apply(Input{Action: "update", Thread: "relay-cors", Status: "active",
		Summary: "CORS fix for the web code-redeem endpoint", Next: "verify header on prod"})
	if isErr {
		t.Fatalf("update failed: %s", msg)
	}

	// Thread file has the front-matter truth and a heading body.
	tf := readFile(t, filepath.Join(s.threadsDir(), "relay-cors.md"))
	for _, want := range []string{
		"slug: relay-cors", "status: active",
		"summary: CORS fix for the web code-redeem endpoint",
		"next: verify header on prod",
		"updated: 2026-07-16T14:02:11Z",
		"# relay-cors",
	} {
		if !strings.Contains(tf, want) {
			t.Errorf("thread file missing %q\n%s", want, tf)
		}
	}

	// Round-trip: parse it back.
	parsed := parseThread("relay-cors", tf)
	if parsed.status != "active" || parsed.next != "verify header on prod" {
		t.Errorf("round-trip lost fields: %+v", parsed)
	}

	// The index table row is inside the markers.
	idx := readFile(t, s.indexPath())
	begin := strings.Index(idx, threadsBeginMarker)
	end := strings.Index(idx, threadsEndMarker)
	table := idx[begin:end]
	if !strings.Contains(table, "| relay-cors | active | CORS fix for the web code-redeem endpoint → verify header on prod | 2026-07-16 |") {
		t.Errorf("index table row wrong:\n%s", table)
	}
}

func TestIndexMarkerFencingPreservesOutsideContent(t *testing.T) {
	s := newTestStore(t)
	// Add operator prose outside the markers.
	idx := readFile(t, s.indexPath())
	idx = strings.Replace(idx, threadsBeginMarker, "## My own section\n- keep me\n\n"+threadsBeginMarker, 1)
	if err := os.WriteFile(s.indexPath(), []byte(idx), 0o600); err != nil {
		t.Fatal(err)
	}
	// A write regenerates the table but must preserve the prose + standing notes.
	if _, isErr := s.Apply(Input{Action: "update", Thread: "t1", Summary: "x"}); isErr {
		t.Fatal("update failed")
	}
	got := readFile(t, s.indexPath())
	for _, want := range []string{"## My own section", "- keep me", "mission control initialized", "| t1 |"} {
		if !strings.Contains(got, want) {
			t.Errorf("regen dropped %q\n%s", want, got)
		}
	}
}

// TestAddStandingNotePreservesProse is the P1-4 repro: operator prose and blank
// lines inside the Standing notes section must survive an addStandingNote write —
// the tool manages only the bullets it owns.
func TestAddStandingNotePreservesProse(t *testing.T) {
	s := newTestStore(t)
	// Rewrite INDEX.md with prose + blank lines interleaved in the standing section.
	idx := readFile(t, s.indexPath())
	custom := strings.Replace(idx, standingHeader+"\n",
		standingHeader+"\n"+
			"Operator prose that must not vanish.\n"+
			"\n"+
			"- 2026-01-01: a hand-written bullet\n"+
			"\n"+
			"More prose below the bullet.\n"+
			"\n", 1)
	if err := os.WriteFile(s.indexPath(), []byte(custom), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, isErr := s.Apply(Input{Action: "note", Text: "a fresh fact"}); isErr {
		t.Fatal("standing note failed")
	}
	got := readFile(t, s.indexPath())
	for _, want := range []string{
		"Operator prose that must not vanish.",
		"- 2026-01-01: a hand-written bullet",
		"More prose below the bullet.",
		"- 2026-07-16: a fresh fact",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("addStandingNote dropped %q\n%s", want, got)
		}
	}
	// The blank line between the prose lines must survive (verbatim structure).
	if !strings.Contains(got, "a hand-written bullet\n\nMore prose") {
		t.Errorf("blank line inside standing section not preserved:\n%s", got)
	}
}

func TestNoteStandingAndThread(t *testing.T) {
	s := newTestStore(t)
	if _, isErr := s.Apply(Input{Action: "note", Text: "deploys go through make ship"}); isErr {
		t.Fatal("standing note failed")
	}
	idx := readFile(t, s.indexPath())
	if !strings.Contains(idx, "- 2026-07-16: deploys go through make ship") {
		t.Errorf("standing note not appended:\n%s", idx)
	}

	// note to a thread appends to its log (thread must exist).
	if _, isErr := s.Apply(Input{Action: "update", Thread: "ping", Summary: "ping tuning"}); isErr {
		t.Fatal("update failed")
	}
	if _, isErr := s.Apply(Input{Action: "note", Thread: "ping", Text: "shipped idle-gated 30s"}); isErr {
		t.Fatal("thread note failed")
	}
	tf := readFile(t, filepath.Join(s.threadsDir(), "ping.md"))
	if !strings.Contains(tf, "- 2026-07-16 14:02 — shipped idle-gated 30s") {
		t.Errorf("thread log line missing:\n%s", tf)
	}

	// note to an unknown thread errors, naming the live set.
	msg, isErr := s.Apply(Input{Action: "note", Thread: "nope", Text: "x"})
	if !isErr || !strings.Contains(msg, `unknown thread "nope"`) || !strings.Contains(msg, "ping") {
		t.Errorf("unknown-thread note should error naming actives, got: %s", msg)
	}
}

func TestArchiveMovesFileAndSyncsIndex(t *testing.T) {
	s := newTestStore(t)
	s.Apply(Input{Action: "update", Thread: "done-me", Summary: "temp"})
	msg, isErr := s.Apply(Input{Action: "archive", Thread: "done-me", Outcome: "shipped and verified"})
	if isErr {
		t.Fatalf("archive failed: %s", msg)
	}
	if _, err := os.Stat(filepath.Join(s.threadsDir(), "done-me.md")); !os.IsNotExist(err) {
		t.Error("archived thread still in threads/")
	}
	arc := readFile(t, filepath.Join(s.archiveDir(), "done-me.md"))
	if !strings.Contains(arc, "status: done") || !strings.Contains(arc, "archived — shipped and verified") {
		t.Errorf("archive file wrong:\n%s", arc)
	}
	if strings.Contains(readFile(t, s.indexPath()), "| done-me |") {
		t.Error("archived thread still in index table")
	}
}

func TestUpdateStatusDoneArchives(t *testing.T) {
	s := newTestStore(t)
	s.Apply(Input{Action: "update", Thread: "x", Summary: "s"})
	if _, isErr := s.Apply(Input{Action: "update", Thread: "x", Status: "done"}); isErr {
		t.Fatal("update done failed")
	}
	if _, err := os.Stat(filepath.Join(s.archiveDir(), "x.md")); err != nil {
		t.Errorf("update status=done did not archive: %v", err)
	}
}

func TestHandoffWriteAndRotate(t *testing.T) {
	s := newTestStore(t)
	if _, isErr := s.Apply(Input{Action: "handoff", State: "first pass done", Next: "wire redial"}); isErr {
		t.Fatal("handoff failed")
	}
	h := readFile(t, s.handoffPath())
	for _, want := range []string{"written: 2026-07-16T14:02:11Z", "trigger: manual", "**State:** first pass done", "**Next:** wire redial"} {
		if !strings.Contains(h, want) {
			t.Errorf("handoff.md missing %q\n%s", want, h)
		}
	}
	// A second handoff rotates the first into archive/handoffs.md.
	if _, isErr := s.Apply(Input{Action: "handoff", State: "second", Next: "ship"}); isErr {
		t.Fatal("second handoff failed")
	}
	arc := readFile(t, filepath.Join(s.archiveDir(), "handoffs.md"))
	if !strings.Contains(arc, "first pass done") {
		t.Errorf("previous handoff not rotated to archive:\n%s", arc)
	}
	if !strings.Contains(readFile(t, s.handoffPath()), "second") {
		t.Error("current handoff.md not the newest")
	}
}

// TestHandoffRotateSeparatorCollision guards the archive framing against a body
// that contains what used to be the entry separator ("---8<---"): with the old
// naive split it would fracture one entry into several and corrupt the archive.
func TestHandoffRotateSeparatorCollision(t *testing.T) {
	s := newTestStore(t)
	nasty := "line before\n---8<---\nline after the fake separator"
	s.Apply(Input{Action: "handoff", State: nasty, Next: "one"})
	s.Apply(Input{Action: "handoff", State: "second", Next: "two"})
	s.Apply(Input{Action: "handoff", State: "third", Next: "three"})

	arc := readFile(t, filepath.Join(s.archiveDir(), "handoffs.md"))
	entries := parseArchivedHandoffs(arc)
	if len(entries) != 2 {
		t.Fatalf("expected exactly 2 archived entries, got %d:\n%s", len(entries), arc)
	}
	// The first archived entry is the nasty one; it must be intact, collision and all.
	if !strings.Contains(entries[0], "---8<---") || !strings.Contains(entries[0], "line after the fake separator") {
		t.Errorf("collision fractured the entry: %q", entries[0])
	}
	if !strings.Contains(entries[1], "second") {
		t.Errorf("second entry lost: %q", entries[1])
	}
}

func TestVerbValidation(t *testing.T) {
	s := newTestStore(t)
	cases := []struct {
		name string
		in   Input
		want string
	}{
		{"unknown action", Input{Action: "frobnicate"}, `unknown action "frobnicate"`},
		{"bad slug", Input{Action: "update", Thread: "Not A Slug", Summary: "x"}, "kebab-case slug"},
		{"note no text", Input{Action: "note"}, "note requires text"},
		{"update empty", Input{Action: "update", Thread: "t"}, "at least one of"},
		{"update bad status", Input{Action: "update", Thread: "t", Status: "wut"}, "status must be"},
		{"handoff no state", Input{Action: "handoff", Next: "x"}, "handoff requires state"},
		{"handoff no next", Input{Action: "handoff", State: "x"}, "handoff requires next"},
		{"archive no outcome", Input{Action: "archive", Thread: "t"}, "archive requires outcome"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			msg, isErr := s.Apply(tc.in)
			if !isErr {
				t.Fatalf("expected error, got ok: %s", msg)
			}
			if !strings.Contains(msg, tc.want) {
				t.Errorf("error = %q, want substring %q", msg, tc.want)
			}
		})
	}
}

func TestClampField(t *testing.T) {
	long := strings.Repeat("a", maxField+500)
	got := clampField(long)
	if len(got) != maxField {
		t.Errorf("clampField len = %d, want %d", len(got), maxField)
	}
	// A multibyte rune straddling the cap is dropped whole (no invalid UTF-8).
	multi := strings.Repeat("a", maxField-1) + "€€€"
	if c := clampField(multi); len(c) > maxField {
		t.Errorf("clamp exceeded cap: %d", len(c))
	}
}
