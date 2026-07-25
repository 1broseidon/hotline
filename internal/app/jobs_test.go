package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

func TestJobLifecycleStartUpdateDoneNotify(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()

	// start
	msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "header pass", Detail: "compiling"})
	if isErr {
		t.Fatalf("job start failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "msg" {
		t.Fatalf("start should post one msg, got %+v", items)
	}
	els := items[0]["elements"].([]any)
	el0 := els[0].(map[string]any)
	if el0["el"] != "job" || el0["state"] != "running" || el0["title"] != "header pass" {
		t.Fatalf("start job element wrong: %+v", el0)
	}
	if el0["id"] != "el-job-1" {
		t.Errorf("element id = %v, want el-job-1", el0["id"])
	}
	// E6: the message text is the synthesized fallback (old clients render it,
	// and it is the push preview).
	if items[0]["text"] != "header pass: compiling" {
		t.Errorf("start message text = %q, want the synthesized fallback", items[0]["text"])
	}
	// registry now reports one active run
	runs := srv.jobs.activeRuns(maxAgentRuns)
	if len(runs) != 1 || runs[0].ID != "job-1" || runs[0].State != "running" {
		t.Fatalf("activeRuns after start = %+v", runs)
	}

	// update — element-only edit, no buzz
	p := 0.5
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "update", JobID: "job-1", Detail: "linking", Progress: &p}); isErr {
		t.Fatalf("job update failed: %s", msg)
	}
	items = drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "edit" {
		t.Fatalf("update should be one edit, got %+v", items)
	}
	if items[0]["text"] != "header pass: linking" {
		t.Errorf("update edit text = %q, want the synthesized fallback (E6)", items[0]["text"])
	}
	uel := items[0]["elements"].([]any)[0].(map[string]any)
	if uel["state"] != "running" || uel["detail"] != "linking" || uel["progress"].(float64) != 0.5 {
		t.Errorf("update element wrong: %+v", uel)
	}

	// done + notify — final edit FIRST, then a fresh buzzing message
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok", Detail: "shipped", Notify: "header pass done ✅"}); isErr {
		t.Fatalf("job done failed: %s", msg)
	}
	items = drainItems(t, sub)
	if len(items) != 2 {
		t.Fatalf("done+notify should be edit then msg, got %d: %+v", len(items), items)
	}
	if items[0]["t"] != "edit" {
		t.Errorf("first frame after done should be the terminal edit, got %v", items[0]["t"])
	}
	dEl := items[0]["elements"].([]any)[0].(map[string]any)
	if dEl["state"] != "ok" || dEl["detail"] != "shipped" {
		t.Errorf("terminal element wrong: %+v", dEl)
	}
	if items[1]["t"] != "msg" || items[1]["text"] != "header pass done ✅" {
		t.Errorf("notify message wrong: %+v", items[1])
	}

	// run has left the active set AND the registry itself (P2-4); a duplicate
	// done gets a clear already-finished error, not "unknown".
	if runs := srv.jobs.activeRuns(maxAgentRuns); len(runs) != 0 {
		t.Fatalf("activeRuns after done = %+v, want empty", runs)
	}
	if _, live := srv.jobs.jobs["job-1"]; live {
		t.Fatal("terminal job must leave the registry")
	}
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok"}); !isErr {
		t.Fatal("duplicate done must error")
	} else if !strings.Contains(msg, "already finished (ok)") {
		t.Errorf("duplicate done error = %q, want already-finished", msg)
	}
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "update", JobID: "job-1", Detail: "late"}); !isErr {
		t.Fatal("update after done must error")
	} else if !strings.Contains(msg, "already finished") {
		t.Errorf("late update error = %q", msg)
	}
}

func TestJobUnknownIDErrors(t *testing.T) {
	_, tools, dev, _ := activeHarness(t)
	for _, action := range []string{"update", "done"} {
		in := mcpchan.JobInput{ChatID: dev, Action: action, JobID: "job-404", State: "ok"}
		if msg, isErr := tools.Job(context.Background(), in); !isErr {
			t.Errorf("%s unknown job should error, got %q", action, msg)
		}
	}
}

func TestJobDoneRejectsBadState(t *testing.T) {
	_, tools, dev, _ := activeHarness(t)
	if _, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: dev, Action: "start", Title: "x"}); isErr {
		t.Fatal("start failed")
	}
	if msg, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "meh"}); !isErr {
		t.Errorf("bad done state should error, got %q", msg)
	}
}

// TestJobLifecyclePushCounts pins E10 plus FB44 end-to-end for an AWAY device
// (no live subscriber): start = exactly one banner, update = zero, bare ok done
// = exactly one custom completion, done+notify = exactly one fresh notify (not
// two pushes).
func TestJobLifecyclePushCounts(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	// Away: drop the live subscription the harness opened.
	srv.mailbox.unsubscribe(dev, sub)

	token := "ExponentPushToken[job-push-counts]"
	if err := srv.store.SetPush(dev, token, "ios"); err != nil {
		t.Fatal(err)
	}
	got := make(chan map[string]any, 8)
	expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		got <- body
		w.WriteHeader(http.StatusOK)
	}))
	defer expo.Close()
	srv.pushEndpoint = expo.URL

	expect := func(phase string, n int) []map[string]any {
		t.Helper()
		var bodies []map[string]any
		for i := 0; i < n; i++ {
			select {
			case b := <-got:
				bodies = append(bodies, b)
			case <-time.After(2 * time.Second):
				t.Fatalf("%s: got %d push(es), want %d", phase, i, n)
			}
		}
		select {
		case b := <-got:
			t.Fatalf("%s: extra push %v", phase, b)
		case <-time.After(150 * time.Millisecond):
		}
		return bodies
	}
	ctx := context.Background()

	// start → exactly one banner, previewing the synthesized fallback.
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "header pass", Detail: "compiling"}); isErr {
		t.Fatalf("start failed: %s", msg)
	}
	bodies := expect("start", 1)
	if bodies[0]["body"] != "header pass: compiling" {
		t.Errorf("start push body = %v, want the synthesized fallback", bodies[0]["body"])
	}

	// update → silent.
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "update", JobID: "job-1", Detail: "linking"}); isErr {
		t.Fatalf("update failed: %s", msg)
	}
	expect("update", 0)

	// second job for the bare-done case. The final detail is blank, so the
	// custom completion uses the exact job title plus the "Completed" fallback.
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "quiet job"}); isErr {
		t.Fatalf("second start failed: %s", msg)
	}
	expect("second start", 1)
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-2", State: "ok"}); isErr {
		t.Fatalf("bare done failed: %s", msg)
	}
	bodies = expect("bare done", 1)
	if bodies[0]["title"] != "quiet job" || bodies[0]["body"] != "Completed" {
		t.Errorf("bare done push = title %q body %q, want %q/%q", bodies[0]["title"], bodies[0]["body"], "quiet job", "Completed")
	}

	// done+notify → exactly one banner, from the fresh notify message. The
	// terminal edit carries no automatic completion intent, so this cannot buzz
	// twice. Opting out of automatic completion pushes must not suppress notify.
	if err := srv.store.SetDeviceJobCompletionPush(dev, false); err != nil {
		t.Fatal(err)
	}
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok", Notify: "header pass done"}); isErr {
		t.Fatalf("done+notify failed: %s", msg)
	}
	bodies = expect("done+notify", 1)
	if bodies[0]["body"] != "header pass done" {
		t.Errorf("notify push body = %v, want the message text", bodies[0]["body"])
	}

	// err/cancelled stay silent only when bare. A notify on either state remains
	// one normal message push, independent of the automatic-completion opt-out.
	for _, tc := range []struct {
		state string
		jobID string
	}{
		{state: "err", jobID: "job-3"},
		{state: "cancelled", jobID: "job-4"},
	} {
		if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: tc.state + " job"}); isErr {
			t.Fatalf("%s start failed: %s", tc.state, msg)
		}
		expect(tc.state+" start", 1)
		notify := tc.state + " job finished"
		if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: tc.jobID, State: tc.state, Notify: notify}); isErr {
			t.Fatalf("%s done+notify failed: %s", tc.state, msg)
		}
		bodies = expect(tc.state+" done+notify", 1)
		if bodies[0]["body"] != notify {
			t.Errorf("%s notify push body = %v, want %q", tc.state, bodies[0]["body"], notify)
		}
	}
}

// A card's message id (a-NNNN) resolves update/done just like the minted job id.
// It is the recovery path for the id an operator can actually see: the job id is
// only ever returned once, in the start reply, and a caller who loses it (notes,
// a restarted orchestrator) can still read a-NNNN off the card and close it.
func TestJobResolvesByMessageID(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()

	reply, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "sol lane", Detail: "dispatched"})
	if isErr {
		t.Fatalf("job start failed: %s", reply)
	}
	drainItems(t, sub)
	msgID := srv.jobs.jobs["job-1"].messageID
	if msgID == "" || !strings.HasPrefix(msgID, "a-") {
		t.Fatalf("start did not mint an a-NNNN message id: %q", msgID)
	}
	if !strings.Contains(reply, msgID) {
		t.Errorf("start reply %q should name message id %s", reply, msgID)
	}

	// update by message id — resolves, and answers with the canonical job id.
	upd, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "update", JobID: msgID, Detail: "linking"})
	if isErr {
		t.Fatalf("update by message id failed: %s", upd)
	}
	if !strings.Contains(upd, "job-1") {
		t.Errorf("update reply = %q, want the canonical job_id job-1", upd)
	}
	items := drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "edit" {
		t.Fatalf("update by message id should emit one edit, got %+v", items)
	}
	if uel := items[0]["elements"].([]any)[0].(map[string]any); uel["detail"] != "linking" {
		t.Errorf("update element wrong: %+v", uel)
	}

	// done by message id — same card, terminal edit on the original message.
	done, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: msgID, State: "ok", Detail: "landed"})
	if isErr {
		t.Fatalf("done by message id failed: %s", done)
	}
	if !strings.Contains(done, "job-1") {
		t.Errorf("done reply = %q, want the canonical job_id job-1", done)
	}
	items = drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "edit" {
		t.Fatalf("done by message id should emit one edit, got %+v", items)
	}
	dEl := items[0]["elements"].([]any)[0].(map[string]any)
	if dEl["state"] != "ok" || dEl["detail"] != "landed" {
		t.Errorf("terminal element wrong: %+v", dEl)
	}
	if _, live := srv.jobs.jobs["job-1"]; live {
		t.Error("done by message id must remove the record from the registry")
	}

	// The message id is remembered as terminal too, so a repeat says so plainly
	// instead of claiming the card never existed.
	for _, key := range []string{"job-1", msgID} {
		if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: key, State: "ok"}); !isErr {
			t.Fatalf("duplicate done by %s must error", key)
		} else if !strings.Contains(msg, "already finished (ok)") {
			t.Errorf("duplicate done by %s = %q, want already-finished", key, msg)
		}
	}
}

// A card recovered from disk after a restart is resolvable by its message id —
// the orphan case: the box restarted, the minted job id is long gone from the
// caller's context, and a-NNNN on the phone is the only handle left.
func TestJobDoneByMessageIDAfterRestart(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()

	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "flicker hunt"}); isErr {
		t.Fatalf("job start failed: %s", msg)
	}
	drainItems(t, sub)
	msgID := srv.jobs.jobs["job-1"].messageID

	// Simulate the restart sweep: the card survives, marked stale.
	srv.restoreJobCards()
	drainItems(t, sub)
	if rec := srv.jobs.jobs["job-1"]; rec == nil || !rec.stale {
		t.Fatalf("card should survive the restart sweep as stale, got %+v", rec)
	}

	msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: msgID, State: "ok", Detail: "landed as abc1234"})
	if isErr {
		t.Fatalf("closing a restart-swept card by message id failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "edit" {
		t.Fatalf("done should emit one terminal edit, got %+v", items)
	}
	dEl := items[0]["elements"].([]any)[0].(map[string]any)
	if dEl["state"] != "ok" || dEl["detail"] != "landed as abc1234" {
		t.Errorf("terminal element wrong: %+v", dEl)
	}
}

// The automatic (jobspool) path must log a "job start" line, not just a "job
// done" one. The dispatcher opens cards via startCard but closes them via
// jobDone; when only the latter logged, an auto-card left a done line with no
// matching start, and anything pairing those lines — the fleet sweep's
// stale-card check — saw no automatic cards at all.
func TestAutomaticCardLogsStartAndDone(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	driver := &JobDriver{t: tools}

	jobID, msgID, _, err := driver.StartCard("fan-out lane", "dispatched", dev, nil)
	if err != nil {
		t.Fatalf("StartCard failed: %v", err)
	}
	drainItems(t, sub)
	if err := driver.DoneCard(jobID, "ok", "landed", "", dev); err != nil {
		t.Fatalf("DoneCard failed: %v", err)
	}
	drainItems(t, sub)

	raw, err := os.ReadFile(srv.cfg.TranscriptFile)
	if err != nil {
		t.Fatalf("read transcript: %v", err)
	}
	var start, done bool
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		if line == "" {
			continue
		}
		var rec struct {
			Kind      string `json:"kind"`
			MessageID string `json:"message_id"`
			Text      string `json:"text"`
		}
		if err := json.Unmarshal([]byte(line), &rec); err != nil || rec.Kind != "job" {
			continue
		}
		if rec.MessageID != msgID {
			continue
		}
		if rec.Text == "job start: fan-out lane" {
			start = true
		}
		if rec.Text == "job done: fan-out lane (ok)" {
			done = true
		}
	}
	if !start {
		t.Errorf("automatic card logged no %q line for %s", "job start", msgID)
	}
	if !done {
		t.Errorf("automatic card logged no %q line for %s", "job done", msgID)
	}
}

// TestReaperClosedCardIsCorrectableByAnExplicitDone is the job-143 regression on
// the app side: the box force-closed a card that had in fact succeeded, and the
// agent that did the work could not set the record straight.
//
// The reaper's verdict is a guess; the agent's is ground truth. So a card closed
// automatically (JobDriver — reaper, restart sweep, completion hook) accepts a
// later explicit `job done`, which re-edits the SAME card to the reported state.
func TestReaperClosedCardIsCorrectableByAnExplicitDone(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()
	driver := &JobDriver{t: tools}

	if _, _, _, err := driver.StartCard("Build fleet authority grant", "compiling", dev, nil); err != nil {
		t.Fatalf("start card: %v", err)
	}
	drainItems(t, sub)

	// The lease expires; the box closes the card on a guess. Never err — it does
	// not know the build failed, only that it stopped hearing about it.
	if err := driver.DoneCard("job-1", "cancelled", "tracking lost", "", dev); err != nil {
		t.Fatalf("auto close: %v", err)
	}
	drainItems(t, sub)
	if _, live := srv.jobs.jobs["job-1"]; live {
		t.Fatal("auto-closed card should leave the live registry")
	}

	// The agent that ran the build reports the truth. This must be accepted.
	msg, isErr := tools.Job(ctx, mcpchan.JobInput{
		ChatID: dev, Action: "done", JobID: "job-1", State: "ok", Detail: "build green",
	})
	if isErr {
		t.Fatalf("an explicit done must be able to correct a box-closed card, got %q", msg)
	}
	items := drainItems(t, sub)
	if len(items) == 0 {
		t.Fatal("the correction should re-edit the card")
	}
	last := items[len(items)-1]
	els, _ := last["elements"].([]any)
	if len(els) == 0 {
		t.Fatalf("correction carried no element: %+v", last)
	}
	el := els[0].(map[string]any)
	if el["id"] != "el-job-1" {
		t.Fatalf("correction should edit the ORIGINAL card, got element %v", el["id"])
	}
	if el["state"] != "ok" || el["detail"] != "build green" {
		t.Fatalf("corrected card = %+v, want state ok / detail build green", el)
	}

	// Now that a real outcome has been reported, it is final: a second done is
	// refused, and the refusal says why.
	msg, isErr = tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "err"})
	if !isErr {
		t.Fatal("a reported outcome must not be re-openable")
	}
	if !strings.Contains(msg, "already finished (ok)") {
		t.Errorf("refusal should name the terminal state: %q", msg)
	}
	if !strings.Contains(msg, "explicit job done") {
		t.Errorf("refusal should say WHY the card is terminal: %q", msg)
	}
}

// TestAutoDoneCannotOverrideAReportedOutcome: the automatic path is the one that
// guesses, so it must never overwrite what the agent explicitly reported — only
// the other direction is a correction.
func TestAutoDoneCannotOverrideAReportedOutcome(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	ctx := context.Background()
	driver := &JobDriver{t: tools}

	if _, _, _, err := driver.StartCard("Build", "compiling", dev, nil); err != nil {
		t.Fatalf("start card: %v", err)
	}
	drainItems(t, sub)
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok"}); isErr {
		t.Fatalf("explicit done: %s", msg)
	}
	drainItems(t, sub)

	err := driver.DoneCard("job-1", "cancelled", "tracking lost", "", dev)
	if err == nil {
		t.Fatal("the reaper must not be able to re-close a card the agent already reported on")
	}
	if !strings.Contains(err.Error(), "already finished (ok)") {
		t.Errorf("auto re-close refusal = %q", err)
	}
}
