package app

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
)

// terminalJobCap bounds the remembered terminal job ids (FIFO), so a
// duplicate done/update gets a clear "already finished" error instead of
// "unknown job_id" while the registry itself stays free of completed records.
const (
	terminalJobCap     = 128
	boxRestartedDetail = "box restarted"
)

// jobRecord is one tracked job card. It backs both the live element on its
// original message (via elementID/messageID) and, while non-stale, the
// agent_state runs list. A restart marks a surviving running card stale after
// cancelling its visible state; stale cards remain resolvable by update/done.
type jobRecord struct {
	jobID     string
	elementID string
	messageID string
	chatID    string
	title     string
	detail    string
	progress  *float64
	startedAt int64
	stale     bool
}

// finishedJob is what the registry remembers about a card it has closed: the
// state it closed in, and whether that closure was the BOX's own decision.
//
// auto marks a closure nobody reported: the restart sweep, the jobspool lease
// reaper, or an automatic completion hook — all of them guesses made on the
// agent's behalf. An auto closure is correctable, because the agent that did the
// work knows better than the timer that gave up on it (2026-07-24: a green build
// was force-closed by the reaper, and the agent's `job done ok` was refused, so
// the operator's card kept claiming the build had failed). rec is retained for
// exactly that: a correction re-edits the SAME card rather than minting a new
// one. An explicit `job done` is not correctable — that outcome was reported,
// not guessed, and re-opening it would let a stale duplicate overwrite it.
type finishedJob struct {
	state string
	rec   *jobRecord
	auto  bool
}

// jobRegistry is the app-owned job map. Its mutex is held across BOTH record
// mutation and corresponding frame emission (P2-4): concurrent calls cannot
// emit edits in reverse mutation order. storagePath is optional; an empty path
// preserves the memory-only constructor used by focused tests.
type jobRegistry struct {
	mu          sync.Mutex
	seq         int
	jobs        map[string]*jobRecord
	terminal    map[string]finishedJob // job id (and message id) → how it closed
	termOrder   []string               // FIFO for terminal eviction
	storagePath string
}

func newJobRegistry() *jobRegistry {
	return &jobRegistry{jobs: map[string]*jobRecord{}, terminal: map[string]finishedJob{}}
}

// restoreJobCards runs once during NewServer construction, after every durable
// delivery dependency and the optional live-activity sender are ready. Each
// previously-running card receives one cancellation edit, is durably marked
// stale, and remains addressable for a later true update/done. Already-stale
// cards restore only their identity (and retry token cleanup), never the edit.
func (s *Server) restoreJobCards() {
	reg := s.jobs
	if reg == nil || s.elIndex == nil || s.outbox == nil || s.mailbox == nil || s.store == nil {
		return
	}
	reg.mu.Lock()
	defer reg.mu.Unlock()

	ids := make([]string, 0, len(reg.jobs))
	for id := range reg.jobs {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		rec := reg.jobs[id]
		if !rec.stale {
			rec.stale = true
			rec.detail = boxRestartedDetail
			el := jobElement(rec.elementID, rec.title, "cancelled", boxRestartedDetail, rec.startedAt, rec.progress)
			// Rebuild the in-memory identity before issuing the replacement edit.
			s.elIndex.record(rec.messageID, []Element{el})
			synth := synthesizedText([]Element{el})
			s.emit(func(seq uint64) []byte {
				return editFrame(seq, rec.messageID, synth, []Element{el})
			})
			logJobRegistryFailure("stale mark", reg.persistLocked())
		} else {
			// A second startup must not emit another cancellation, but generic
			// element edits still need the original message/element identity.
			el := jobElement(rec.elementID, rec.title, "cancelled", rec.detail, rec.startedAt, rec.progress)
			s.elIndex.record(rec.messageID, []Element{el})
		}

		// Take is a durable, atomic snapshot+remove. Retrying this for an
		// already-stale row heals a prior store-write failure without duplicating
		// the card edit; normally the first sweep leaves no targets for a retry.
		targets, err := s.store.TakeLiveActivityTargets(rec.jobID)
		if err != nil {
			fmt.Fprintf(os.Stderr, "hotline: restart live activity cleanup failed job=%s: %v\n", rec.jobID, err)
			continue
		}
		activityRec := *rec
		activityRec.detail = boxRestartedDetail
		s.enqueueLiveActivities(targets, &activityRec, liveActivityEventEnd, "cancelled")
	}
}

// finishLocked moves a job to the bounded terminal set. The card's message id is
// remembered alongside the job id so a done/update addressed by message id (see
// resolveLocked) also reports "already finished" rather than "unknown". auto
// records whether the box closed the card on its own (see finishedJob). Caller
// holds mu.
func (r *jobRegistry) finishLocked(jobID, state string, auto bool) {
	rec := r.jobs[jobID]
	if rec != nil && rec.messageID != "" && rec.messageID != jobID {
		r.rememberTerminalLocked(rec.messageID, finishedJob{state: state, rec: rec, auto: auto})
	}
	delete(r.jobs, jobID)
	r.rememberTerminalLocked(jobID, finishedJob{state: state, rec: rec, auto: auto})
}

// rememberTerminalLocked records one terminal key under the FIFO cap.
func (r *jobRegistry) rememberTerminalLocked(key string, fin finishedJob) {
	if len(r.termOrder) >= terminalJobCap {
		delete(r.terminal, r.termOrder[0])
		r.termOrder = r.termOrder[1:]
	}
	r.terminal[key] = fin
	r.termOrder = append(r.termOrder, key)
}

// reopenLocked reinstates an automatically-closed card so an explicit done can
// correct its state, re-editing the original element. It returns nil when the
// key names no correctable card — unknown, or closed by a reported outcome.
// Caller holds mu.
func (r *jobRegistry) reopenLocked(key string) *jobRecord {
	fin, ok := r.terminal[key]
	if !ok || !fin.auto || fin.rec == nil {
		return nil
	}
	rec := fin.rec
	delete(r.terminal, rec.jobID)
	if rec.messageID != "" {
		delete(r.terminal, rec.messageID)
	}
	r.jobs[rec.jobID] = rec
	return rec
}

// resolveLocked finds a live card by job id, falling back to the card's message
// id ("a-NNNN"). The message id is the only handle visible on the phone and in
// the transcript, so it is the recovery key when the minted job id has been lost
// (an operator reading the card, or a caller whose notes outlived the id).
// Message ids are minted from the monotonic emit sequence, so the scan has at
// most one hit; the registry holds a handful of rows, so the cost is nil.
// Caller holds mu.
func (r *jobRegistry) resolveLocked(key string) (*jobRecord, bool) {
	if rec, ok := r.jobs[key]; ok {
		return rec, true
	}
	for _, rec := range r.jobs {
		if rec.messageID == key {
			return rec, true
		}
	}
	return nil, false
}

// rehydrate re-registers a jobspool card that survived a process restart. A
// subsequent update/done resolves against the original message, and the id is
// persisted so future starts cannot collide with it.
func (r *jobRegistry) rehydrate(rec *jobRecord) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if _, done := r.terminal[rec.jobID]; done {
		return
	}
	if _, live := r.jobs[rec.jobID]; live {
		return
	}
	rec.progress = cloneJobProgress(rec.progress)
	r.jobs[rec.jobID] = rec
	if n := jobSeqOf(rec.jobID); n > r.seq {
		r.seq = n
	}
	logJobRegistryFailure("rehydrate", r.persistLocked())
}

// jobSeqOf extracts N from a "job-N" id, or 0 if it doesn't match.
func jobSeqOf(jobID string) int {
	const prefix = "job-"
	if !strings.HasPrefix(jobID, prefix) {
		return 0
	}
	n, err := strconv.Atoi(jobID[len(prefix):])
	if err != nil {
		return 0
	}
	return n
}

// lookupErrLocked renders the not-found error for an update/done target:
// finished jobs are named as such, everything else is unknown. The message says
// WHY the card is already terminal, because the two reasons need different
// responses from the caller — a card the box closed on its own is correctable by
// a done (see reopenLocked), while a reported outcome is final and a second done
// means the caller is holding a stale id. Caller holds mu.
func (r *jobRegistry) lookupErrLocked(jobID string) string {
	if fin, ok := r.terminal[jobID]; ok {
		why := "an explicit job done reported that outcome; it is final"
		if fin.auto {
			why = "the box closed it (restart sweep or lease reaper) and its record has since been evicted, so it can no longer be corrected"
		}
		return fmt.Sprintf("job failed: job %q is already finished (%s) — %s", jobID, fin.state, why)
	}
	return fmt.Sprintf("job failed: unknown job_id %q (a card's message id, a-NNNN, also resolves)", jobID)
}

// activeRuns returns up to limit running jobs as agent_state runs, ordered by
// start time then id for a stable snapshot.
func (r *jobRegistry) activeRuns(limit int) []agentRun {
	r.mu.Lock()
	defer r.mu.Unlock()
	recs := make([]*jobRecord, 0, len(r.jobs))
	for _, j := range r.jobs {
		if j.stale {
			continue
		}
		recs = append(recs, j)
	}
	sort.Slice(recs, func(i, j int) bool {
		if recs[i].startedAt != recs[j].startedAt {
			return recs[i].startedAt < recs[j].startedAt
		}
		return recs[i].jobID < recs[j].jobID
	})
	out := make([]agentRun, 0, len(recs))
	for _, j := range recs {
		if len(out) >= limit {
			break
		}
		out = append(out, agentRun{ID: j.jobID, Title: j.title, State: "running", Detail: j.detail, StartedAt: j.startedAt})
	}
	return out
}

// jobElement builds the job Element for a job's current state, with a
// fallback line old clients and push previews can show (≤ 200 code points
// including the ellipsis, per E5).
func jobElement(id, title, state, detail string, startedAt int64, progress *float64) Element {
	fb := title + ": " + state
	if detail != "" {
		fb = title + ": " + detail
	}
	return Element{
		El:        "job",
		ID:        id,
		Fallback:  truncateFallback(fb),
		Title:     title,
		State:     state,
		Detail:    detail,
		StartedAt: startedAt,
		Progress:  progress,
	}
}

func truncateFallback(s string) string { return firstLineRunes(s, maxFallbackRunes) }

// Job implements the job tool (SPEC §2.2): start | update | done. It is the
// sugar the harness main loop calls around dispatches, and it drives both the
// live job card and the agent_state runs list.
func (t *Tools) Job(ctx context.Context, in mcpchan.JobInput) (string, bool) {
	if !t.srv.chatAllowed(in.ChatID) {
		return "job failed: device is not active", true
	}
	switch in.Action {
	case "start":
		return t.jobStart(in)
	case "update":
		return t.jobUpdate(in)
	case "done":
		return t.jobDone(in)
	default:
		return fmt.Sprintf("job failed: unknown action %q (start, update, done)", in.Action), true
	}
}

// jobFrameGuard validates a box-authored job element against the same size
// rules agent-authored elements face (title/detail are agent-supplied and
// unbounded at the schema level).
func jobFrameGuard(el Element) error {
	canon, err := json.Marshal(el)
	if err != nil {
		return fmt.Errorf("could not serialize the job element")
	}
	if len(canon) > maxElementBytes {
		return fmt.Errorf("title+detail serialize to %d bytes; the element limit is %d — shorten them", len(canon), maxElementBytes)
	}
	return nil
}

func (t *Tools) jobStart(in mcpchan.JobInput) (string, bool) {
	title := strings.TrimSpace(in.Title)
	if title == "" {
		return "job failed: start requires title", true
	}
	if in.Progress != nil && (*in.Progress < 0 || *in.Progress > 1) {
		return "job failed: progress must be within 0..1", true
	}
	detail := strings.TrimSpace(in.Detail)
	started := time.Now().Unix()
	probeEl := jobElement("el-job-probe", title, "running", detail, started, in.Progress)
	if err := jobFrameGuard(probeEl); err != nil {
		return "job failed: " + err.Error(), true
	}

	jobID, msgID, _ := t.startCard(title, detail, in.ChatID, in.Progress, started)
	return fmt.Sprintf("job started (job_id: %s, message_id: %s)", jobID, msgID), false
}

// startCard mints a running job card and registers it. It is the shared core of
// the manual `job start` tool and the automatic jobspool driver: allocate the
// job/element id, emit the element-only message, and record the live registry
// row under the registry lock (so a concurrent update can never reorder its edit
// ahead of this start). Callers validate title/detail/progress first.
func (t *Tools) startCard(title, detail, chatID string, progress *float64, started int64) (jobID, msgID, elID string) {
	reg := t.srv.jobs
	reg.mu.Lock()
	reg.seq++
	jobID = fmt.Sprintf("job-%d", reg.seq)
	elID = "el-" + jobID
	// The durable sequence prevents reuse across normal restarts. Keep this
	// defensive cleanup for a memory-only registry or a prior persistence error.
	if _, err := t.srv.store.TakeLiveActivityTargets(jobID); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: stale live activity cleanup failed job=%s: %v\n", jobID, err)
	}
	el := jobElement(elID, title, "running", detail, started, progress)
	// The message text is the E6 synthesized fallback (element-only message):
	// old clients render it, and it is the push preview for an away device.
	synth := synthesizedText([]Element{el})
	t.srv.emit(func(seq uint64) []byte {
		msgID = fmt.Sprintf("a-%d", seq)
		return msgFrame(seq, msgID, synth, nil, "", nil, []Element{el})
	})
	reg.jobs[jobID] = &jobRecord{
		jobID: jobID, elementID: elID, messageID: msgID, chatID: chatID,
		title: title, detail: detail, progress: cloneJobProgress(progress), startedAt: started,
	}
	// Persistence is best-effort like the outbox: the already-emitted start is
	// never repeated merely because this snapshot write failed.
	logJobRegistryFailure("start", reg.persistLocked())
	reg.mu.Unlock()

	t.srv.elIndex.record(msgID, []Element{el})
	// Logged HERE rather than in jobStart so the automatic path logs it too: the
	// jobspool dispatcher calls startCard directly but terminalizes through
	// jobDone, which meant an auto-card wrote a "job done" line with no matching
	// "job start". Anything pairing those lines (the fleet sweep's stale-card
	// check) silently saw no automatic cards at all.
	if t.log != nil {
		t.log.Append(transcript.Record{Dir: "out", ChatID: chatID, Kind: "job", MessageID: msgID, Text: "job start: " + title})
	}
	t.srv.agentStateChanged()
	return jobID, msgID, elID
}

func (t *Tools) jobUpdate(in mcpchan.JobInput) (string, bool) {
	if in.Progress != nil && (*in.Progress < 0 || *in.Progress > 1) {
		return "job failed: progress must be within 0..1", true
	}
	reg := t.srv.jobs
	reg.mu.Lock()
	rec, ok := reg.resolveLocked(in.JobID)
	if !ok {
		msg := reg.lookupErrLocked(in.JobID)
		reg.mu.Unlock()
		return msg, true
	}
	// in.JobID may be the card's message id; every downstream keyed operation
	// (live activities, terminal bookkeeping, the reply) uses the canonical id.
	jobID := rec.jobID
	progress := rec.progress
	if in.Progress != nil {
		progress = cloneJobProgress(in.Progress)
	}
	detail := rec.detail
	if d := strings.TrimSpace(in.Detail); d != "" {
		detail = d
	}
	el := jobElement(rec.elementID, rec.title, "running", detail, rec.startedAt, progress)
	if err := jobFrameGuard(el); err != nil {
		reg.mu.Unlock()
		return "job failed: " + err.Error(), true
	}
	rec.progress = progress
	rec.detail = detail
	rec.stale = false // a successful update explicitly reactivates a swept card
	// Element-only edit (E6): the synthesized fallback rides as text so old
	// clients stay current; per the push rule (E10) this edit never buzzes.
	synth := synthesizedText([]Element{el})
	msgID := rec.messageID
	t.srv.emit(func(seq uint64) []byte { return editFrame(seq, msgID, synth, []Element{el}) })
	logJobRegistryFailure("update", reg.persistLocked())
	// Snapshot and enqueue after the durable edit while registry serialization is
	// still held, so registration/update/done cannot reverse lifecycle order.
	t.srv.enqueueLiveActivities(t.srv.store.ActiveLiveActivityTargets(jobID), rec, liveActivityEventUpdate, "running")
	reg.mu.Unlock()

	t.srv.agentStateChanged()
	return fmt.Sprintf("job updated (job_id: %s)", jobID), false
}

// jobDone is the agent's own `job done`: an explicitly REPORTED outcome, and the
// ground truth that outranks anything the box guessed.
func (t *Tools) jobDone(in mcpchan.JobInput) (string, bool) { return t.finishJob(in, false) }

// finishJob terminalizes a card. auto marks a closure the box decided on its own
// (the jobspool driver: a completion hook, the lease reaper, the restart sweep)
// rather than one the agent reported — see finishedJob.
func (t *Tools) finishJob(in mcpchan.JobInput, auto bool) (string, bool) {
	switch in.State {
	case "ok", "err", "cancelled":
	default:
		return fmt.Sprintf("job failed: done requires state ok|err|cancelled, got %q", in.State), true
	}
	notify := strings.TrimSpace(in.Notify)
	reg := t.srv.jobs
	reg.mu.Lock()
	rec, ok := reg.resolveLocked(in.JobID)
	if !ok && !auto {
		// CORRECTION. The card is terminal, but the box closed it on a guess and
		// this caller did the work. Reinstate the record so the edit below lands on
		// the original card, replacing the guessed state with the reported one. Only
		// an explicit done may do this; the automatic path must never re-open a card
		// its own reaper just closed.
		if fixed := reg.reopenLocked(in.JobID); fixed != nil {
			rec, ok = fixed, true
		}
	}
	if !ok {
		msg := reg.lookupErrLocked(in.JobID)
		reg.mu.Unlock()
		return msg, true
	}
	// in.JobID may be the card's message id; every downstream keyed operation
	// (live activities, terminal bookkeeping, the reply) uses the canonical id.
	jobID := rec.jobID
	detail := rec.detail
	if d := strings.TrimSpace(in.Detail); d != "" {
		detail = d
	}
	el := jobElement(rec.elementID, rec.title, in.State, detail, rec.startedAt, rec.progress)
	if err := jobFrameGuard(el); err != nil {
		reg.mu.Unlock()
		return "job failed: " + err.Error(), true
	}
	rec.detail = detail
	title := rec.title
	msgID := rec.messageID
	// Final edit first so the card lands in its terminal state. It stays the
	// wire-identical, generic-push-ineligible element-only edit from E10. FB44
	// attaches an internal completion intent to this SAME durable fanout only for
	// a bare successful finish; the mailbox decides per device at durable insert
	// time whether that device was away.
	synth := synthesizedText([]Element{el})
	if in.State == "ok" && notify == "" {
		body := rec.detail
		if body == "" {
			body = "Completed"
		}
		t.srv.emitWithPushIntent(pushIntent{Title: rec.title, Body: body}, func(seq uint64) []byte {
			return editFrame(seq, msgID, synth, []Element{el})
		})
	} else {
		t.srv.emit(func(seq uint64) []byte { return editFrame(seq, msgID, synth, []Element{el}) })
	}
	// Clear registrations durably and enqueue the final end after the terminal
	// edit, still under registry serialization. APNs I/O runs later on sender
	// lanes; only immutable requests are captured here.
	if targets, err := t.srv.store.TakeLiveActivityTargets(jobID); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: terminal live activity cleanup failed job=%s: %v\n", jobID, err)
	} else {
		t.srv.enqueueLiveActivities(targets, rec, liveActivityEventEnd, in.State)
	}
	// A nonblank notify follows the terminal edit as a fresh message. It is the
	// sole completion push: the branch above deliberately attaches no custom
	// intent when notify is present, avoiding a double buzz.
	var notifyID string
	if notify != "" {
		t.srv.emit(func(seq uint64) []byte {
			notifyID = fmt.Sprintf("a-%d", seq)
			return msgFrame(seq, notifyID, notify, nil, "", nil, nil)
		})
	}
	reg.finishLocked(jobID, in.State, auto)
	logJobRegistryFailure("terminal removal", reg.persistLocked())
	reg.mu.Unlock()

	t.srv.agentStateChanged()
	if t.log != nil {
		t.log.Append(transcript.Record{Dir: "out", ChatID: in.ChatID, Kind: "job", MessageID: msgID, Text: "job done: " + title + " (" + in.State + ")"})
	}
	if notifyID != "" {
		return fmt.Sprintf("job done (job_id: %s, notify_id: %s)", jobID, notifyID), false
	}
	return fmt.Sprintf("job done (job_id: %s)", jobID), false
}
