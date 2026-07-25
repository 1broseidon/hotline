package jobspool

import (
	"context"
	"fmt"
	"os"
	"sort"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

// JobSink is the seam onto the app channel's job registry — the four operations
// the dispatcher needs to drive a live card. app.JobDriver satisfies it
// structurally (no import either way), the same trick notify.Sink uses.
//
//   - StartCard creates a running card and returns its ids.
//   - UpdateCard is a silent (non-buzzing) edit of a running card.
//   - DoneCard moves a card to its terminal state, with an optional buzz line.
//   - RehydrateCard re-registers a card persisted across a restart so a later
//     Update/Done resolves against the surviving message instead of erroring.
type JobSink interface {
	StartCard(title, detail, chatID string, progress *float64) (jobID, msgID, elementID string, err error)
	UpdateCard(jobID, detail, chatID string, progress *float64) error
	DoneCard(jobID, state, detail, notify, chatID string) error
	RehydrateCard(jobID, elementID, msgID, chatID, title, detail string, startedAt int64, progress *float64)
}

// tickInterval matches notify's control-file latency: job intents deserve the
// same 2s responsiveness as machine events.
const tickInterval = 2 * time.Second

// NudgeSink is the inbound-injection seam the FB84 check-in nudge rides: the one
// method it needs from provider.InboundSink, declared locally so jobspool never
// imports internal/provider (the same trick notify.Sink uses). The harness
// inbound sink — piSink → the channel notifier on injected hosts, or Claude
// Code's own notifier — satisfies it structurally, so a nudge lands as the same
// synthetic <channel> turn a notify or schedule fire does.
type NudgeSink interface {
	SendChannel(ctx context.Context, content string, meta map[string]string) error
}

// Dispatcher drains the intent spool and drives the batch rollup cards. now/tick/
// lease are fields (defaulted in NewDispatcher) so tests inject a fixed clock and
// never sleep.
type Dispatcher struct {
	spoolPath  string
	activePath string

	active     *ActiveDoc
	rehydrated map[string]bool   // batch → live in the current process's registry
	rendered   map[string]string // batch → last rendered running signature (update dedup)

	now   func() time.Time
	tick  time.Duration
	lease time.Duration

	// FB84 check-in nudge: while a carded batch stays running, re-inject a
	// hygiene reminder through the harness inbound path every nudgeEvery, so
	// closing the card never depends on the agent remembering. Disabled when
	// nudgeSink is nil or nudgeEvery <= 0. nudgeSrcs/nudgeLog are the static
	// half (provider sources for the reply-routing key + transcript logger),
	// bound at wiring; nudgeSink is bound by the poller that owns the inbound
	// sink. lastNudged is the per-batch timer, in-memory only (re-armed on boot
	// from each surviving card's StartedAt).
	nudgeSink  NudgeSink
	nudgeEvery time.Duration
	nudgeSrcs  []string
	nudgeLog   *transcript.Logger
	lastNudged map[string]time.Time
}

// NewDispatcher builds a Dispatcher over spool.json/activejobs.json at the jobs
// paths.
func NewDispatcher(spoolPath, activePath string) *Dispatcher {
	return &Dispatcher{
		spoolPath:  spoolPath,
		activePath: activePath,
		rehydrated: map[string]bool{},
		rendered:   map[string]string{},
		lastNudged: map[string]time.Time{},
		now:        time.Now,
		tick:       tickInterval,
		lease:      DefaultLease,
		nudgeEvery: DefaultNudgeInterval,
	}
}

// ConfigureNudge sets the static half of the FB84 check-in nudge: the sources it
// may route a check-in to (the reply-routing source key, mirroring the
// scheduler), the transcript logger (nil-safe), and the interval. A non-positive
// interval — or an EMPTY source list — disables the nudge. Called once at
// wiring, before Run; the inbound sink is bound separately by SetNudgeSink,
// since the poller is where the box's inbound sink actually lives.
//
// sources must be CARD-CAPABLE channels only (provider.Router.CardSources), not
// every configured provider. A nudge is stamped with a source and the agent
// replies on it, so routing one to a channel that cannot render job cards buzzes
// the operator about a card he has no way to see: on a telegram+app box the
// first configured provider was telegram, and every 15 minutes for eight hours a
// nudge for an app-channel card arrived as a telegram message (2026-07-24,
// job-142). Give the dispatcher only channels that can show the thing it is
// pointing at, and none at all if there are none.
func (d *Dispatcher) ConfigureNudge(sources []string, log *transcript.Logger, every time.Duration) {
	d.nudgeSrcs = sources
	d.nudgeLog = log
	d.nudgeEvery = every
}

// SetNudgeSink binds the harness inbound sink the check-in nudge injects
// through. A nil sink leaves the nudge disabled.
func (d *Dispatcher) SetNudgeSink(sink NudgeSink) { d.nudgeSink = sink }

// Run drives cards until ctx is cancelled: it loads durable state and sweeps any
// card orphaned by the previous process to a terminal state, then reconciles once
// eagerly (restart catch-up) and once per tick. It returns nil on cancellation
// and never otherwise exits — store/injection failures are logged and retried.
func (d *Dispatcher) Run(ctx context.Context, sink JobSink) error {
	d.load()
	d.sweepOrphans()

	d.dispatch(ctx, sink)
	t := time.NewTicker(d.tick)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-t.C:
			d.dispatch(ctx, sink)
		}
	}
}

// load reads activejobs.json into memory. A read failure logs and starts empty
// (a lost activejobs.json costs card continuity, never correctness).
func (d *Dispatcher) load() {
	a, err := LoadActive(d.activePath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: job activejobs read failed: %v\n", err)
		a = &ActiveDoc{}
		a.normalize()
	}
	d.active = a
}

// sweepOrphans marks every job still running in the loaded state as cancelled
// with the app registry's restart detail. The shared semantics prevent the
// jobspool reconcile from overwriting the app startup cancellation with an
// err/lost-on-restart terminal edit.
func (d *Dispatcher) sweepOrphans() {
	now := d.now().Unix()
	for _, b := range d.active.Batches {
		for _, j := range b.Jobs {
			if !j.terminal() {
				j.State = stateCancelled
				j.Detail = boxRestartedDetail
				j.UpdatedAt = now
			}
		}
	}
}

// dispatch runs one cycle: drain the spool into durable state, apply the reaper,
// then reconcile every active card. State changes are persisted (best effort)
// before any card edit so a mid-cycle crash cannot double-drive a card.
//
// Every active batch is reconciled each tick — not just the ones an intent
// touched — so a card whose StartCard/DoneCard failed (no device linked yet)
// retries until it lands, and a batch orphaned by a restart terminalizes on the
// first tick. A running batch whose rollup is unchanged emits nothing (the render
// signature dedups), so idle batches cost only a JSON read.
func (d *Dispatcher) dispatch(ctx context.Context, sink JobSink) {
	if d.active == nil {
		d.load()
	}
	intents := d.drain()
	changed := len(intents) > 0
	for _, in := range intents {
		d.apply(in)
	}
	if reaped := d.reap(); len(reaped) > 0 {
		changed = true
	}
	if changed {
		d.save()
	}
	if len(d.active.Batches) == 0 {
		return
	}

	// Reconcile in a stable order so a burst of new batches cards deterministically.
	keys := make([]string, 0, len(d.active.Batches))
	for k := range d.active.Batches {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		d.reconcile(ctx, sink, k)
	}
	d.save() // persist the ids StartCard just minted / the prune of finished batches
	d.nudge(ctx)
}

// nudge re-injects a check-in reminder for every carded batch that has sat
// running past the nudge interval — the FB84 backstop so a card whose done hook
// never fired gets a periodic "still working?" prod through the same inbound
// path notify and schedule fires use, instead of depending on the agent
// remembering to close it.
//
// Arming is a pure function of durable state: a batch is armed exactly while it
// has a live card (JobID set) and is not yet allTerminal. So it disarms the
// instant the batch reaches a terminal state — a done intent, the lease reaper,
// or the boot orphan sweep (all of which reconcile then prunes) — and re-arms on
// boot for any card still running in activejobs.json. The per-batch timer is
// in-memory and resets on restart; the next nudge is measured from the card's
// StartedAt, so a card open across a bounce is nudged promptly with its true age.
func (d *Dispatcher) nudge(ctx context.Context) {
	if d.nudgeSink == nil || d.nudgeEvery <= 0 || d.active == nil {
		return
	}
	// No card-capable channel attached → never nudge. A check-in is a prompt to go
	// look at a card, so it is only meaningful on a channel that can render one.
	// See ConfigureNudge.
	if len(d.nudgeSrcs) == 0 {
		return
	}
	now := d.now()
	for key, b := range d.active.Batches {
		if b.JobID == "" || b.allTerminal() {
			delete(d.lastNudged, key) // not armed: uncarded, or terminal awaiting prune
			continue
		}
		started := time.Unix(b.StartedAt, 0)
		ref := started
		if last, ok := d.lastNudged[key]; ok && last.After(ref) {
			ref = last
		}
		if now.Sub(ref) < d.nudgeEvery {
			continue
		}
		content := nudgeContent(b, now.Sub(started))
		if err := d.nudgeSink.SendChannel(ctx, content, d.nudgeMeta(b, now)); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: job %q check-in nudge not delivered: %v\n", key, err)
			continue
		}
		d.lastNudged[key] = now
		d.nudgeLog.Append(transcript.Record{Dir: "in", ChatID: b.ChatID, Kind: "job_nudge", Text: content}) // nil-safe
	}
}

// nudgeMeta stamps the routing/framing meta for an injected check-in, matching
// the scheduler's shape: source is the reply-routing key (first configured
// provider), chat_id is the card's own chat, kind marks the synthetic turn.
func (d *Dispatcher) nudgeMeta(b *Batch, now time.Time) map[string]string {
	src := ""
	if len(d.nudgeSrcs) > 0 {
		src = d.nudgeSrcs[0]
	}
	return map[string]string{
		"source":  src,
		"chat_id": b.ChatID,
		"kind":    "job_nudge",
		"job_id":  b.JobID,
		"ts":      now.UTC().Format("2006-01-02T15:04:05.000Z07:00"),
	}
}

// nudgeContent frames the check-in as an automatic hygiene prod — not a user
// message — mirroring the scheduler's self-authored framing so the agent treats
// it as a box timer and acts on the card (or ignores it) rather than replying to
// a phantom user.
func nudgeContent(b *Batch, openFor time.Duration) string {
	mins := int(openFor.Minutes())
	if mins < 0 {
		mins = 0
	}
	title := firstNonEmpty(b.Title, "Subagent work")
	return fmt.Sprintf("🔔 Job card %s %q has been open %dm. This is an automatic check-in — not a new message from the user. If its work is done, close it with the job tool (done); if it's still going, post an update or leave it. Doing nothing is fine if nothing changed.",
		b.JobID, title, mins)
}

// drain removes and returns every pending intent under the spool lock.
func (d *Dispatcher) drain() []Intent {
	var out []Intent
	err := MutateSpool(d.spoolPath, func(sp *SpoolDoc) error {
		if len(sp.Pending) == 0 {
			return nil
		}
		out = append(out, sp.Pending...)
		sp.Pending = sp.Pending[:0]
		return nil
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: job spool drain failed: %v\n", err)
		return nil
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Seq < out[j].Seq })
	return out
}

// apply folds one intent into durable batch/job state and returns the batch key
// it touched (empty if the intent was unusable).
func (d *Dispatcher) apply(in Intent) string {
	cookie := in.Cookie
	if cookie == "" {
		// Agent-addressed (a completion event that knows only the harness's agent
		// id). Resolve through the binding an earlier update established; an
		// unbound agent id is dropped rather than self-batched, so a completion for
		// work this box never carded cannot mint a phantom card.
		ref, ok := d.active.Agents[in.Agent]
		if !ok || d.active.Batches[ref.Batch] == nil {
			return ""
		}
		in.Batch, cookie = ref.Batch, ref.Cookie
	}
	batchKey := d.resolveBatchKey(in)
	now := d.now().Unix()
	chatID := in.ChatID
	if chatID == "" {
		chatID = "app"
	}

	b := d.active.Batches[batchKey]
	if b != nil && in.Action == "start" && b.allTerminal() {
		// SEAL. Every job of the previous fan-out under this key has finished, so
		// this start belongs to a NEW one. Park the finished batch under a unique
		// key (reconcile still owes it a terminal card edit, which it will make
		// there) and fall through to open a fresh batch — and therefore a fresh
		// card, carrying THIS fan-out's title — at the caller's key.
		d.seal(batchKey)
		b = nil
	}
	if b == nil {
		// A start or a fire-and-forget done opens a batch; a stray update for an
		// unknown batch is dropped (its start was lost, or it already finished and
		// was pruned).
		if in.Action == "update" {
			return ""
		}
		b = &Batch{
			Batch:     batchKey,
			ChatID:    chatID,
			Title:     firstNonEmpty(in.Title, "Subagent work"),
			StartedAt: now,
			Jobs:      map[string]*Job{},
		}
		d.active.Batches[batchKey] = b
	}

	j := b.Jobs[cookie]
	switch in.Action {
	case "start":
		if j == nil {
			j = &Job{Cookie: cookie, StartedAt: now, State: stateRunning}
			b.Jobs[cookie] = j
			b.Order = append(b.Order, cookie)
		}
		if in.Title != "" {
			j.Title = in.Title
		}
		if in.Detail != "" {
			j.Detail = in.Detail
		}
		j.State = stateRunning
		j.UpdatedAt = now
	case "update":
		if j == nil {
			return "" // update before start — ignore
		}
		if in.Detail != "" {
			j.Detail = in.Detail
		}
		j.UpdatedAt = now
	case "done":
		if j == nil {
			// done with no prior start: synthesize a completed job so a fire-and-forget
			// hook (start+done in one shot) still cards.
			j = &Job{Cookie: cookie, StartedAt: now, Title: in.Title}
			b.Jobs[cookie] = j
			b.Order = append(b.Order, cookie)
		}
		if in.Detail != "" {
			j.Detail = in.Detail
		}
		j.State = terminalState(in.State)
		j.UpdatedAt = now
		if in.Notify != "" {
			b.Notify = in.Notify
		}
	default:
		return ""
	}
	if in.Agent != "" && b.Jobs[cookie] != nil {
		// Bind the harness's completion-side id to this job so a later done can
		// address it by --agent alone. The dispatch hook learns the agent id only
		// AFTER launch, which is why this rides an ordinary update.
		b.Jobs[cookie].AgentID = in.Agent
		if d.active.Agents == nil {
			d.active.Agents = map[string]AgentRef{}
		}
		d.active.Agents[in.Agent] = AgentRef{Batch: batchKey, Cookie: cookie}
	}
	b.Recent = cookie
	return batchKey
}

// seal parks a finished batch under a unique "#sealed-N" key, moving its
// bookkeeping with it, and frees the caller's key for the next fan-out. The
// parked batch is still reconciled (and pruned) from its new key, so the card it
// owns is closed exactly once.
func (d *Dispatcher) seal(key string) {
	b := d.active.Batches[key]
	if b == nil {
		return
	}
	d.active.Seal++
	parked := fmt.Sprintf("%s#sealed-%d", key, d.active.Seal)
	b.Batch = parked
	delete(d.active.Batches, key)
	d.active.Batches[parked] = b
	for id, ref := range d.active.Agents {
		if ref.Batch == key {
			d.active.Agents[id] = AgentRef{Batch: parked, Cookie: ref.Cookie}
		}
	}
	moveKey(d.rehydrated, key, parked)
	moveKey(d.rendered, key, parked)
	moveKey(d.lastNudged, key, parked)
}

// moveKey re-keys one in-memory bookkeeping entry, if present.
func moveKey[V any](m map[string]V, from, to string) {
	if v, ok := m[from]; ok {
		m[to] = v
		delete(m, from)
	}
}

// resolveBatchKey maps an intent to the batch key it belongs to.
//
// An explicit --batch always wins. Without one, the cookie is the designed
// correlator: an update or done arriving batch-less must land on the SAME rollup
// its start opened, so we look the cookie up across the active batches. Only a
// cookie no active batch has ever seen falls back to a self-batch (a batch of one
// keyed on the cookie) — the fire-and-forget hook that never passed a batch.
//
// This is the FB13 phantom-card fix: before it, `job done --cookie X` with no
// --batch self-batched X, spawned a throwaway "Subagent work" card, closed THAT,
// and left the real rollup slot running forever. Ambiguity (the same cookie live
// in two batches — not expected, but survivable) resolves to the batch whose copy
// of the cookie was touched most recently, and logs.
func (d *Dispatcher) resolveBatchKey(in Intent) string {
	if in.Batch != "" {
		return in.Batch
	}
	cookie := in.Cookie
	if d.active == nil {
		return cookie
	}
	var hits []*Batch
	for _, b := range d.active.Batches {
		if _, ok := b.Jobs[cookie]; ok {
			hits = append(hits, b)
		}
	}
	switch len(hits) {
	case 0:
		return cookie // genuinely unknown cookie → self-batch of one
	case 1:
		return hits[0].Batch
	default:
		// Pick the batch whose copy of the cookie is freshest; tie-break on batch
		// key so the choice is deterministic.
		best := hits[0]
		for _, b := range hits[1:] {
			if bj, cj := b.Jobs[cookie], best.Jobs[cookie]; bj.UpdatedAt > cj.UpdatedAt ||
				(bj.UpdatedAt == cj.UpdatedAt && b.Batch < best.Batch) {
				best = b
			}
		}
		fmt.Fprintf(os.Stderr, "hotline: job cookie %q found in %d active batches; resolving to %q\n", cookie, len(hits), best.Batch)
		return best.Batch
	}
}

// reap closes any job that has sat running past the lease as CANCELLED
// "tracking lost". Returns the batch keys it touched. This is the backstop for a
// completion signal that never arrived (user interrupt, killed harness).
//
// The state is deliberately cancelled, not err. The reaper has no idea whether
// the work succeeded — only that the lease expired — and the app renders err as
// a "failed" pill. Reporting a lost job as failed made a green build read as a
// failure on the operator's phone. A rollup therefore reads err only when a
// member genuinely REPORTED an error (see Batch.finalState).
func (d *Dispatcher) reap() []string {
	if d.lease <= 0 {
		return nil
	}
	now := d.now()
	cutoff := now.Add(-d.lease).Unix()
	var touched []string
	for key, b := range d.active.Batches {
		hit := false
		for _, j := range b.Jobs {
			if !j.terminal() && j.UpdatedAt > 0 && j.UpdatedAt < cutoff {
				j.State = stateCancelled
				j.Detail = trackingLostDetail
				j.UpdatedAt = now.Unix()
				hit = true
			}
		}
		if hit {
			touched = append(touched, key)
		}
	}
	return touched
}

// reconcile drives the batch's card to match its current rollup state: create it
// if new, rehydrate it if it survived a restart, then edit or terminalize it.
func (d *Dispatcher) reconcile(ctx context.Context, sink JobSink, key string) {
	b := d.active.Batches[key]
	if b == nil {
		return
	}
	_ = ctx
	detail := rollupDetail(b)
	progress := rollupProgress(b)

	if b.JobID == "" {
		jobID, msgID, elID, err := sink.StartCard(b.Title, detail, b.ChatID, progress)
		if err != nil {
			// No card yet (e.g. no device linked). Retry next reconcile; if the batch
			// already finished with nothing to show, drop it.
			if b.allTerminal() {
				d.prune(key)
			}
			return
		}
		b.JobID, b.MessageID, b.ElementID = jobID, msgID, elID
		d.rehydrated[key] = true
		d.rendered[key] = renderSig(detail, progress)
		if !b.allTerminal() {
			return // StartCard already rendered the current running state
		}
	} else if !d.rehydrated[key] {
		// Card persisted across a restart: re-register it so the edit below lands on
		// the surviving message rather than erroring as an unknown job.
		sink.RehydrateCard(b.JobID, b.ElementID, b.MessageID, b.ChatID, b.Title, detail, b.StartedAt, progress)
		d.rehydrated[key] = true
	}

	if b.allTerminal() {
		if err := sink.DoneCard(b.JobID, b.finalState(), detail, b.Notify, b.ChatID); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: job %q not closed: %v\n", key, err)
			return // keep the batch; retry next tick
		}
		d.prune(key)
		return
	}
	// Running: emit only when the rollup actually changed, so an idle batch is free.
	sig := renderSig(detail, progress)
	if d.rendered[key] == sig {
		return
	}
	if err := sink.UpdateCard(b.JobID, detail, b.ChatID, progress); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: job %q not updated: %v\n", key, err)
		return
	}
	d.rendered[key] = sig
}

// prune drops a finished batch from durable state and its bookkeeping maps,
// including any agent-id bindings that pointed at it (so a late completion for a
// closed card resolves to nothing instead of reopening one).
func (d *Dispatcher) prune(key string) {
	delete(d.active.Batches, key)
	delete(d.rehydrated, key)
	delete(d.rendered, key)
	delete(d.lastNudged, key)
	for id, ref := range d.active.Agents {
		if ref.Batch == key {
			delete(d.active.Agents, id)
		}
	}
}

// renderSig is the change-detection key for a running card's rollup content.
func renderSig(detail string, progress *float64) string {
	if progress == nil {
		return detail
	}
	return fmt.Sprintf("%s\x00%.4f", detail, *progress)
}

func (d *Dispatcher) save() {
	if err := SaveActive(d.active, d.activePath); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: job activejobs write failed: %v\n", err)
	}
}

// rollupProgress is completed/total across the batch's jobs (0 when empty).
func rollupProgress(b *Batch) *float64 {
	total, term := b.counts()
	if total == 0 {
		return nil
	}
	p := float64(term) / float64(total)
	return &p
}

// rollupDetail describes the batch's current state. One job renders as that job's
// own detail; many render as "k/N done · <most-recent worker>".
func rollupDetail(b *Batch) string {
	// Preserve the exact startup cancellation detail even for a multi-worker
	// rollup; otherwise the follow-up terminal edit would replace it with a
	// "N/N done" prefix immediately after restart.
	allCancelled := len(b.Jobs) > 0
	sawRestart := false
	for _, j := range b.Jobs {
		if j.State != stateCancelled {
			allCancelled = false
			break
		}
		if j.Detail == boxRestartedDetail {
			sawRestart = true
		}
	}
	if allCancelled && sawRestart {
		return boxRestartedDetail
	}

	total, term := b.counts()
	recent := b.Jobs[b.Recent]
	if total <= 1 {
		if recent != nil {
			return firstNonEmpty(recent.Detail, recent.Title)
		}
		return ""
	}
	label := ""
	if recent != nil {
		label = firstNonEmpty(recent.Detail, recent.Title, recent.Cookie)
	}
	if label == "" {
		return fmt.Sprintf("%d/%d done", term, total)
	}
	return fmt.Sprintf("%d/%d done · %s", term, total, label)
}

// terminalState maps a done intent's --state to a stored terminal value,
// defaulting an unknown/empty value to ok.
func terminalState(s string) string {
	switch s {
	case stateOK, stateErr, stateCancelled:
		return s
	default:
		return stateOK
	}
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}
