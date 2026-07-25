package app

// Model catalog amendment 2026-07-20 — the app's model row stops guessing.
//
// Before this, the app rendered a CURATED list compiled into the client: a
// hand-maintained set of ids that may or may not exist on the box in front of
// it. This carries the box's OWN answer instead: the harness enumerates the
// models it can actually select, the box holds the latest one, and every
// attached device gets it.
//
// Three deliberate choices, all about size and staleness:
//
//  1. It is NOT a field on AgentInfo / harness_info. That frame restamps on
//     every model and effort change (and on every TUI Ctrl+P cycle); stapling a
//     model list to it would re-send the whole catalog several times per apply
//     to say nothing new. The catalog changes only when the harness restarts or
//     its registry moves, which is a completely different cadence.
//
//  2. It is PUSHED, not pulled. The client needs it the moment the settings
//     panel opens, and a pull would cost a round-trip (and a request state
//     machine, and a timeout surface) on every open. Pushing it once when a
//     device subscribes — right next to the agent_state snapshot that already
//     rides that moment — means the data is simply there, and a device that
//     attaches before the harness reports gets the later change broadcast.
//
//  3. Absence is meaningful and safe. A box that never sends a catalog (every
//     binary older than this amendment, plus any harness that hasn't
//     implemented it) leaves the app on its curated list, exactly as today.
//     There is no negotiation and no version flag.

import (
	"strings"
	"unicode/utf8"
)

// Catalog caps. The cap is generous versus a real operator scope (3-8 models)
// but bounds an UNSCOPED box, where the honest list is "every model with a
// credential" and a fat models.json could run long. A cut list is marked
// truncated; the client's free-text escape still reaches the full registry, so
// truncation narrows the menu, never the reach.
const (
	maxCatalogModels    = 64
	maxCatalogModelID   = 64
	maxCatalogLabel     = 40
	maxCatalogSourceLen = 16
)

// catalogModel is one selectable model on the app wire.
type catalogModel struct {
	ID    string `json:"id"`
	Label string `json:"label,omitempty"`
	// Available reports a usable credential on the box. Serialized always (not
	// omitempty): "this model is not usable" is information the row must be able
	// to render, and an omitted false would be indistinguishable from an old
	// frame that never carried the field.
	Available bool `json:"available"`
}

// AgentCatalog is the box-held model catalog, sourced from the harness and
// mirrored to devices on the transient agent_catalog frame. The zero value
// (no Models) means "no catalog" and is never emitted.
type AgentCatalog struct {
	Harness   string
	Source    string
	Models    []catalogModel
	Truncated bool
}

// empty reports whether this catalog carries nothing worth sending. A catalog
// with no models is not a narrower menu, it is no menu — emitting it would
// blank the client's rows and leave the operator with only free text, which is
// strictly worse than the curated fallback.
func (c AgentCatalog) empty() bool { return len(c.Models) == 0 }

// sanitizeAgentCatalog bounds and de-duplicates a harness-reported catalog
// before it is ever held or sent. The harness is our own code, but it reads a
// third-party registry (pi's models.json, plus whatever extensions register at
// runtime), so every string here is ultimately operator-supplied data and is
// treated as such: capped, single-lined, and de-duplicated.
//
// An entry with no id is dropped — the id is what a selection sends back, so a
// row without one could only ever fail. A missing label is left empty and the
// client renders the id.
func sanitizeAgentCatalog(harness, source string, models []catalogModel, truncated bool) AgentCatalog {
	out := AgentCatalog{
		Harness:   catalogLine(harness, maxCatalogSourceLen),
		Source:    catalogLine(source, maxCatalogSourceLen),
		Truncated: truncated,
	}
	seen := make(map[string]bool, len(models))
	for _, m := range models {
		id := catalogLine(m.ID, maxCatalogModelID)
		if id == "" || seen[id] {
			continue
		}
		if len(out.Models) >= maxCatalogModels {
			out.Truncated = true
			break
		}
		seen[id] = true
		out.Models = append(out.Models, catalogModel{
			ID:        id,
			Label:     catalogLine(m.Label, maxCatalogLabel),
			Available: m.Available,
		})
	}
	return out
}

// catalogLine flattens s to a single trimmed line and caps it at n runes
// (runes, not bytes: a label may legitimately carry non-ASCII, and a byte slice
// could split one mid-sequence and emit invalid UTF-8 on the wire).
func catalogLine(s string, n int) string {
	s = strings.TrimSpace(strings.Join(strings.Fields(s), " "))
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	return string([]rune(s)[:n])
}

// SetAgentCatalog replaces the box's catalog with the harness's latest report
// and broadcasts it when it actually changed. Replace, not merge: the harness
// sends a complete list every time, so a merge could only resurrect models a
// re-scoped box no longer offers.
//
// An empty report CLEARS the catalog (and broadcasts the clear), which is how
// a re-scoped box whose patterns now match nothing tells the app to fall back
// rather than keep showing a list that no longer exists.
func (s *Server) SetAgentCatalog(cat AgentCatalog) {
	s.agentCatalogMu.Lock()
	changed := !agentCatalogEqual(s.agentCatalog, cat)
	if changed {
		s.agentCatalog = cat
	}
	s.agentCatalogMu.Unlock()
	if changed {
		s.broadcastAgentCatalog()
	}
}

func (s *Server) currentAgentCatalog() AgentCatalog {
	s.agentCatalogMu.RLock()
	defer s.agentCatalogMu.RUnlock()
	return s.agentCatalog
}

// agentCatalogEqual is a field-wise compare — the change gate. Cheap enough at
// this size, and it keeps a harness that re-reports an identical catalog on
// every respawn from re-broadcasting to every attached device.
func agentCatalogEqual(a, b AgentCatalog) bool {
	if a.Harness != b.Harness || a.Source != b.Source || a.Truncated != b.Truncated || len(a.Models) != len(b.Models) {
		return false
	}
	for i := range a.Models {
		if a.Models[i] != b.Models[i] {
			return false
		}
	}
	return true
}

// broadcastAgentCatalog sends the current catalog to every active device. No
// throttle: unlike agent_state (which reacts to job/schedule/loop churn), this
// fires only on a real catalog change — at most once per harness (re)start.
func (s *Server) broadcastAgentCatalog() {
	cat := s.currentAgentCatalog()
	s.emitTransient(func(seq uint64) []byte { return agentCatalogFrame(seq, cat) })
}

// snapshotAgentCatalogTo sends the catalog to one freshly-subscribed device.
// Skipped when there is no catalog: a box with nothing to say must leave the
// client on its curated fallback, and an empty frame would be a claim that the
// box HAS a catalog and it is empty.
func (s *Server) snapshotAgentCatalogTo(deviceID string) {
	cat := s.currentAgentCatalog()
	if cat.empty() {
		return
	}
	s.emitTransientTo(deviceID, func(seq uint64) []byte { return agentCatalogFrame(seq, cat) })
}
