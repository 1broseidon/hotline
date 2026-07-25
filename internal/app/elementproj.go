package app

import (
	"encoding/json"
	"fmt"
	"os"
	"sync"
)

// FB92 — box-authored resolution edits.
//
// When an inbound `/el` action (pick/approve/deny/toggle) parses in
// handleDeviceSend, the box synthesizes a confirming edit frame that records the
// resolution (decision.chosenKey / approval.resolved / checklist item.done) into
// the target element, so cold hydration/replay renders the card settled rather
// than unanswered. The forward-to-agent turn is unchanged; the edit rides beside
// it.
//
// The synthesis needs the CURRENT folded payload of one (msgID, elID) target, but
// edit frames are per-element-id deltas, so the latest raw frame is NOT the folded
// state (a later text-only or sibling-only edit does not carry the target). A
// journal scan under deliveryMu is forbidden (unbounded, on the hot path), so the
// box maintains this projection instead: a live map[(msgID, elID)] → latest
// element payload, folded on every emitted msg/edit frame that carries elements
// and seeded once at startup from the journal (off the hot path). A miss is a
// no-op (forward-only, never a scan).

// projectionMaxMessages bounds how many element-bearing messages the projection
// tracks (depth). The journal is never trimmed, so without a bound a long-lived
// box would grow the projection without limit. Eviction is oldest-first by
// insertion order (the same FIFO shape as elementIndex). An action against an
// evicted message resolves to a projection miss = forward-only, exactly like an
// unknown message — an accepted degradation for very old cards.
const projectionMaxMessages = 1024

// elementProjection is the maintained (msgID, elID) → latest folded Element map.
// It is written on every durable emit that carries elements (foldFrame, under the
// server's deliveryMu) and read once per parsed action (latest, under the same
// mutex). Its own mutex keeps the boot-time seed (single-threaded, off the hot
// path) and the folds honest without assuming any particular caller lock.
//
// The fold MIRRORS the app's merge semantics so the projection never claims an
// element the app would not render (sol F2/F4): an entry is created ONLY by a
// msg frame (a message the app rendered), edits fold ONLY into an already-known
// message (the app ignores edits for unknown ids — orphan edits are dropped, not
// seeded), and within a message the 4-element cap is honored (same id replaces in
// place, a new id appends only while the count is under the cap, extras dropped —
// matching wire.ts mergeElements). This preserves the F4 skip rationale: a
// projection hit proves the app knows that element, so the box edit can never grow
// the count and elIndex.applyEdit is safe to skip.
type elementProjection struct {
	mu    sync.Mutex
	byMsg map[string]map[string]Element
	// order is the insertion order of message ids, oldest first, for FIFO
	// eviction once byMsg exceeds projectionMaxMessages.
	order []string
}

func newElementProjection() *elementProjection {
	return &elementProjection{byMsg: map[string]map[string]Element{}}
}

// foldFrame folds a just-emitted frame into the projection. A msg frame
// establishes (or refreshes) the message and folds any elements it carries; an
// edit frame folds only into an already-known message. Everything else (typing,
// react, sent, an edit for an unknown message) is a no-op. Frames were validated
// at emission time, so a decode miss here is treated as nothing to fold.
func (p *elementProjection) foldFrame(data []byte) {
	var f struct {
		T        string    `json:"t"`
		ID       string    `json:"id"`
		Elements []Element `json:"elements"`
	}
	if json.Unmarshal(data, &f) != nil || f.ID == "" {
		return
	}
	switch f.T {
	case "msg":
		p.mu.Lock()
		defer p.mu.Unlock()
		p.mergeLocked(p.ensureLocked(f.ID), f.Elements)
	case "edit":
		p.mu.Lock()
		defer p.mu.Unlock()
		m, known := p.byMsg[f.ID]
		if !known {
			return // orphan edit: the app ignores edits for unknown messages
		}
		p.mergeLocked(m, f.Elements)
	}
}

// ensureLocked returns the element map for msgID, creating it (and evicting the
// oldest tracked message past the cap) on first sight. Caller holds p.mu.
func (p *elementProjection) ensureLocked(msgID string) map[string]Element {
	if m := p.byMsg[msgID]; m != nil {
		return m
	}
	if len(p.order) >= projectionMaxMessages {
		delete(p.byMsg, p.order[0])
		p.order = p.order[1:]
	}
	m := map[string]Element{}
	p.byMsg[msgID] = m
	p.order = append(p.order, msgID)
	return m
}

// mergeLocked folds els into a message's element map with the app's cap-4 merge:
// a same-id element replaces in place (no growth), a new id appends only while the
// message is under maxElementsPerMessage, and any excess new ids are dropped —
// the same frames the app's wire.ts merge would drop. Caller holds p.mu.
func (p *elementProjection) mergeLocked(m map[string]Element, els []Element) {
	for _, el := range els {
		if _, exists := m[el.ID]; exists {
			m[el.ID] = el
		} else if len(m) < maxElementsPerMessage {
			m[el.ID] = el
		}
		// else: message already full of other ids — the app would drop this append,
		// so the projection must not claim it either.
	}
}

// seed replays the whole journal once at startup to reconstruct the projection.
// Runs single-threaded at server construction, before any client connects — the
// one bounded pass the review permits off the hot path. Frames are folded oldest
// first (framesAfter(0) preserves append/seq order), so later edits win.
func (p *elementProjection) seed(frames []journalFrame) {
	for _, fr := range frames {
		p.foldFrame(fr.data)
	}
}

// get returns the latest folded element for a (msgID, elID) target, or ok=false
// on a miss (unknown message, or a message that never carried that element id).
func (p *elementProjection) get(msgID, elID string) (Element, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	m := p.byMsg[msgID]
	if m == nil {
		return Element{}, false
	}
	el, ok := m[elID]
	return el, ok
}

// applyResolution returns the target with the action's resolution recorded and
// changed=true, or the untouched target and changed=false when the act does not
// apply (wrong element type, unmatched option key, malformed value) or is a
// no-change re-tap (same chosenKey, already-resolved approval, toggle to the same
// value). It mutates only a copy of the caller's Element: pointer fields are
// replaced (never aliased) and the checklist item slice is copied before a toggle,
// so the projection's stored payload is never disturbed.
func applyResolution(el Element, act elementAction) (Element, bool) {
	switch act.Act {
	case "pick":
		if el.El != "decision" {
			return el, false
		}
		var key string
		if json.Unmarshal(act.V, &key) != nil {
			return el, false
		}
		matched := false
		for _, opt := range el.Options {
			if opt.Key == key {
				matched = true
				break
			}
		}
		if !matched {
			return el, false
		}
		if el.ChosenKey != nil && *el.ChosenKey == key {
			return el, false // no-change re-pick
		}
		k := key
		el.ChosenKey = &k // last tap wins on replay (append order arbitrates)
		return el, true
	case "approve", "deny":
		if el.El != "approval" {
			return el, false
		}
		want := "approved"
		if act.Act == "deny" {
			want = "denied"
		}
		if el.Resolved != nil && *el.Resolved == want {
			return el, false // already resolved that way
		}
		w := want
		el.Resolved = &w
		return el, true
	case "toggle":
		if el.El != "checklist" {
			return el, false
		}
		var m map[string]bool
		if json.Unmarshal(act.V, &m) != nil || len(m) == 0 {
			return el, false
		}
		items := make([]checklistItem, len(el.Items)) // copy: never alias the projection slice
		copy(items, el.Items)
		changed := false
		for i := range items {
			if v, ok := m[items[i].Key]; ok && items[i].Done != v {
				items[i].Done = v
				changed = true
			}
		}
		if !changed {
			return el, false
		}
		el.Items = items
		return el, true
	default:
		return el, false
	}
}

// synthesizeResolutionEditLocked appends the confirming edit for a parsed element
// action, if one is warranted. It runs INSIDE handleDeviceSend while deliveryMu is
// already held, so it emits via emitLocked (never s.emit / EditMessage — the mutex
// is non-recursive and either would self-deadlock). It runs BEFORE the inbound
// message_id prediction (so the harness metadata still matches the eventual
// sent.id) and BEFORE the CID-keyed sent echo (so the echo's Sync fences the
// resolution; a crash before the echo replays the retry, which re-runs synthesis
// to a no-change → no duplicate edit).
//
// The confirming edit carries EXACTLY ONE element — the complete mutated target —
// and its text is synthesizedText of just that element (the sentinel the app maps
// to empty, preserving the message's existing text; siblings persist because the
// app merge is by element id). elIndex.applyEdit is deliberately SKIPPED: the
// projection lookup already proved the id lives on the message, so the edit cannot
// grow the element count, and a restart-empty index would falsely reject it.
//
// Every failure path is best-effort: a projection miss, a non-applicable/no-change
// action, a validation or size-cap rejection all leave the outbox untouched and
// return, NEVER blocking or failing the action's forward to the agent.
func (s *Server) synthesizeResolutionEditLocked(act elementAction) {
	target, ok := s.elementProj.get(act.Msg, act.El)
	if !ok {
		return // forward-only: the target message/element is not in the projection
	}
	mutated, changed := applyResolution(target, act)
	if !changed {
		return // wrong-type, unmatched key, or a no-change re-tap: emit nothing
	}
	raw, err := json.Marshal(mutated)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: FB92 marshal resolution el=%s: %v\n", act.El, err)
		return
	}
	// Validate exactly as EditMessage does before emission: element grammar +
	// per-element cap, then the complete-payload size cap probed with the widest
	// seq. A near-cap element that would exceed the limit with the resolution field
	// added is logged and dropped — the action still forwards.
	els, err := validateElements([]json.RawMessage{raw})
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: FB92 validate resolution msg=%s el=%s: %v\n", act.Msg, act.El, err)
		return
	}
	wireText := synthesizedText(els)
	if err := validatePayloadSize(editFrame(probeSeq, act.Msg, wireText, els)); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: FB92 resolution over payload cap msg=%s el=%s: %v\n", act.Msg, act.El, err)
		return
	}
	// SKIP elIndex.applyEdit (see doc comment). emitLocked appends to outbox.jsonl,
	// fans to every device, and foldFrame folds the resolved payload back into the
	// projection so an immediate re-tap reads a no-change.
	s.emitLocked(func(seq uint64) []byte { return editFrame(seq, act.Msg, wireText, els) })
}
