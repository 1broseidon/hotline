package jobspool

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"
)

// cardOp records one call the dispatcher made onto the sink.
type cardOp struct {
	kind     string // start|update|done|rehydrate
	jobID    string
	title    string
	detail   string
	state    string
	notify   string
	progress *float64
}

// fakeSink is a scriptable JobSink: it mints deterministic ids, records every
// op, and can be told to fail StartCard/UpdateCard/DoneCard to exercise retry.
type fakeSink struct {
	ops       []cardOp
	seq       int
	failStart bool
	failDone  bool
	live      map[string]bool // job ids the sink considers registered
}

func newFakeSink() *fakeSink { return &fakeSink{live: map[string]bool{}} }

func (f *fakeSink) StartCard(title, detail, chatID string, progress *float64) (string, string, string, error) {
	if f.failStart {
		return "", "", "", errors.New("no active device")
	}
	f.seq++
	jobID := "job-" + itoa(f.seq)
	f.live[jobID] = true
	f.ops = append(f.ops, cardOp{kind: "start", jobID: jobID, title: title, detail: detail, progress: progress})
	return jobID, "a-" + itoa(f.seq), "el-" + jobID, nil
}

func (f *fakeSink) UpdateCard(jobID, detail, chatID string, progress *float64) error {
	if !f.live[jobID] {
		return errors.New("unknown job_id")
	}
	f.ops = append(f.ops, cardOp{kind: "update", jobID: jobID, detail: detail, progress: progress})
	return nil
}

func (f *fakeSink) DoneCard(jobID, state, detail, notify, chatID string) error {
	if f.failDone {
		return errors.New("no active device")
	}
	if !f.live[jobID] {
		return errors.New("unknown job_id")
	}
	f.ops = append(f.ops, cardOp{kind: "done", jobID: jobID, state: state, detail: detail, notify: notify})
	delete(f.live, jobID)
	return nil
}

func (f *fakeSink) RehydrateCard(jobID, elementID, msgID, chatID, title, detail string, startedAt int64, progress *float64) {
	f.live[jobID] = true
	f.ops = append(f.ops, cardOp{kind: "rehydrate", jobID: jobID, title: title})
}

func itoa(n int) string { return string(rune('0' + n%10)) } // single-digit ids are enough for tests

func (f *fakeSink) lastOf(kind string) (cardOp, bool) {
	for i := len(f.ops) - 1; i >= 0; i-- {
		if f.ops[i].kind == kind {
			return f.ops[i], true
		}
	}
	return cardOp{}, false
}

func (f *fakeSink) count(kind string) int {
	n := 0
	for _, o := range f.ops {
		if o.kind == kind {
			n++
		}
	}
	return n
}

// newTestDispatcher builds a dispatcher over temp paths with a fixed clock.
func newTestDispatcher(t *testing.T) (*Dispatcher, string) {
	t.Helper()
	dir := t.TempDir()
	d := NewDispatcher(filepath.Join(dir, "spool.json"), filepath.Join(dir, "activejobs.json"))
	d.now = func() time.Time { return time.Unix(1_000, 0) }
	return d, dir
}

// enqueue appends an intent through the real spool path.
func enqueue(t *testing.T, d *Dispatcher, in Intent) {
	t.Helper()
	if _, err := Enqueue(d.spoolPath, in, d.now()); err != nil {
		t.Fatalf("enqueue: %v", err)
	}
}

func TestSingleJobLifecycle(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "Explore auth"})
	d.dispatch(ctx, sink)
	if got := sink.count("start"); got != 1 {
		t.Fatalf("want 1 start, got %d", got)
	}
	st, _ := sink.lastOf("start")
	if st.title != "Explore auth" {
		t.Fatalf("start title = %q", st.title)
	}

	enqueue(t, d, Intent{Action: "update", Cookie: "c1", Detail: "reading files"})
	d.dispatch(ctx, sink)
	up, ok := sink.lastOf("update")
	if !ok || up.detail != "reading files" {
		t.Fatalf("update detail = %q ok=%v", up.detail, ok)
	}

	enqueue(t, d, Intent{Action: "done", Cookie: "c1", State: "ok", Notify: "found it"})
	d.dispatch(ctx, sink)
	dn, ok := sink.lastOf("done")
	if !ok || dn.state != "ok" || dn.notify != "found it" {
		t.Fatalf("done = %+v ok=%v", dn, ok)
	}
	if len(d.active.Batches) != 0 {
		t.Fatalf("finished batch should be pruned, have %d", len(d.active.Batches))
	}
}

func TestRollupProgressAndTerminal(t *testing.T) {
	tests := []struct {
		name      string
		dones     []string // states in the order workers finish
		wantFinal string
	}{
		{"all ok", []string{"ok", "ok", "ok"}, "ok"},
		{"one err", []string{"ok", "err", "ok"}, "err"},
		{"ok wins over cancelled", []string{"ok", "cancelled", "ok"}, "ok"},
		{"all cancelled", []string{"cancelled", "cancelled", "cancelled"}, "cancelled"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			d, _ := newTestDispatcher(t)
			sink := newFakeSink()
			ctx := context.Background()

			cookies := []string{"w1", "w2", "w3"}
			for _, c := range cookies {
				enqueue(t, d, Intent{Action: "start", Cookie: c, Batch: "sess", Title: "Fan out"})
			}
			d.dispatch(ctx, sink)
			// One card for the whole batch.
			if got := sink.count("start"); got != 1 {
				t.Fatalf("want 1 rollup card, got %d starts", got)
			}
			st, _ := sink.lastOf("start")
			if st.progress == nil || *st.progress != 0 {
				t.Fatalf("initial progress = %v, want 0", st.progress)
			}

			for i, state := range tc.dones {
				enqueue(t, d, Intent{Action: "done", Cookie: cookies[i], Batch: "sess", State: state})
				d.dispatch(ctx, sink)
				if i < len(tc.dones)-1 {
					up, ok := sink.lastOf("update")
					if !ok {
						t.Fatalf("expected progress update after worker %d", i)
					}
					want := float64(i+1) / 3.0
					if up.progress == nil || abs(*up.progress-want) > 1e-9 {
						t.Fatalf("progress after %d = %v, want %v", i+1, up.progress, want)
					}
					if sink.count("done") != 0 {
						t.Fatal("card closed before all workers terminal")
					}
				}
			}
			dn, ok := sink.lastOf("done")
			if !ok || dn.state != tc.wantFinal {
				t.Fatalf("final = %+v ok=%v, want state %s", dn, ok, tc.wantFinal)
			}
			if len(d.active.Batches) != 0 {
				t.Fatal("completed batch not pruned")
			}
		})
	}
}

func TestBatchOfOneRendersLikeSingle(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()
	enqueue(t, d, Intent{Action: "start", Cookie: "solo", Batch: "b", Title: "Just me"})
	enqueue(t, d, Intent{Action: "done", Cookie: "solo", Batch: "b", State: "ok"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 1 || sink.count("done") != 1 {
		t.Fatalf("batch-of-one ops: %+v", sink.ops)
	}
	dn, _ := sink.lastOf("done")
	if dn.state != "ok" {
		t.Fatalf("final state %q", dn.state)
	}
}

func TestOrphanSweepOnRestart(t *testing.T) {
	d, dir := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	// Boot 1: start two workers, card them, then "crash" (drop the dispatcher).
	enqueue(t, d, Intent{Action: "start", Cookie: "a", Batch: "sess", Title: "Long job"})
	enqueue(t, d, Intent{Action: "start", Cookie: "b", Batch: "sess", Title: "Long job"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 1 {
		t.Fatalf("want 1 card, got %d", sink.count("start"))
	}

	// Boot 2: fresh dispatcher over the same activejobs.json, fresh (empty) sink
	// registry — the card message survived but its in-memory row did not.
	d2 := NewDispatcher(filepath.Join(dir, "spool.json"), filepath.Join(dir, "activejobs.json"))
	d2.now = func() time.Time { return time.Unix(2_000, 0) }
	sink2 := newFakeSink()
	d2.load()
	d2.sweepOrphans()
	for cookie, job := range d2.active.Batches["sess"].Jobs {
		if job.State != "cancelled" || job.Detail != "box restarted" {
			t.Fatalf("swept job %s = state %q detail %q", cookie, job.State, job.Detail)
		}
	}
	d2.dispatch(ctx, sink2)

	// The orphaned card is rehydrated then closed with the same restart
	// cancellation semantics as the app-owned card registry.
	if sink2.count("rehydrate") != 1 {
		t.Fatalf("want 1 rehydrate, got %d (%+v)", sink2.count("rehydrate"), sink2.ops)
	}
	dn, ok := sink2.lastOf("done")
	if !ok || dn.state != "cancelled" || dn.detail != "box restarted" {
		t.Fatalf("orphan close = %+v ok=%v", dn, ok)
	}
	if len(d2.active.Batches) != 0 {
		t.Fatal("swept batch not pruned")
	}
}

func TestReaperClosesStaleJob(t *testing.T) {
	d, _ := newTestDispatcher(t)
	d.lease = 10 * time.Minute
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "slow", Title: "No done hook"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 1 {
		t.Fatal("card not started")
	}
	// Advance past the lease with no update; the reaper must close it — as
	// CANCELLED, never err. The reaper knows the job went quiet, not that it
	// failed, and the app paints err as a red "failed" pill.
	d.now = func() time.Time { return time.Unix(1_000+11*60, 0) }
	d.dispatch(ctx, sink)
	dn, ok := sink.lastOf("done")
	if !ok || dn.state != "cancelled" || dn.detail != "tracking lost" {
		t.Fatalf("reaper close = %+v ok=%v", dn, ok)
	}
}

// TestReapedSuccessNeverCardsAsFailure is the job-143 regression: a fan-out that
// really succeeded, whose completion signal never arrived, must not tell the
// operator the work failed. Reaped members are cancelled "tracking lost", and
// the rollup reads err only when a member genuinely REPORTED an error.
func TestReapedSuccessNeverCardsAsFailure(t *testing.T) {
	d, _ := newTestDispatcher(t)
	d.lease = 10 * time.Minute
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "build", Batch: "sess", Title: "Build fleet authority grant"})
	d.dispatch(ctx, sink)

	d.now = func() time.Time { return time.Unix(1_000+11*60, 0) }
	d.dispatch(ctx, sink)

	dn, ok := sink.lastOf("done")
	if !ok {
		t.Fatal("reaper never closed the card")
	}
	if dn.state == "err" {
		t.Fatalf("a job the box merely lost track of carded as a failure: %+v", dn)
	}
	if dn.state != "cancelled" || dn.detail != "tracking lost" {
		t.Fatalf("want cancelled/tracking lost, got %+v", dn)
	}

	// And a batch where a worker DID report an error still reads err, so the
	// honest signal is not lost along with the dishonest one.
	d2, _ := newTestDispatcher(t)
	s2 := newFakeSink()
	enqueue(t, d2, Intent{Action: "start", Cookie: "w", Batch: "b", Title: "real failure"})
	d2.dispatch(ctx, s2)
	enqueue(t, d2, Intent{Action: "done", Cookie: "w", Batch: "b", State: "err"})
	d2.dispatch(ctx, s2)
	if dn2, ok := s2.lastOf("done"); !ok || dn2.state != "err" {
		t.Fatalf("a reported error must still card as err, got %+v ok=%v", dn2, ok)
	}
}

func TestStartCardRetriesWhenNoDevice(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	sink.failStart = true
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "waiting for device"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 0 {
		t.Fatal("should not have carded without a device")
	}
	if len(d.active.Batches) != 1 {
		t.Fatal("batch state should persist for retry")
	}
	// Device links: an update intent re-drives the batch and the card now opens.
	sink.failStart = false
	enqueue(t, d, Intent{Action: "update", Cookie: "c1", Detail: "still going"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 1 {
		t.Fatalf("card should open on retry, ops=%+v", sink.ops)
	}
}

func TestDoneRetriesWhenCloseFails(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "job"})
	d.dispatch(ctx, sink)
	sink.failDone = true
	enqueue(t, d, Intent{Action: "done", Cookie: "c1", State: "ok"})
	d.dispatch(ctx, sink)
	if sink.count("done") != 0 {
		t.Fatal("done should have failed")
	}
	if len(d.active.Batches) != 1 {
		t.Fatal("batch must survive a failed close for retry")
	}
	// Recover: next tick re-drives and closes.
	sink.failDone = false
	d.dispatch(ctx, sink)
	if sink.count("done") != 1 || len(d.active.Batches) != 0 {
		t.Fatalf("retry close failed, ops=%+v batches=%d", sink.ops, len(d.active.Batches))
	}
}

func TestUnknownUpdateDropped(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()
	// update/done with no prior start for the batch: update dropped, done synthesizes.
	enqueue(t, d, Intent{Action: "update", Cookie: "ghost", Batch: "none", Detail: "x"})
	d.dispatch(ctx, sink)
	if len(sink.ops) != 0 {
		t.Fatalf("stray update should be dropped, ops=%+v", sink.ops)
	}
}

func TestFireAndForgetDone(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()
	// A done with no prior start still cards (start+done coalesced) via synthesis.
	enqueue(t, d, Intent{Action: "done", Cookie: "quick", Title: "did a thing", State: "ok"})
	d.dispatch(ctx, sink)
	if sink.count("start") != 1 || sink.count("done") != 1 {
		t.Fatalf("fire-and-forget ops=%+v", sink.ops)
	}
}

// TestDoneWithoutBatchResolvesCookie is the FB13 phantom-card regression: a rollup
// is opened with an explicit --batch, then its workers finish batch-LESS (only the
// cookie). Each done must land on the real rollup and close it — never spawn a
// throwaway self-batch card and leave the rollup running forever.
func TestDoneWithoutBatchResolvesCookie(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	for _, c := range []string{"w1", "w2"} {
		enqueue(t, d, Intent{Action: "start", Cookie: c, Batch: "sess", Title: "Fan out"})
	}
	d.dispatch(ctx, sink)
	if got := sink.count("start"); got != 1 {
		t.Fatalf("want 1 rollup card, got %d starts", got)
	}

	// Both workers report done with NO batch — only the cookie.
	enqueue(t, d, Intent{Action: "done", Cookie: "w1", State: "ok"})
	d.dispatch(ctx, sink)
	if _, ok := d.active.Batches["w1"]; ok {
		t.Fatal("done-without-batch must not spawn a self-batch card for the cookie")
	}
	if sink.count("done") != 0 {
		t.Fatal("rollup closed before every worker terminal")
	}
	enqueue(t, d, Intent{Action: "done", Cookie: "w2", State: "ok"})
	d.dispatch(ctx, sink)

	if sink.count("start") != 1 || sink.count("done") != 1 {
		t.Fatalf("want exactly one card started and closed, ops=%+v", sink.ops)
	}
	if len(d.active.Batches) != 0 {
		t.Fatalf("real rollup should be pruned once resolved, have %d", len(d.active.Batches))
	}
}

// TestUpdateWithoutBatchResolvesCookie is the same correlation for `update`.
func TestUpdateWithoutBatchResolvesCookie(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "w1", Batch: "sess", Title: "Fan out"})
	d.dispatch(ctx, sink)

	enqueue(t, d, Intent{Action: "update", Cookie: "w1", Detail: "reading files"})
	d.dispatch(ctx, sink)

	if _, ok := d.active.Batches["w1"]; ok {
		t.Fatal("update-without-batch must not self-batch the cookie")
	}
	b := d.active.Batches["sess"]
	if b == nil || b.Jobs["w1"].Detail != "reading files" {
		t.Fatalf("update should land on the sess rollup, batches=%+v", d.active.Batches)
	}
}

// TestUnknownCookieStillSelfBatches keeps the fire-and-forget fallback: a cookie no
// active batch has seen self-batches (a batch of one keyed on the cookie).
func TestUnknownCookieStillSelfBatches(t *testing.T) {
	d, _ := newTestDispatcher(t)
	if got := d.resolveBatchKey(Intent{Action: "done", Cookie: "orphan"}); got != "orphan" {
		t.Fatalf("unknown cookie should self-batch, got %q", got)
	}
	// An explicit batch is always honored verbatim.
	if got := d.resolveBatchKey(Intent{Action: "start", Cookie: "c", Batch: "explicit"}); got != "explicit" {
		t.Fatalf("explicit batch must win, got %q", got)
	}
}

// TestAmbiguousCookiePicksMostRecent covers the shouldn't-happen case of one cookie
// live in two batches: resolution picks the batch whose copy is freshest.
func TestAmbiguousCookiePicksMostRecent(t *testing.T) {
	d, _ := newTestDispatcher(t)
	d.active = &ActiveDoc{Batches: map[string]*Batch{
		"older": {Batch: "older", Jobs: map[string]*Job{"dup": {Cookie: "dup", State: stateRunning, UpdatedAt: 100}}},
		"newer": {Batch: "newer", Jobs: map[string]*Job{"dup": {Cookie: "dup", State: stateRunning, UpdatedAt: 200}}},
	}}
	if got := d.resolveBatchKey(Intent{Action: "done", Cookie: "dup"}); got != "newer" {
		t.Fatalf("ambiguous cookie should resolve to freshest batch, got %q", got)
	}
}

// TestAgentIDClosesBackgroundJob covers the real closure path for asynchronous
// work: the card opens under the dispatch-side cookie, an update binds the
// harness's agent id to it, and the completion event — which knows only that
// agent id — closes it.
func TestAgentIDClosesBackgroundJob(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "toolu_1", Batch: "sess", Title: "Explore auth"})
	d.dispatch(ctx, sink)
	// Launch returns the agent id; bind it.
	enqueue(t, d, Intent{Action: "update", Cookie: "toolu_1", Batch: "sess", Agent: "agent_a1"})
	d.dispatch(ctx, sink)
	if sink.count("done") != 0 {
		t.Fatal("binding the agent id must not close the card")
	}

	// Completion, addressed by agent id alone — no cookie, no batch.
	enqueue(t, d, Intent{Action: "done", Agent: "agent_a1", State: "ok"})
	d.dispatch(ctx, sink)
	dn, ok := sink.lastOf("done")
	if !ok || dn.state != "ok" {
		t.Fatalf("agent-addressed done should have closed the card ok, got %+v ok=%v", dn, ok)
	}
	if len(d.active.Batches) != 0 {
		t.Fatalf("closed batch should be pruned, got %+v", d.active.Batches)
	}
	if len(d.active.Agents) != 0 {
		t.Fatalf("pruning a batch must drop its agent bindings, got %+v", d.active.Agents)
	}
}

// TestUnboundAgentDoneIsDropped: a completion for an agent this box never carded
// (a nested subagent, another project's session, or a card already closed) must
// vanish, not mint a phantom card that opens and instantly closes.
func TestUnboundAgentDoneIsDropped(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "done", Agent: "agent_unknown", State: "ok"})
	d.dispatch(ctx, sink)
	if len(sink.ops) != 0 {
		t.Fatalf("unbound agent completion drove the card surface: %+v", sink.ops)
	}
	if len(d.active.Batches) != 0 {
		t.Fatalf("unbound agent completion opened a batch: %+v", d.active.Batches)
	}
}

// TestBatchSealsSoALaterDispatchGetsAFreshCard: while a fan-out is live, extra
// dispatches roll up into its one card (the intended behaviour); once every
// member is terminal the batch seals, so the NEXT dispatch under the same caller
// key opens a card of its own instead of resurrecting the closed one.
func TestBatchSealsSoALaterDispatchGetsAFreshCard(t *testing.T) {
	d, _ := newTestDispatcher(t)
	sink := newFakeSink()
	ctx := context.Background()
	const sess = "sess"

	// Fan-out of two under one key → ONE card.
	enqueue(t, d, Intent{Action: "start", Cookie: "a", Batch: sess, Title: "first"})
	enqueue(t, d, Intent{Action: "start", Cookie: "b", Batch: sess, Title: "second"})
	d.dispatch(ctx, sink)
	if got := sink.count("start"); got != 1 {
		t.Fatalf("one fan-out should be one card, got %d", got)
	}

	// Close both in the SAME cycle as the next dispatch's start, the race the
	// seal has to survive: the batch is still present and allTerminal when the
	// start is applied.
	enqueue(t, d, Intent{Action: "done", Cookie: "a", Batch: sess, State: "ok"})
	enqueue(t, d, Intent{Action: "done", Cookie: "b", Batch: sess, State: "ok"})
	enqueue(t, d, Intent{Action: "start", Cookie: "c", Batch: sess, Title: "much later"})
	d.dispatch(ctx, sink)

	if got := sink.count("start"); got != 2 {
		t.Fatalf("the later dispatch needs its own card, got %d starts", got)
	}
	if got := sink.count("done"); got != 1 {
		t.Fatalf("the sealed batch should close exactly once, got %d", got)
	}
	st, _ := sink.lastOf("start")
	if st.title != "much later" {
		t.Fatalf("the fresh card should wear its own title, got %q", st.title)
	}
	// The live batch is the new one, under the caller's un-suffixed key.
	if b := d.active.Batches[sess]; b == nil || len(b.Jobs) != 1 || b.Jobs["c"] == nil {
		t.Fatalf("caller key should hold only the new fan-out, got %+v", d.active.Batches[sess])
	}
}

func abs(f float64) float64 {
	if f < 0 {
		return -f
	}
	return f
}
