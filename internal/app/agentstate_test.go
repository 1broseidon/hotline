package app

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/schedule"
)

// drainTransients collects agent_state (and any other) transient frames queued
// on the subscription within a short window.
func drainTransients(t *testing.T, sub *mailboxSubscriber, wait time.Duration) []map[string]any {
	t.Helper()
	var out []map[string]any
	deadline := time.After(wait)
	for {
		select {
		case raw := <-sub.transients:
			var m map[string]any
			if err := json.Unmarshal(raw, &m); err != nil {
				t.Fatalf("decode transient: %v", err)
			}
			out = append(out, m)
		case <-deadline:
			return out
		}
	}
}

func seedSchedule(t *testing.T, srv *Server, dev string, paused bool) {
	t.Helper()
	sc := schedule.Schedule{
		Prompt:     "morning positions brief\nsecond line",
		Source:     "app",
		ChatID:     dev,
		Recurrence: schedule.Recurrence{Kind: schedule.KindDaily, TimeOfDay: "07:00"},
		NextFire:   time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
		Paused:     paused,
	}
	stored, err := schedule.Add(srv.schedulesPath(), sc, time.Now())
	if err != nil {
		t.Fatalf("seed schedule: %v", err)
	}
	if paused {
		if _, err := schedule.SetPaused(srv.schedulesPath(), stored.ID, true, time.Now(), time.Local); err != nil {
			t.Fatalf("pause schedule: %v", err)
		}
	}
}

func TestBuildAgentStateSourcesRunsSchedulesLoops(t *testing.T) {
	// The pending-loop fixture is independent of the operator's ambient YOLO
	// posture; dedicated loop tests cover automatic approval.
	t.Setenv("HOTLINE_YOLO", "0")
	t.Setenv("HOTLINE_HARNESS", "claude")
	srv, _, dev, _ := activeHarness(t)

	// a running job (and a finished one that must be excluded from runs — done
	// jobs leave the registry entirely, P2-4)
	srv.jobs.jobs["job-1"] = &jobRecord{jobID: "job-1", title: "header pass", detail: "tests", startedAt: 100}
	srv.jobs.finishLocked("job-2", "ok", false)

	// an active schedule and a paused one (paused excluded)
	seedSchedule(t, srv, dev, false)
	seedSchedule(t, srv, dev, true)

	// an approved loop and a pending one (pending excluded)
	if _, err := loop.Add(loop.Path(srv.stateRoot()), loop.Loop{Label: "email-sentry", Every: "15m", Cmd: "true"}, time.Now()); err != nil {
		t.Fatalf("seed loop: %v", err)
	}
	if _, err := loop.Add(loop.Path(srv.stateRoot()), loop.Loop{Label: "pending-loop", Every: "30m", Cmd: "true", Approved: false}, time.Now(),
		loop.WithApprovalGate(srv.stateRoot(), false)); err != nil {
		t.Fatalf("seed pending loop: %v", err)
	}

	snap := srv.buildAgentState()
	if len(snap.Runs) != 1 || snap.Runs[0].ID != "job-1" {
		t.Fatalf("runs = %+v, want only job-1", snap.Runs)
	}
	if len(snap.Schedules) != 1 {
		t.Fatalf("schedules = %+v, want 1 active", snap.Schedules)
	}
	if snap.Schedules[0].Label != "morning positions brief" {
		t.Errorf("schedule label = %q (should be first line of prompt)", snap.Schedules[0].Label)
	}
	if snap.Schedules[0].Next == 0 || snap.Schedules[0].Recurrence == "" {
		t.Errorf("schedule next/recurrence missing: %+v", snap.Schedules[0])
	}
	if len(snap.Loops) != 1 || snap.Loops[0].Label != "email-sentry" || snap.Loops[0].State != "active" {
		t.Fatalf("loops = %+v, want only approved email-sentry active", snap.Loops)
	}
}

// TestSnapshotOnSubscribeUnconditional pins ERRATA E7: the on-subscribe
// snapshot is sent even when NOTHING is standing — {runs:[],schedules:[],
// loops:[]} — so a device holding a stale pre-restart snapshot is corrected.
func TestSnapshotOnSubscribeUnconditional(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	srv.snapshotAgentStateTo(dev)
	got := drainTransients(t, sub, 200*time.Millisecond)
	if len(got) != 1 || got[0]["t"] != "agent_state" {
		t.Fatalf("want one (empty) agent_state snapshot, got %+v", got)
	}
	state := got[0]["state"].(map[string]any)
	for _, k := range []string{"runs", "schedules", "loops"} {
		arr, ok := state[k].([]any)
		if !ok {
			t.Fatalf("state.%s should be an empty ARRAY (never null): %v", k, state[k])
		}
		if len(arr) != 0 {
			t.Errorf("state.%s = %v, want empty", k, arr)
		}
	}

	// with a running job → the device gets the populated snapshot
	srv.jobs.jobs["job-1"] = &jobRecord{jobID: "job-1", title: "t", startedAt: 1}
	srv.snapshotAgentStateTo(dev)
	got = drainTransients(t, sub, 200*time.Millisecond)
	if len(got) != 1 || got[0]["t"] != "agent_state" {
		t.Fatalf("want one agent_state snapshot, got %+v", got)
	}
	state = got[0]["state"].(map[string]any)
	if runs := state["runs"].([]any); len(runs) != 1 {
		t.Errorf("snapshot runs = %v", runs)
	}
}

func TestAgentStateThrottleImmediateThenTrailing(t *testing.T) {
	srv, _, _, sub := activeHarness(t)
	srv.asThrottle = 80 * time.Millisecond

	// first change → immediate send
	srv.jobs.jobs["job-1"] = &jobRecord{jobID: "job-1", title: "a", startedAt: 1}
	srv.agentStateChanged()
	first := drainTransients(t, sub, 60*time.Millisecond)
	if len(first) != 1 {
		t.Fatalf("first change should send immediately, got %d frames", len(first))
	}

	// a burst inside the window coalesces into ONE trailing send carrying the
	// latest state (two runs).
	srv.jobs.jobs["job-2"] = &jobRecord{jobID: "job-2", title: "b", startedAt: 2}
	srv.agentStateChanged()
	srv.agentStateChanged()
	srv.agentStateChanged()
	trailing := drainTransients(t, sub, 300*time.Millisecond)
	if len(trailing) != 1 {
		t.Fatalf("burst should coalesce to one trailing send, got %d", len(trailing))
	}
	state := trailing[0]["state"].(map[string]any)
	if runs := state["runs"].([]any); len(runs) != 2 {
		t.Errorf("trailing snapshot should reflect latest state (2 runs), got %d", len(runs))
	}

	// no further change → a later trigger dedupes to nothing
	time.Sleep(120 * time.Millisecond)
	srv.agentStateChanged()
	if extra := drainTransients(t, sub, 150*time.Millisecond); len(extra) != 0 {
		t.Errorf("unchanged snapshot should be suppressed, got %d", len(extra))
	}
}

func TestStoreObserverTriggersAgentState(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	srv.asThrottle = 40 * time.Millisecond
	unobserve := srv.registerStoreObservers()
	defer unobserve()

	// a schedule create flows through schedule.Mutate → observer → agent_state
	seedSchedule(t, srv, dev, false)
	got := drainTransients(t, sub, 300*time.Millisecond)
	if len(got) == 0 {
		t.Fatal("schedule mutation did not trigger an agent_state emission")
	}
	last := got[len(got)-1]
	if last["t"] != "agent_state" {
		t.Fatalf("expected agent_state, got %v", last["t"])
	}
	state := last["state"].(map[string]any)
	if scheds := state["schedules"].([]any); len(scheds) != 1 {
		t.Errorf("observer snapshot schedules = %v", scheds)
	}
}

// TestStopAgentStateEmitterSilencesServer pins P2-5: after stop, a pending
// trailing send is cancelled and later change notifications are no-ops.
func TestStopAgentStateEmitterSilencesServer(t *testing.T) {
	srv, _, _, sub := activeHarness(t)
	srv.asThrottle = 500 * time.Millisecond
	srv.jobs.jobs["job-1"] = &jobRecord{jobID: "job-1", title: "a", startedAt: 1}
	srv.agentStateChanged() // immediate send
	_ = drainTransients(t, sub, 60*time.Millisecond)

	srv.jobs.jobs["job-2"] = &jobRecord{jobID: "job-2", title: "b", startedAt: 2}
	srv.agentStateChanged() // still inside the 500ms window → trailing timer
	srv.stopAgentStateEmitter()
	if got := drainTransients(t, sub, 700*time.Millisecond); len(got) != 0 {
		t.Fatalf("stopped emitter still sent %d frames", len(got))
	}
	srv.agentStateChanged() // must be a no-op after stop
	if got := drainTransients(t, sub, 100*time.Millisecond); len(got) != 0 {
		t.Fatalf("agentStateChanged after stop emitted %d frames", len(got))
	}
}
