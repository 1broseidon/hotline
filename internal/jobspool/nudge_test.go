package jobspool

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// nudgeMsg records one synthetic inbound turn the dispatcher injected.
type nudgeMsg struct {
	content string
	meta    map[string]string
}

// fakeNudgeSink is a scriptable NudgeSink: it records every injected turn and can
// be told to fail delivery to exercise the retry-next-tick path.
type fakeNudgeSink struct {
	msgs []nudgeMsg
	fail bool
}

func (s *fakeNudgeSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	if s.fail {
		return errors.New("no active device")
	}
	s.msgs = append(s.msgs, nudgeMsg{content: content, meta: meta})
	return nil
}

// armedDispatcher builds a dispatcher with the nudge sink bound and a short
// interval, over temp paths with a fixed clock.
func armedDispatcher(t *testing.T, every time.Duration) (*Dispatcher, *fakeNudgeSink, string) {
	t.Helper()
	d, dir := newTestDispatcher(t)
	// A long lease by default so nudge-timing assertions aren't cut short by the
	// reaper (the reaper-interaction test overrides this).
	d.lease = time.Hour
	sink := &fakeNudgeSink{}
	d.ConfigureNudge([]string{"app"}, nil, every)
	d.SetNudgeSink(sink)
	return d, sink, dir
}

// TestNudgeArmsWhileRunning: a carded batch that sits running past the interval
// gets a check-in; before the interval it stays silent.
func TestNudgeArmsWhileRunning(t *testing.T) {
	d, nudge, _ := armedDispatcher(t, 15*time.Minute)
	card := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "Refactor auth"})
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 0 {
		t.Fatalf("nudged immediately, want silence until the interval: %+v", nudge.msgs)
	}

	// 10m in: still under the interval, no nudge.
	d.now = func() time.Time { return time.Unix(1_000+10*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 0 {
		t.Fatalf("nudged before the interval elapsed: %+v", nudge.msgs)
	}

	// 16m in: past the interval, exactly one check-in.
	d.now = func() time.Time { return time.Unix(1_000+16*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 1 {
		t.Fatalf("want 1 nudge past the interval, got %d", len(nudge.msgs))
	}
	m := nudge.msgs[0]
	if !strings.Contains(m.content, "job-1") || !strings.Contains(m.content, "Refactor auth") {
		t.Fatalf("nudge content missing id/title: %q", m.content)
	}
	if !strings.Contains(m.content, "16m") {
		t.Fatalf("nudge should report the card's true age (16m): %q", m.content)
	}
	if m.meta["kind"] != "job_nudge" || m.meta["job_id"] != "job-1" || m.meta["source"] != "app" || m.meta["chat_id"] == "" {
		t.Fatalf("nudge meta wrong: %+v", m.meta)
	}

	// Another 16m without any change: the timer measures from the last nudge, so
	// exactly one more fires (not one per 2s tick).
	d.now = func() time.Time { return time.Unix(1_000+32*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 2 {
		t.Fatalf("want a second nudge one interval after the first, got %d", len(nudge.msgs))
	}
	// A tick right after does NOT re-fire (still inside the interval since the last).
	d.now = func() time.Time { return time.Unix(1_000+32*60+2, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 2 {
		t.Fatalf("nudge re-fired inside the interval: %d", len(nudge.msgs))
	}
}

// TestNudgeDisarmsOnDone: the moment a card reaches a terminal state it stops
// nudging (and the per-batch timer is dropped).
func TestNudgeDisarmsOnDone(t *testing.T) {
	d, nudge, _ := armedDispatcher(t, 15*time.Minute)
	card := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "job"})
	d.dispatch(ctx, card)
	// One nudge lands after the interval.
	d.now = func() time.Time { return time.Unix(1_000+16*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 1 {
		t.Fatalf("want 1 nudge while running, got %d", len(nudge.msgs))
	}

	// Close the card. It's pruned, so no further nudge can ever fire.
	enqueue(t, d, Intent{Action: "done", Cookie: "c1", State: "ok"})
	d.dispatch(ctx, card)
	if _, ok := d.lastNudged["c1"]; ok {
		t.Fatal("nudge timer should be dropped when the card is pruned")
	}
	before := len(nudge.msgs)
	d.now = func() time.Time { return time.Unix(1_000+40*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != before {
		t.Fatalf("terminal card kept nudging: %d -> %d", before, len(nudge.msgs))
	}
}

// TestNudgeDisarmsOnReaper: the lease reaper closing a stale card also disarms
// the nudge — no reminder after the card is force-closed.
func TestNudgeDisarmsOnReaper(t *testing.T) {
	d, nudge, _ := armedDispatcher(t, 15*time.Minute)
	d.lease = 20 * time.Minute
	card := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "slow", Title: "no done hook"})
	d.dispatch(ctx, card)

	// Past the lease: the reaper closes the card cancelled "tracking lost" and
	// prunes it.
	d.now = func() time.Time { return time.Unix(1_000+21*60, 0) }
	d.dispatch(ctx, card)
	if dn, ok := card.lastOf("done"); !ok || dn.state != "cancelled" {
		t.Fatalf("reaper should have closed the card, got %+v ok=%v", dn, ok)
	}
	// The reaped card must not have nudged this same tick, and never again.
	if len(nudge.msgs) != 0 {
		t.Fatalf("reaped card should not nudge: %+v", nudge.msgs)
	}
	d.now = func() time.Time { return time.Unix(1_000+60*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 0 {
		t.Fatalf("nudge fired after the reaper closed the card: %+v", nudge.msgs)
	}
}

// TestNudgeReArmsOnBoot: a fresh dispatcher over an activejobs.json whose card
// is still running re-arms from durable state and nudges with the card's true
// age — the restart-survival property (in-memory timer reset, StartedAt-based).
func TestNudgeReArmsOnBoot(t *testing.T) {
	dir := t.TempDir()
	active := filepath.Join(dir, "activejobs.json")

	// Persist a batch that was carded and left running 40m ago (StartedAt=1000).
	seed := &ActiveDoc{Batches: map[string]*Batch{
		"sess": {
			Batch: "sess", JobID: "job-7", ChatID: "app", Title: "Long build",
			MessageID: "a-7", ElementID: "el-job-7", StartedAt: 1_000,
			Order: []string{"w1"},
			Jobs:  map[string]*Job{"w1": {Cookie: "w1", State: stateRunning, StartedAt: 1_000, UpdatedAt: 1_000}},
		},
	}}
	if err := SaveActive(seed, active); err != nil {
		t.Fatalf("seed active: %v", err)
	}

	// Boot 2: fresh dispatcher, lease long enough that the reaper won't fire.
	d := NewDispatcher(filepath.Join(dir, "spool.json"), active)
	d.lease = time.Hour
	bootAt := int64(1_000 + 40*60)
	d.now = func() time.Time { return time.Unix(bootAt, 0) }
	nudge := &fakeNudgeSink{}
	d.ConfigureNudge([]string{"app"}, nil, 15*time.Minute)
	d.SetNudgeSink(nudge)
	card := newFakeSink()
	ctx := context.Background()

	// One reconcile cycle after load. (No sweepOrphans here: this exercises the
	// re-arm for a card that survives running, e.g. a warm restart path.)
	d.load()
	d.dispatch(ctx, card)

	if len(nudge.msgs) != 1 {
		t.Fatalf("want a prompt re-armed nudge for the surviving card, got %d", len(nudge.msgs))
	}
	if got := nudge.msgs[0].content; !strings.Contains(got, "job-7") || !strings.Contains(got, "40m") {
		t.Fatalf("re-armed nudge should report the card's true age: %q", got)
	}
	// The card is rehydrated, not re-created.
	if card.count("rehydrate") != 1 {
		t.Fatalf("want the surviving card rehydrated once, got %d", card.count("rehydrate"))
	}
}

// TestNudgeDisabled: a zero/negative interval, or a nil sink, injects nothing.
func TestNudgeDisabled(t *testing.T) {
	ctx := context.Background()
	run := func(t *testing.T, configure func(d *Dispatcher, sink *fakeNudgeSink)) int {
		d, _ := newTestDispatcher(t)
		sink := &fakeNudgeSink{}
		configure(d, sink)
		card := newFakeSink()
		enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "job"})
		d.dispatch(ctx, card)
		d.now = func() time.Time { return time.Unix(1_000+60*60, 0) }
		d.dispatch(ctx, card)
		return len(sink.msgs)
	}

	if n := run(t, func(d *Dispatcher, s *fakeNudgeSink) {
		d.ConfigureNudge([]string{"app"}, nil, 0) // disabled interval
		d.SetNudgeSink(s)
	}); n != 0 {
		t.Fatalf("zero interval should disable the nudge, got %d", n)
	}
	if n := run(t, func(d *Dispatcher, s *fakeNudgeSink) {
		d.ConfigureNudge([]string{"app"}, nil, 15*time.Minute) // no sink bound
	}); n != 0 {
		t.Fatalf("nil sink should disable the nudge, got %d", n)
	}
}

// TestNudgeRetriesOnSinkFailure: a failed injection is not counted as sent, so
// the next tick past the interval re-attempts it.
func TestNudgeRetriesOnSinkFailure(t *testing.T) {
	d, nudge, _ := armedDispatcher(t, 15*time.Minute)
	card := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "job"})
	d.dispatch(ctx, card)

	nudge.fail = true
	d.now = func() time.Time { return time.Unix(1_000+16*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 0 {
		t.Fatalf("failed injection should record nothing, got %+v", nudge.msgs)
	}
	if _, ok := d.lastNudged["c1"]; ok {
		t.Fatal("a failed nudge must not advance the timer")
	}

	// Recover: the very next tick (still past interval-from-start) re-fires.
	nudge.fail = false
	d.now = func() time.Time { return time.Unix(1_000+16*60+2, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 1 {
		t.Fatalf("nudge should retry after the sink recovers, got %d", len(nudge.msgs))
	}
}

// TestNudgeNeverRoutesToACardBlindChannel: with no card-capable source
// configured the dispatcher stays silent, however long a card sits running.
//
// The wiring hands ConfigureNudge only Router.CardSources(). On a telegram-only
// box that list is empty, and a check-in there would buzz the operator about a
// card telegram cannot render. Silence is the correct behaviour, not a fallback
// to whichever provider happens to be first.
func TestNudgeNeverRoutesToACardBlindChannel(t *testing.T) {
	d, _ := newTestDispatcher(t)
	d.lease = time.Hour
	sink := &fakeNudgeSink{}
	d.ConfigureNudge(nil, nil, 15*time.Minute) // no card-capable channel attached
	d.SetNudgeSink(sink)
	card := newFakeSink()
	ctx := context.Background()

	enqueue(t, d, Intent{Action: "start", Cookie: "c1", Title: "Diagnose relay push-wake 503"})
	d.dispatch(ctx, card)
	for m := 16; m <= 8*60; m += 16 {
		d.now = func() time.Time { return time.Unix(int64(1_000+m*60), 0) }
		d.dispatch(ctx, card)
	}
	if len(sink.msgs) != 0 {
		t.Fatalf("nudged %d times with no card-capable channel attached: %+v", len(sink.msgs), sink.msgs[0])
	}
}

// TestNudgeStopsWhenTheFanOutFinishes is the job-142 regression: the exact
// eight-hour phantom loop.
//
// A long-lived orchestrator session dispatches subagents under ONE caller batch
// key (Claude Code passes its session id). Before sealing, each new dispatch
// rejoined the session's original batch, which therefore never reached
// allTerminal — so nudge() stayed armed for the life of the session and fired
// every interval for hours after the work was done, quoting the FIRST dispatch's
// title. Sealing bounds a batch to one fan-out: once its members are terminal
// the card closes, the nudge disarms permanently, and the next dispatch opens a
// fresh card of its own.
func TestNudgeStopsWhenTheFanOutFinishes(t *testing.T) {
	d, nudge, _ := armedDispatcher(t, 15*time.Minute)
	d.lease = time.Hour
	card := newFakeSink()
	ctx := context.Background()
	const sess = "session-abc" // one session id for the whole run

	enqueue(t, d, Intent{Action: "start", Cookie: "t1", Batch: sess, Title: "Diagnose relay push-wake 503"})
	d.dispatch(ctx, card)
	enqueue(t, d, Intent{Action: "done", Cookie: "t1", Batch: sess, State: "ok"})
	d.dispatch(ctx, card)

	// Eight hours of ticks. The fan-out is finished, so not one check-in may fire.
	for m := 15; m <= 8*60; m += 15 {
		d.now = func() time.Time { return time.Unix(int64(1_000+m*60), 0) }
		d.dispatch(ctx, card)
	}
	if len(nudge.msgs) != 0 {
		t.Fatalf("finished fan-out nudged %d times over 8h (job-142 phantom loop): %+v", len(nudge.msgs), nudge.msgs[0])
	}

	// A later dispatch on the SAME session key opens a FRESH card with its own
	// title, rather than resurrecting the closed one.
	d.now = func() time.Time { return time.Unix(1_000+9*60*60, 0) }
	enqueue(t, d, Intent{Action: "start", Cookie: "t2", Batch: sess, Title: "Build fleet authority grant"})
	d.dispatch(ctx, card)
	if card.count("start") != 2 {
		t.Fatalf("want a second, fresh card for the later dispatch, got %d starts", card.count("start"))
	}
	st, _ := card.lastOf("start")
	if st.title != "Build fleet authority grant" {
		t.Fatalf("fresh card wore the first dispatch's title: %q", st.title)
	}
	// And that new card nudges normally — sealing disarms the old batch, it does
	// not disable the feature.
	d.now = func() time.Time { return time.Unix(1_000+9*60*60+16*60, 0) }
	d.dispatch(ctx, card)
	if len(nudge.msgs) != 1 {
		t.Fatalf("the fresh card should nudge once past the interval, got %d", len(nudge.msgs))
	}
	if !strings.Contains(nudge.msgs[0].content, "Build fleet authority grant") {
		t.Fatalf("nudge quoted the wrong card: %q", nudge.msgs[0].content)
	}
}

// sanity: content is a self-authored, non-user framing.
func TestNudgeContentFraming(t *testing.T) {
	b := &Batch{JobID: "job-3", Title: "Explore", ChatID: "app"}
	got := nudgeContent(b, 22*time.Minute)
	for _, want := range []string{"job-3", "Explore", "22m", "automatic check-in", "not a new message from the user"} {
		if !strings.Contains(got, want) {
			t.Fatalf("nudge content missing %q: %q", want, got)
		}
	}
	// Empty title falls back rather than rendering an empty quoted string.
	if g := nudgeContent(&Batch{JobID: "job-4"}, time.Minute); !strings.Contains(g, "Subagent work") {
		t.Fatalf("empty-title nudge should fall back: %q", g)
	}
}
