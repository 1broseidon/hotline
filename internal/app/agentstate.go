package app

import (
	"bytes"
	"encoding/json"
	"path/filepath"
	"time"
	"unicode/utf8"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/schedule"
)

// agent_state snapshot caps (SPEC §1.2).
const (
	maxAgentRuns       = 8
	maxAgentSchedules  = 24
	maxAgentLoops      = 24
	agentStateThrottle = 2 * time.Second
	scheduleLabelRunes = 60
)

// agentRun is one live job in the snapshot's runs list. Sourced from the
// app job registry (§2.2); restart-stale cards are deliberately excluded.
type agentRun struct {
	ID        string `json:"id"`
	Title     string `json:"title"`
	State     string `json:"state"`
	Detail    string `json:"detail,omitempty"`
	StartedAt int64  `json:"startedAt"`
}

// agentSchedule is one entry in the snapshot's schedules list, sourced from the
// schedule store.
type agentSchedule struct {
	ID         string `json:"id"`
	Label      string `json:"label"`
	Next       int64  `json:"next"`
	Recurrence string `json:"recurrence"`
}

// agentLoop is one entry in the snapshot's loops list, sourced from the loop
// store.
type agentLoop struct {
	ID      string `json:"id"`
	Label   string `json:"label"`
	Cadence string `json:"cadence"`
	State   string `json:"state"`
}

// AgentInfo is the box-side harness/model identity carried as optional,
// additive wire metadata on the two identity envelopes (welcome §3.2 and the
// agent_state snapshot §1.2). All fields optional; absent = unknown (old
// boxes). Harness is the kind ("claude" = the TUI, "claude-sdk", "pi",
// "opencode"); Model is the resolved or configured model id; Effort is the
// operator effort knob verbatim.
type AgentInfo struct {
	Harness string
	Model   string
	Effort  string
	// ModelKnown / EffortKnown record that the HARNESS reported this field, as
	// opposed to it merely being empty because nothing ever said otherwise.
	//
	// Empty-means-unknown cannot express a clear (hot-clear amendment). A model
	// cleared back to the harness default produces an empty value that used to
	// be indistinguishable from "not reported yet" — so the box merged nothing,
	// kept advertising the OLD model, and its no-op check went on believing that
	// model was live. Re-selecting it later was answered "already effective" and
	// never applied, with the app showing a model the session was not running.
	//
	// With these set, an empty-and-known field means "the harness says there is
	// no explicit value here", and the config fallback (which reads the boot
	// env, not the live session) is correctly skipped.
	ModelKnown  bool
	EffortKnown bool
}

// SetAgentInfo merges non-empty fields into the box identity and, on any
// change, schedules an agent_state emit so attached devices refresh live
// (the claude-sdk harness reports its RESOLVED model only once the SDK
// session initializes — after a device may already be connected).
//
// This is the value-merge entry point: it can SET a field, never clear one.
// The seed path and every non-harness caller use it. A harness reporting an
// explicit clear goes through MergeAgentInfo.
func (s *Server) SetAgentInfo(info AgentInfo) {
	model, effort := (*string)(nil), (*string)(nil)
	if info.Model != "" {
		model = &info.Model
	}
	if info.Effort != "" {
		effort = &info.Effort
	}
	s.MergeAgentInfo(info.Harness, model, effort)
}

// MergeAgentInfo merges a presence-aware identity report and, on any change,
// schedules an agent_state emit. nil leaves a field untouched; a pointer to ""
// is an explicit CLEAR — the value is emptied and marked known, so the box
// stops advertising the old one and its no-op check stops believing it.
func (s *Server) MergeAgentInfo(harness string, model, effort *string) {
	s.agentInfoMu.Lock()
	changed := false
	if harness != "" && harness != s.agentInfo.Harness {
		s.agentInfo.Harness = harness
		changed = true
	}
	if model != nil {
		if *model != s.agentInfo.Model || !s.agentInfo.ModelKnown {
			changed = changed || *model != s.agentInfo.Model
			s.agentInfo.Model = *model
			s.agentInfo.ModelKnown = true
		}
	}
	if effort != nil {
		if *effort != s.agentInfo.Effort || !s.agentInfo.EffortKnown {
			changed = changed || *effort != s.agentInfo.Effort
			s.agentInfo.Effort = *effort
			s.agentInfo.EffortKnown = true
		}
	}
	s.agentInfoMu.Unlock()
	if changed {
		s.agentStateChanged()
		// caps-design §5: a harness identity change (model/effort/kind) changes the
		// box-attested manifest — rebuild the outgoing cache (B2, off the hot path) then
		// resend to caps-sendable peers (debounced ≥30s, off the operator path).
		s.refreshCapsOut()
		s.fleetCapsChanged()
	}
}

func (s *Server) currentAgentInfo() AgentInfo {
	s.agentInfoMu.RLock()
	defer s.agentInfoMu.RUnlock()
	return s.agentInfo
}

// agentStateSnapshot is the full transient agent_state.state payload. The app
// treats each snapshot as a full replacement (§1.2). Slices are always non-nil
// so the frame carries [] not null.
type agentStateSnapshot struct {
	// Name is the box-owned assistant identity (FB21). Every device in every room
	// renders it from the live snapshot; omitempty on a never-seeded box keeps the
	// wire shape backward-compatible.
	Name string `json:"name,omitempty"`
	// Harness/Model/Effort are the box identity metadata (AgentInfo), riding
	// the snapshot next to Name exactly like FB35 — optional and additive.
	Harness   string          `json:"harness,omitempty"`
	Model     string          `json:"model,omitempty"`
	Effort    string          `json:"effort,omitempty"`
	Runs      []agentRun      `json:"runs"`
	Schedules []agentSchedule `json:"schedules"`
	Loops     []agentLoop     `json:"loops"`
}

// empty reports whether the snapshot carries nothing worth announcing. An
// empty snapshot is never the FIRST thing announced on the wire: the app's
// default per-bot state is already empty, so sending "nothing" before
// anything has been shown is pure noise. A transition from non-empty back to
// empty (work cleared) IS announced — that carries information. A snapshot
// carrying a Harness identity is NOT empty: an idle box still announces who
// it is (the original suppression rationale — the app default already equals
// the snapshot — no longer holds once identity rides the snapshot).
func (snap agentStateSnapshot) empty() bool {
	return snap.Harness == "" && len(snap.Runs) == 0 && len(snap.Schedules) == 0 && len(snap.Loops) == 0
}

// buildAgentState assembles the current snapshot from the job registry and the
// two file-backed stores. It reads the stores fresh each time (they are the
// source of truth and may be mutated out of process), so the snapshot is always
// disk-honest at emit time.
func (s *Server) buildAgentState() agentStateSnapshot {
	snap := agentStateSnapshot{Runs: []agentRun{}, Schedules: []agentSchedule{}, Loops: []agentLoop{}}
	if s.store != nil {
		// Box-owned identity (FB21): disk-honest at emit time, so a rename done in
		// another process (a CLI, or this box handling set_name) shows up live.
		if name, ok := s.store.IdentityName(); ok {
			snap.Name = name
		}
	}
	info := s.currentAgentInfo()
	snap.Harness, snap.Model, snap.Effort = info.Harness, info.Model, info.Effort
	if s.jobs != nil {
		snap.Runs = s.jobs.activeRuns(maxAgentRuns)
	}

	// Schedules: only active (non-paused) ones. The wire shape carries no
	// paused flag (§1.2), and a paused schedule has no honest next-fire
	// countdown — so excluding it is the no-staleness-lies reading.
	if d, err := schedule.Load(s.schedulesPath()); err == nil {
		for _, sc := range d.Schedules {
			if len(snap.Schedules) >= maxAgentSchedules {
				break
			}
			if sc.Paused {
				continue
			}
			next := int64(0)
			if t, perr := time.Parse(time.RFC3339, sc.NextFire); perr == nil {
				next = t.Unix()
			}
			snap.Schedules = append(snap.Schedules, agentSchedule{
				ID:         sc.ID,
				Label:      firstLineRunes(sc.Prompt, scheduleLabelRunes),
				Next:       next,
				Recurrence: schedule.Describe(sc.Recurrence),
			})
		}
	}

	// Loops: only approved ones (a pending loop is not standing work yet and
	// the wire enum is only active|paused, with no pending state to express).
	if d, err := loop.Load(loop.Path(s.stateRoot())); err == nil {
		for _, l := range d.Loops {
			if len(snap.Loops) >= maxAgentLoops {
				break
			}
			if !l.Approved {
				continue
			}
			state := "active"
			if l.Paused {
				state = "paused"
			}
			snap.Loops = append(snap.Loops, agentLoop{ID: l.Label, Label: l.Label, Cadence: l.Every, State: state})
		}
	}
	return snap
}

// stateRoot is where this box's schedule and loop stores live, not the app
// provider's own StateDir (which is <base>/app — see config.LoadApp).
// runChannel carries the resolved box root explicitly; the StateDir fallback
// keeps directly-constructed test configs working.
func (s *Server) stateRoot() string {
	if s.cfg.StateRoot != "" {
		return s.cfg.StateRoot
	}
	return s.cfg.StateDir
}

func (s *Server) schedulesPath() string { return filepath.Join(s.stateRoot(), "schedules.json") }

// agentStateChanged is the change hook: it emits a fresh snapshot to all active
// devices, throttled to at most one send per agentStateThrottle with a trailing
// send. The first change in a quiet period sends immediately; a burst coalesces
// into one accurate snapshot at the end of the window (SPEC §1.2). Identical
// consecutive snapshots are suppressed so a store's periodic no-op re-save (the
// scheduler ticker re-writes the file every scan) does not spam the channel.
func (s *Server) agentStateChanged() {
	s.asMu.Lock()
	defer s.asMu.Unlock()
	if s.asClosed {
		return // the server has shut down; never schedule new work
	}
	if s.asTimer != nil {
		return // a trailing send is already scheduled; it will carry the latest state
	}
	throttle := s.asThrottle
	if throttle <= 0 {
		throttle = agentStateThrottle
	}
	wait := throttle - time.Since(s.asLastSent)
	if wait <= 0 {
		s.broadcastAgentStateLocked()
		return
	}
	s.asTimer = time.AfterFunc(wait, func() {
		s.asMu.Lock()
		defer s.asMu.Unlock()
		s.asTimer = nil
		if s.asClosed {
			return
		}
		s.broadcastAgentStateLocked()
	})
}

// stopAgentStateEmitter marks the emitter closed and stops any pending
// trailing send. Called when Run exits so a stopped server never emits again
// (P2-5).
func (s *Server) stopAgentStateEmitter() {
	s.asMu.Lock()
	defer s.asMu.Unlock()
	s.asClosed = true
	if s.asTimer != nil {
		s.asTimer.Stop()
		s.asTimer = nil
	}
}

// broadcastAgentStateLocked computes and sends a snapshot to every active
// device over the transient path, deduping against the last broadcast. Caller
// holds asMu.
func (s *Server) broadcastAgentStateLocked() {
	s.asLastSent = time.Now()
	snap := s.buildAgentState()
	body, err := json.Marshal(snap)
	if err != nil {
		return
	}
	if bytes.Equal(body, s.asLastSnap) {
		return // no change since the last broadcast
	}
	if snap.empty() && s.asLastSnap == nil {
		return // never announce "nothing standing" before anything was shown
	}
	s.asLastSnap = append([]byte(nil), body...)
	s.emitTransient(func(seq uint64) []byte { return agentStateFrame(seq, snap) })
}

// broadcastAgentStateNow forces an immediate snapshot to every active device,
// bypassing the throttle and the "never announce empty first" guard. Used by the
// device set_name handler (FB21 §4): a rename must reach every device/room right
// away, even on a box with no standing work. It still refreshes the dedupe cache
// and last-sent time so a following throttled broadcast won't duplicate it.
func (s *Server) broadcastAgentStateNow() {
	s.asMu.Lock()
	defer s.asMu.Unlock()
	if s.asClosed {
		return
	}
	if s.asTimer != nil {
		s.asTimer.Stop()
		s.asTimer = nil
	}
	s.asLastSent = time.Now()
	snap := s.buildAgentState()
	body, err := json.Marshal(snap)
	if err != nil {
		return
	}
	s.asLastSnap = append([]byte(nil), body...)
	s.emitTransient(func(seq uint64) []byte { return agentStateFrame(seq, snap) })
}

// snapshotAgentStateTo sends the current snapshot to a single device. Used on
// subscribe (post-drain): a freshly connected device gets the full picture
// immediately, UNCONDITIONALLY — an empty snapshot ({runs:[],schedules:[],
// loops:[]}) is sent too, so a device holding a stale in-memory snapshot from
// before a box restart is corrected (ERRATA E7). Unthrottled and independent
// of the change-broadcast dedupe cache.
func (s *Server) snapshotAgentStateTo(deviceID string) {
	if s.jobs == nil {
		return
	}
	snap := s.buildAgentState()
	s.emitTransientTo(deviceID, func(seq uint64) []byte { return agentStateFrame(seq, snap) })
}

// registerStoreObservers wires the schedule and loop stores' mutation
// callbacks to agentStateChanged, scoped to THIS server's store paths, and
// returns the deregistration func (P2-5). Both stores fire their observers
// from the single Mutate write path, in-process — so a schedule
// create/cancel/fire or a loop setup/pause done in this process re-snapshots.
// Cross-process mutations (the loop runner, the CLI) are caught instead by the
// on-subscribe snapshot when a device (re)connects.
func (s *Server) registerStoreObservers() func() {
	unSched := schedule.Observe(s.schedulesPath(), s.agentStateChanged)
	unLoop := loop.Observe(loop.Path(s.stateRoot()), s.agentStateChanged)
	return func() {
		unSched()
		unLoop()
	}
}

// firstLineRunes collapses s to its first line then truncates so the result is
// at most n Unicode code points INCLUDING the ellipsis (ERRATA E5): a cut
// string keeps n-1 code points and appends "…".
func firstLineRunes(s string, n int) string {
	for i := 0; i < len(s); i++ {
		if s[i] == '\n' {
			s = s[:i]
			break
		}
	}
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	if n <= 1 {
		return "…"
	}
	r := []rune(s)
	return string(r[:n-1]) + "…"
}
