// Package jobspool is hotline's automatic job-card ingress: a harness hook (or
// any local script) calls `hotline job start|update|done --cookie <id>`, the CLI
// durably enqueues the intent into spool.json, and an in-process JobDispatcher on
// the daemon side drains it and drives the SAME app-channel job card the `job`
// MCP tool drives (start card / silent updates / terminal state).
//
// It is the notify pattern (flock-guarded spool + 2s-tick dispatcher) applied to
// a new noun, with two differences notify does not have:
//
//   - Correlation. The CLI cannot get a synchronous job_id back, so every intent
//     carries a caller-supplied cookie (the harness's own tool_use_id / agent id).
//     The dispatcher maps cookie → the app job card it created.
//   - Rollup. Every intent also carries a batch key (the dispatching session/run
//     id). All cookies sharing a batch aggregate into ONE card: progress =
//     terminal/total, terminal when every job in the batch is terminal.
//
// Durable card state lives in activejobs.json (StateDir/jobs/). It is what fixes
// the frozen-card-on-restart bug: on boot the dispatcher sweeps every card that
// was still "running" under the previous (now-dead) process to a terminal state
// (cancelled, "box restarted") instead of leaving it spinning forever.
package jobspool

import (
	"path/filepath"
	"time"
)

// maxPending caps the intent spool (mirrors notify.maxPending) so a crashlooping
// hook backpressures at the CLI rather than growing the file without bound.
const maxPending = 200

// maxField bounds a single stored text field (title/detail/notify). Longer input
// is truncated at a rune boundary, never rejected.
const maxField = 2048

// DefaultLease is how long a job may sit "running" with no update before the
// reaper closes its card as CANCELLED "tracking lost" — the backstop that
// unfreezes a card whose completion signal never arrived (user interrupt, killed
// harness, a subagent whose SubagentStop hook never ran).
//
// Cancelled, never err: the reaper does not know the work failed, only that it
// stopped hearing about it. Terminalizing a lost job as err made the app render
// a "failed" pill over work that had in fact succeeded (2026-07-24: job-143, a
// green build, carded as failed). "Lost" and "failed" are different facts and
// the card must not conflate them.
const DefaultLease = 30 * time.Minute

// DefaultNudgeInterval is how often the FB84 check-in nudge re-injects a
// "still working?" reminder for a card that has stayed running, so hygiene never
// depends on the agent remembering. It is below DefaultLease so a live card is
// prodded at least once before the reaper would force it closed. Overridable via
// HOTLINE_JOB_NUDGE_INTERVAL (0/off disables).
const DefaultNudgeInterval = 15 * time.Minute

// Intent is one queued CLI write awaiting the dispatcher. Action is
// start|update|done; Cookie is the caller's correlation id; Batch groups cookies
// into one rollup card. It is the on-disk job-file schema.
//
// Agent is the SECOND correlator, and the one that closes background work. A
// harness that dispatches asynchronously knows the dispatch's cookie at launch
// but only ever learns of completion under a different id (Claude Code: the
// PreToolUse cookie is tool_use_id, while the completion event — SubagentStop —
// carries agent_id and no tool_use_id at all). So an update intent may carry
// --agent to BIND agent id → cookie, and a later done intent may address the job
// by --agent alone. An unbound agent id is dropped, never self-batched: a
// completion for work this box never carded must not mint a phantom card.
type Intent struct {
	Seq      int      `json:"seq"`
	Action   string   `json:"action"`
	Cookie   string   `json:"cookie"`
	Agent    string   `json:"agent,omitempty"`
	Batch    string   `json:"batch,omitempty"`
	Title    string   `json:"title,omitempty"`
	Detail   string   `json:"detail,omitempty"`
	Progress *float64 `json:"progress,omitempty"`
	State    string   `json:"state,omitempty"`  // done: ok|err|cancelled
	Notify   string   `json:"notify,omitempty"` // done: optional buzz line
	ChatID   string   `json:"chatId,omitempty"`
	At       string   `json:"at"`
}

// SpoolDoc is the persisted spool.json: pending intents plus a monotonic seq so
// ids stay unique across appends within one process's view of the file.
type SpoolDoc struct {
	Seq     int      `json:"seq"`
	Pending []Intent `json:"pending"`
}

func (d *SpoolDoc) normalize() {
	if d.Pending == nil {
		d.Pending = []Intent{}
	}
}

// Job is one tracked worker inside a batch. State is running until a done intent
// (or the reaper / restart sweep) moves it to a terminal value.
type Job struct {
	Cookie    string `json:"cookie"`
	AgentID   string `json:"agentId,omitempty"` // harness completion-side id, if bound
	Title     string `json:"title,omitempty"`
	Detail    string `json:"detail,omitempty"`
	State     string `json:"state"` // running|ok|err|cancelled
	StartedAt int64  `json:"startedAt"`
	UpdatedAt int64  `json:"updatedAt"`
}

func (j *Job) terminal() bool { return j.State != "" && j.State != stateRunning }

// Batch is one rollup card: the app job it created (JobID/ElementID/MessageID,
// empty until the card is successfully started) and the per-cookie jobs it
// aggregates. Kept in activejobs.json so a restart can rehydrate and terminalize.
//
// Batch holds the batch's CURRENT map key, which is the caller's key while the
// fan-out is live and the parked "#sealed-N" key once it has finished (see
// ActiveDoc.Seal). Callers key on something coarse — Claude Code's hook passes
// its session id — so without sealing every dispatch for the session's whole
// life joined one batch that never went allTerminal: one immortal card wearing
// the first dispatch's title, nudging forever. Sealing bounds a batch to one
// fan-out.
type Batch struct {
	Batch     string          `json:"batch"`
	JobID     string          `json:"jobId,omitempty"`
	ElementID string          `json:"elementId,omitempty"`
	MessageID string          `json:"messageId,omitempty"`
	ChatID    string          `json:"chatId"`
	Title     string          `json:"title"`
	Notify    string          `json:"notify,omitempty"`
	StartedAt int64           `json:"startedAt"`
	Order     []string        `json:"order"` // cookie insertion order
	Jobs      map[string]*Job `json:"jobs"`
	Recent    string          `json:"recent,omitempty"` // most-recently-touched cookie
}

// AgentRef binds a harness completion-side agent id to the job it closes.
type AgentRef struct {
	Batch  string `json:"batch"`
	Cookie string `json:"cookie"`
}

// ActiveDoc is the persisted activejobs.json.
//
// Agents is the agent-id → job binding an async harness needs (see Intent.Agent).
// Seal is the monotonic counter behind batch sealing: when a fan-out finishes,
// its batch is parked under "<key>#sealed-<n>" so the card still terminalizes
// while the caller's key is free for the NEXT fan-out to open a fresh card.
type ActiveDoc struct {
	Batches map[string]*Batch   `json:"batches"`
	Agents  map[string]AgentRef `json:"agents,omitempty"`
	Seal    int                 `json:"seal,omitempty"`
}

func (d *ActiveDoc) normalize() {
	if d.Batches == nil {
		d.Batches = map[string]*Batch{}
	}
	if d.Agents == nil {
		d.Agents = map[string]AgentRef{}
	}
}

const (
	stateRunning   = "running"
	stateOK        = "ok"
	stateErr       = "err"
	stateCancelled = "cancelled"
	// The two details the box writes when IT, not the worker, ended a job. Both
	// terminalize as cancelled: the box lost the thread, which is not a failure.
	boxRestartedDetail = "box restarted"
	trackingLostDetail = "tracking lost"
)

// Path helpers: everything jobspool owns lives under <box root>/jobs/.
func Dir(stateRoot string) string        { return filepath.Join(stateRoot, "jobs") }
func SpoolPath(stateRoot string) string  { return filepath.Join(Dir(stateRoot), "spool.json") }
func ActivePath(stateRoot string) string { return filepath.Join(Dir(stateRoot), "activejobs.json") }

// counts returns total jobs and how many are terminal.
func (b *Batch) counts() (total, term int) {
	total = len(b.Jobs)
	for _, j := range b.Jobs {
		if j.terminal() {
			term++
		}
	}
	return total, term
}

// allTerminal reports whether every job in the batch has reached a terminal
// state (and there is at least one job).
func (b *Batch) allTerminal() bool {
	if len(b.Jobs) == 0 {
		return false
	}
	_, term := b.counts()
	return term == len(b.Jobs)
}

// finalState is the batch's terminal card state: err wins, then ok when any
// worker completed successfully, otherwise the all-cancelled batch is cancelled.
//
// err can only come from a member that genuinely reported one — the box's own
// terminalizations (lease reaper, restart sweep) are cancelled — so a card
// showing "failed" always means a worker said it failed.
func (b *Batch) finalState() string {
	hasOK := false
	for _, j := range b.Jobs {
		switch j.State {
		case stateErr:
			return stateErr
		case stateOK:
			hasOK = true
		}
	}
	if hasOK {
		return stateOK
	}
	return stateCancelled
}
