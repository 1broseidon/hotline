package supervise

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestAuthFatalExitPinsBackoffAtMax: a harness that exits with code 5 cold-loops
// — the backoff is pinned at Max, not the 2s first step, and the status line
// says credentials need the operator.
func TestAuthFatalExitPinsBackoffAtMax(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			h := &fakeHarness{pid: 200 + n, done: make(chan struct{}), exitOnTerm: true}
			h.exit("exit status 5")
			return h
		},
		sleepOK: func(int) bool { return false }, // shut down after the first backoff
	}
	s := newTestSupervisor(t, rec)
	waitRun(t, runSupervisor(t, s, context.Background()))

	sleeps := rec.sleepDurations()
	if len(sleeps) != 1 || sleeps[0] != s.Backoff.Max {
		t.Errorf("auth-fatal backoff = %v, want [%v]", sleeps, s.Backoff.Max)
	}
	st, err := ReadState(s.Dir)
	if err != nil || st == nil {
		t.Fatalf("ReadState = %v, %v", st, err)
	}
	if !strings.Contains(st.LastExit, "auth failure — credentials need the operator") {
		t.Errorf("status line = %q, want the auth-failure note", st.LastExit)
	}
}

// TestAuthFatalMarkerPinsBackoffAtMax: even when the harness can't set exit 5,
// the auth.fatal marker in the supervisor dir pins the backoff at Max.
func TestAuthFatalMarkerPinsBackoffAtMax(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			h := &fakeHarness{pid: 300 + n, done: make(chan struct{}), exitOnTerm: true}
			h.exit("exit status 1")
			return h
		},
		sleepOK: func(int) bool { return false },
	}
	s := newTestSupervisor(t, rec)
	if err := os.MkdirAll(s.Dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(s.Dir, AuthFatalMarker), []byte("bad creds\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	waitRun(t, runSupervisor(t, s, context.Background()))

	sleeps := rec.sleepDurations()
	if len(sleeps) != 1 || sleeps[0] != s.Backoff.Max {
		t.Errorf("marker-pinned backoff = %v, want [%v]", sleeps, s.Backoff.Max)
	}
}

// TestNormalBackoffRestoredWhenMarkerClears: once the marker is gone (the
// harness recovered), the very next crash restarts at the normal first step
// rather than the pinned Max.
func TestNormalBackoffRestoredWhenMarkerClears(t *testing.T) {
	rec := &recorder{
		sleepOK: func(n int) bool { return n < 1 }, // shut down on the SECOND backoff
	}
	s := newTestSupervisor(t, rec)
	if err := os.MkdirAll(s.Dir, 0o700); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(s.Dir, AuthFatalMarker)
	if err := os.WriteFile(marker, []byte("bad creds\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	rec.makeNext = func(n int) *fakeHarness {
		if n == 1 {
			_ = os.Remove(marker) // the harness "recovered" before this crash
		}
		h := &fakeHarness{pid: 400 + n, done: make(chan struct{}), exitOnTerm: true}
		h.exit("exit status 1")
		return h
	}
	waitRun(t, runSupervisor(t, s, context.Background()))

	sleeps := rec.sleepDurations()
	want := []time.Duration{s.Backoff.Max, 2 * time.Second}
	if len(sleeps) != 2 || sleeps[0] != want[0] || sleeps[1] != want[1] {
		t.Errorf("backoff sleeps = %v, want %v (pinned, then normal first step)", sleeps, want)
	}
}
