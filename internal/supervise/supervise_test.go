package supervise

import (
	"context"
	"io"
	"sync"
	"testing"
	"time"
)

// fakeHarness is a controllable Harness: tests decide when it exits and how
// it reacts to Terminate/Kill.
type fakeHarness struct {
	pid  int
	done chan struct{}

	mu         sync.Mutex
	desc       string
	terminated bool
	killed     bool
	exitOnTerm bool // Terminate ends the process (the normal case)
	exitOnKill bool // only SIGKILL ends it (a stuck harness)
	exited     bool
}

func (h *fakeHarness) Pid() int              { return h.pid }
func (h *fakeHarness) Done() <-chan struct{} { return h.done }
func (h *fakeHarness) ExitDesc() string      { h.mu.Lock(); defer h.mu.Unlock(); return h.desc }

func (h *fakeHarness) exit(desc string) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.exited {
		return
	}
	h.exited, h.desc = true, desc
	close(h.done)
}

func (h *fakeHarness) Terminate() {
	h.mu.Lock()
	h.terminated = true
	shouldExit := h.exitOnTerm
	h.mu.Unlock()
	if shouldExit {
		h.exit("signal: terminated")
	}
}

func (h *fakeHarness) Kill() {
	h.mu.Lock()
	h.killed = true
	shouldExit := h.exitOnKill || h.exitOnTerm
	h.mu.Unlock()
	if shouldExit {
		h.exit("signal: killed")
	}
}

func (h *fakeHarness) wasTerminated() bool { h.mu.Lock(); defer h.mu.Unlock(); return h.terminated }
func (h *fakeHarness) wasKilled() bool     { h.mu.Lock(); defer h.mu.Unlock(); return h.killed }

// recorder is the test seam bundle: it spawns fake harnesses (configured per
// spawn), records backoff sleeps, and never really waits.
type recorder struct {
	mu       sync.Mutex
	spawned  []*fakeHarness
	sleeps   []time.Duration
	sleepOK  func(n int) bool // result of the n-th (0-based) sleep call
	makeNext func(n int) *fakeHarness
	spawnCh  chan *fakeHarness
}

func (r *recorder) start(_ context.Context) (Harness, error) {
	r.mu.Lock()
	n := len(r.spawned)
	h := r.makeNext(n)
	r.spawned = append(r.spawned, h)
	r.mu.Unlock()
	r.spawnCh <- h
	return h, nil
}

func (r *recorder) sleep(ctx context.Context, d time.Duration) bool {
	r.mu.Lock()
	n := len(r.sleeps)
	r.sleeps = append(r.sleeps, d)
	r.mu.Unlock()
	if ctx.Err() != nil {
		return false
	}
	return r.sleepOK(n)
}

func (r *recorder) spawnCount() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.spawned)
}

func (r *recorder) sleepDurations() []time.Duration {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]time.Duration(nil), r.sleeps...)
}

// newTestSupervisor wires a Supervisor over a temp dir with fast poll/grace
// and the recorder's seams (scheduler_test.go style: no long sleeps).
func newTestSupervisor(t *testing.T, rec *recorder) *Supervisor {
	t.Helper()
	if rec.spawnCh == nil {
		rec.spawnCh = make(chan *fakeHarness, 16)
	}
	if rec.sleepOK == nil {
		rec.sleepOK = func(int) bool { return true }
	}
	s := New(t.TempDir(), rec.start)
	s.Poll = 2 * time.Millisecond
	s.Grace = 25 * time.Millisecond
	s.Log = io.Discard
	s.sleep = rec.sleep
	return s
}

func runSupervisor(t *testing.T, s *Supervisor, ctx context.Context) chan error {
	t.Helper()
	done := make(chan error, 1)
	go func() { done <- s.Run(ctx) }()
	return done
}

func waitSpawn(t *testing.T, rec *recorder) *fakeHarness {
	t.Helper()
	select {
	case h := <-rec.spawnCh:
		return h
	case <-time.After(2 * time.Second):
		t.Fatal("harness was not spawned in time")
		return nil
	}
}

func waitRun(t *testing.T, done chan error) {
	t.Helper()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("Run returned %v, want nil", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("Run did not return in time")
	}
}

// TestRestartsOnCrashWithBackoff: three instant crashes take the 2s, 4s, 8s
// backoff steps; when the third sleep is cut short by shutdown, Run finalizes
// with a stopped state that counted every restart.
func TestRestartsOnCrashWithBackoff(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			h := &fakeHarness{pid: 100 + n, done: make(chan struct{}), exitOnTerm: true}
			h.exit("exit status 1") // crashes immediately
			return h
		},
		sleepOK: func(n int) bool { return n < 2 }, // third backoff = shutdown
	}
	s := newTestSupervisor(t, rec)
	waitRun(t, runSupervisor(t, s, context.Background()))

	if got := rec.spawnCount(); got != 3 {
		t.Errorf("spawns = %d, want 3", got)
	}
	want := []time.Duration{2 * time.Second, 4 * time.Second, 8 * time.Second}
	if got := rec.sleepDurations(); len(got) != len(want) || got[0] != want[0] || got[1] != want[1] || got[2] != want[2] {
		t.Errorf("backoff sleeps = %v, want %v", got, want)
	}
	st, err := ReadState(s.Dir)
	if err != nil || st == nil {
		t.Fatalf("state: %v, %v", st, err)
	}
	if st.Phase != PhaseStopped || st.Restarts != 3 {
		t.Errorf("final state = %+v, want stopped with 3 restarts", st)
	}
	if st.LastExit == "" {
		t.Error("LastExit breadcrumb missing")
	}
}

// TestShutdownTerminatesHarness: ctx cancel while the harness is healthy
// stops it gracefully and finalizes state.
func TestShutdownTerminatesHarness(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			return &fakeHarness{pid: 100 + n, done: make(chan struct{}), exitOnTerm: true}
		},
	}
	s := newTestSupervisor(t, rec)
	ctx, cancel := context.WithCancel(context.Background())
	done := runSupervisor(t, s, ctx)
	h := waitSpawn(t, rec)
	cancel()
	waitRun(t, done)

	if !h.wasTerminated() {
		t.Error("harness was not terminated on shutdown")
	}
	if h.wasKilled() {
		t.Error("cooperative harness should not be SIGKILLed")
	}
	st, _ := ReadState(s.Dir)
	if st == nil || st.Phase != PhaseStopped {
		t.Errorf("final state = %+v, want stopped", st)
	}
}

// TestStuckHarnessEscalatesToKill: a harness that ignores SIGTERM is
// SIGKILLed after Grace.
func TestStuckHarnessEscalatesToKill(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			return &fakeHarness{pid: 100 + n, done: make(chan struct{}), exitOnKill: true}
		},
	}
	s := newTestSupervisor(t, rec)
	ctx, cancel := context.WithCancel(context.Background())
	done := runSupervisor(t, s, ctx)
	h := waitSpawn(t, rec)
	cancel()
	waitRun(t, done)

	if !h.wasTerminated() || !h.wasKilled() {
		t.Errorf("terminated=%v killed=%v, want both", h.wasTerminated(), h.wasKilled())
	}
}

// TestRestartRequestBouncesHarness: the control file (the chat restart tool's
// signal path) bounces a healthy harness, resets the backoff, and is consumed
// exactly once.
func TestRestartRequestBouncesHarness(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			return &fakeHarness{pid: 100 + n, done: make(chan struct{}), exitOnTerm: true}
		},
	}
	s := newTestSupervisor(t, rec)
	s.Backoff.n = 5 // pretend earlier crashes ratcheted the backoff up

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := runSupervisor(t, s, ctx)

	h1 := waitSpawn(t, rec)
	if err := RequestRestart(s.Dir, "restart yourself"); err != nil {
		t.Fatal(err)
	}
	h2 := waitSpawn(t, rec) // the bounce
	if !h1.wasTerminated() {
		t.Error("first harness was not terminated on restart request")
	}
	if h1 == h2 {
		t.Fatal("no new harness spawned")
	}
	if _, pending := consumeRestart(s.Dir); pending {
		t.Error("restart request should have been consumed")
	}
	// The Reset happened before the second spawn (the spawnCh receive orders
	// it), so this read is race-free.
	if s.Backoff.n != 0 {
		t.Errorf("backoff not reset on requested restart: n=%d", s.Backoff.n)
	}
	// state.json is written just after the spawn; poll briefly for it.
	deadline := time.Now().Add(2 * time.Second)
	for {
		st, _ := ReadState(s.Dir)
		if st != nil && st.Restarts == 1 && st.Phase == PhaseRunning {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("state after bounce = %+v, want running with 1 restart", st)
		}
		time.Sleep(2 * time.Millisecond)
	}
	cancel()
	waitRun(t, done)
}

// TestStaleRestartRequestDiscarded: a request left on disk from a previous
// supervisor life must not bounce the freshly started harness.
func TestStaleRestartRequestDiscarded(t *testing.T) {
	rec := &recorder{
		makeNext: func(n int) *fakeHarness {
			return &fakeHarness{pid: 100 + n, done: make(chan struct{}), exitOnTerm: true}
		},
	}
	s := newTestSupervisor(t, rec)
	if err := RequestRestart(s.Dir, "from a previous life"); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	done := runSupervisor(t, s, ctx)
	h := waitSpawn(t, rec)
	time.Sleep(20 * time.Millisecond) // several poll ticks
	if h.wasTerminated() {
		t.Error("stale request bounced the fresh harness")
	}
	if got := rec.spawnCount(); got != 1 {
		t.Errorf("spawns = %d, want 1", got)
	}
	cancel()
	waitRun(t, done)
}

// TestSpawnFailureBacksOff: a Start error takes the same backoff path as an
// instant crash and leaves a breadcrumb in state.json.
func TestSpawnFailureBacksOff(t *testing.T) {
	rec := &recorder{
		spawnCh: make(chan *fakeHarness, 16),
		sleepOK: func(n int) bool { return n < 1 },
	}
	failures := 0
	var mu sync.Mutex
	s := newTestSupervisor(t, rec)
	s.Start = func(context.Context) (Harness, error) {
		mu.Lock()
		failures++
		mu.Unlock()
		return nil, io.ErrClosedPipe
	}
	s.sleep = rec.sleep
	waitRun(t, runSupervisor(t, s, context.Background()))

	mu.Lock()
	defer mu.Unlock()
	if failures != 2 {
		t.Errorf("spawn attempts = %d, want 2", failures)
	}
	if got := rec.sleepDurations(); len(got) != 2 || got[0] != 2*time.Second || got[1] != 4*time.Second {
		t.Errorf("sleeps = %v, want [2s 4s]", got)
	}
	st, _ := ReadState(s.Dir)
	if st == nil || st.LastExit == "" || st.Phase != PhaseStopped {
		t.Errorf("state = %+v, want stopped with spawn-failed breadcrumb", st)
	}
}
