package mcpchan

import (
	"context"
	"encoding/json"

	"github.com/1broseidon/hotline/internal/mc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// missionSchema is the verbatim InputSchema for the mission tool (Mission
// Control spec §3): one verb-based tool, four actions, direct-write (no spool).
const missionSchema = `{"type":"object","properties":{"action":{"type":"string","enum":["note","update","handoff","archive"],"description":"note: log a durable fact/event. update: upsert a thread and sync the index. handoff: save resume state for the next session. archive: close a thread."},"thread":{"type":"string","description":"Thread slug, kebab-case (e.g. relay-cors). update: required, creates the thread if new. note: optional — with it the note appends to that thread's log, without it to standing notes. archive: required."},"text":{"type":"string","description":"note: the fact or event, one to three sentences. update: optional log line appended to the thread."},"status":{"type":"string","enum":["active","paused","done"],"description":"update: thread status. done also archives (equivalent to archive)."},"summary":{"type":"string","description":"update: one line — what this thread is."},"next":{"type":"string","description":"update: one line — the next action. handoff: required — what the next session should do first."},"state":{"type":"string","description":"handoff: required — what you were doing and where it stands, a few lines."},"beware":{"type":"string","description":"handoff: optional — constraints or landmines the next session must not trip."},"outcome":{"type":"string","description":"archive: required — one line on how it ended."},"trigger":{"type":"string","enum":["manual","pre-compact","boundary","auto"],"description":"handoff: optional — what prompted this handoff (default manual). Use pre-compact when writing ahead of a context compaction, boundary when a task just wrapped."}},"required":["action"]}`

// missionDesc is the mission tool's description (a peer of the job tool's).
const missionDesc = "Write to mission control — your durable working memory on disk (reads are free: the mc-index in your instructions is the map). Verbs: note {text, thread?} logs a durable fact (to a thread's log with thread, else to standing notes); update {thread, status?, summary?, next?, text?} upserts a thread and re-syncs the index; handoff {state, next, beware?} saves resume state before context runs long or a restart; archive {thread, outcome} closes a thread. The tool owns every timestamp, the front-matter, and the index — you only supply the facts. File as you go; update the thread you're on before telling the user it's done."

// missionInput is the decoded argument set for the mission tool.
type missionInput struct {
	Action  string `json:"action"`
	Thread  string `json:"thread"`
	Text    string `json:"text"`
	Status  string `json:"status"`
	Summary string `json:"summary"`
	Next    string `json:"next"`
	State   string `json:"state"`
	Beware  string `json:"beware"`
	Outcome string `json:"outcome"`
	Trigger string `json:"trigger"`
}

// mcMount is the resolved Mission Control runtime for a session: the store to
// write through and the parameters for the injected map. nil ⇒ MC not mounted.
type mcMount struct {
	store   *mc.Store
	budget  int
	harness Harness
}

// serverConfig collects ServerOptions applied at construction.
type serverConfig struct {
	mc    *mcMount
	fleet FleetTools
}

// ServerOption customizes NewServer / NewPiServer. It exists so Mission Control
// (and any future opt-in surface) can be threaded in without perturbing the
// existing constructor signatures or their many call sites.
type ServerOption func(*serverConfig)

// WithMissionControl mounts Mission Control: it registers the mission tool and
// injects the teaching segment + index map (for non-Claude harnesses) or the
// pointer line (for Claude on HOTLINE_MISSION_CONTROL=1). harness selects the
// injection style; store is the write path; budget caps the injected index.
func WithMissionControl(store *mc.Store, budget int, harness Harness) ServerOption {
	return func(c *serverConfig) {
		c.mc = &mcMount{store: store, budget: budget, harness: harness}
	}
}

// WithFleetTools mounts the fleet channel's tools (fleet + fleet_send). Passed
// only on a box running the A2A fleet lane; nil leaves both tools unregistered.
func WithFleetTools(ft FleetTools) ServerOption {
	return func(c *serverConfig) {
		c.fleet = ft
	}
}

func collectOptions(opts []ServerOption) serverConfig {
	var c serverConfig
	for _, o := range opts {
		o(&c)
	}
	return c
}

// mcTeachingSegment is the ~540-char teaching paragraph (spec §2), dir spliced
// in. It ships to non-Claude harnesses, immediately before the index block.
func mcTeachingSegment(dir string) string {
	return "Mission control is your memory across sessions, at " + dir + ". The mc-index block below is your map — you wake up already holding it; read a thread file only when you need its detail. File as you go with the mission tool: note logs a durable fact or event; update upserts a thread (status, summary, next) and keeps the index current; handoff saves resume state before context runs long or a restart; archive closes a thread. Anything not filed is gone after a restart. Update the thread you're working before you tell the user it's done."
}

// mcPointerSegment is the ~110-char Claude pointer line (spec §2): Claude has a
// Read tool and native memory, so it reads the index itself rather than having
// the body injected against the capped instruction budget.
func mcPointerSegment(dir string) string {
	return "Mission control memory: " + dir + "/INDEX.md — read it first each session; file updates with the mission tool."
}

// mcMechanics returns the Mission Control instruction paragraphs to append after
// the built-in mechanics (before the voice). Claude gets the pointer line;
// non-Claude harnesses get the teaching segment plus the rendered <mc-index>
// block (omitted only when the render is empty).
func mcMechanics(m *mcMount) []string {
	if m == nil || m.store == nil {
		return nil
	}
	dir := m.store.Dir()
	if m.harness == HarnessClaude {
		return []string{mcPointerSegment(dir)}
	}
	out := []string{mcTeachingSegment(dir)}
	if idx := m.store.RenderIndex(m.budget); idx != "" {
		out = append(out, idx)
	}
	// Fresh-at-boundary doctrine (spec §4): Pi only, and only when MC is mounted
	// (it references the mission handoff + restart tool, both Pi-side). OpenCode
	// has no restart tool and no compaction seam yet, so it never ships there.
	if m.harness == HarnessPi {
		out = append(out, mcDoctrineSegment)
	}
	return out
}

// mcDoctrineSegment is the fresh-at-boundary doctrine (spec §4): finish a task on
// a long context → hand off and restart fresh holding the map; mid-task → let
// compaction run and keep the in-context momentum.
const mcDoctrineSegment = "When a task wraps and your context is past half its cap, write a mission handoff (trigger: boundary) and call the restart tool — a fresh session holding the map beats a long one dragging its history. Mid-task, don't restart: let compaction run and keep going."

// addMissionTool registers the mission tool, delegating every verb to the store
// (direct write under its flock; never a spool). Gated on MC being mounted.
func addMissionTool(s *mcp.Server, store *mc.Store) {
	addTool(s, "mission", missionDesc, missionSchema,
		func(ctx context.Context, raw json.RawMessage) (string, bool) {
			var in missionInput
			if err := json.Unmarshal(raw, &in); err != nil {
				return "mission failed: " + err.Error(), true
			}
			return store.Apply(mc.Input{
				Action:  in.Action,
				Thread:  in.Thread,
				Text:    in.Text,
				Status:  in.Status,
				Summary: in.Summary,
				Next:    in.Next,
				State:   in.State,
				Beware:  in.Beware,
				Outcome: in.Outcome,
				Trigger: in.Trigger,
			})
		})
}
