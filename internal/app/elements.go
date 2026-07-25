package app

import (
	"bytes"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"unicode/utf8"
)

// Element limits (SPEC §1.1). These are enforced box-side at the tool layer so
// an agent always gets a precise, learnable error rather than a silently
// mangled element on the wire.
const (
	maxElementsPerMessage = 4
	maxElementBytes       = 2 << 10 // 2 KiB per element, serialized
	// maxMessagePayloadBytes is the 16 KiB limit, defined per ERRATA E9 as the
	// canonical serialized bytes of the COMPLETE msg/edit payload (text +
	// elements + all fields), validated before enqueue.
	maxMessagePayloadBytes = 16 << 10
	maxFallbackRunes       = 200 // Unicode code points, both sides (E5)
	maxDecisionOptions     = 4
	maxChecklistItems      = 12
	maxThumbBytes          = 256 << 10 // 256 KiB per decision thumb
)

// elementFallbackSep joins element fallbacks into the synthesized text of an
// element-only message or edit (ERRATA E6). The exact byte sequence is part of
// the wire contract: the app suppresses a frame's text iff it equals the join.
const elementFallbackSep = " · "

// elementIDRe is the id grammar from the SPEC: el- then 1..32 of [A-Za-z0-9_-].
var elementIDRe = regexp.MustCompile(`^el-[A-Za-z0-9_-]{1,32}$`)

// elementKeyRe is the shared grammar for decision option keys and checklist
// item keys (ERRATA E4). Element-action values reference these keys, so the
// action parser enforces the same grammar on inbound values.
var elementKeyRe = regexp.MustCompile(`^[A-Za-z0-9_-]{1,32}$`)

var (
	chipKinds       = map[string]bool{"ok": true, "warn": true, "err": true, "info": true}
	jobStates       = map[string]bool{"running": true, "ok": true, "err": true, "cancelled": true}
	approvalResolve = map[string]bool{"approved": true, "denied": true}
)

// thumb is a decision option preview image. It reuses the existing blob/xfer
// transfer and media cache (see writeItemWithBlobs, which walks payloads for
// "xfer" keys), so the field name must stay "xfer".
type thumb struct {
	Xfer string `json:"xfer"`
	Mime string `json:"mime,omitempty"`
	Size int64  `json:"size,omitempty"`
}

// decisionOption is one pick-one choice.
type decisionOption struct {
	Key    string `json:"key"`
	Label  string `json:"label"`
	Detail string `json:"detail,omitempty"`
	Thumb  *thumb `json:"thumb,omitempty"`
}

// checklistItem is one tickable row.
type checklistItem struct {
	Key   string `json:"key"`
	Label string `json:"label"`
	Done  bool   `json:"done"`
}

// Element is the union of every element variant. json omitempty keeps a
// marshaled element to exactly the fields its variant uses, so the wire frame
// carries no empty variant fields and golden frames stay stable. Field order
// here is the marshaled order (encoding/json emits struct fields in
// declaration order), so it is deliberately: common fields, then chip, job,
// decision, approval, checklist.
type Element struct {
	El       string `json:"el"`
	ID       string `json:"id"`
	Fallback string `json:"fallback"`

	// chip
	Kind  string `json:"kind,omitempty"`
	Label string `json:"label,omitempty"`
	Value string `json:"value,omitempty"`

	// job
	Title     string   `json:"title,omitempty"`
	State     string   `json:"state,omitempty"`
	Detail    string   `json:"detail,omitempty"`
	StartedAt int64    `json:"startedAt,omitempty"`
	Progress  *float64 `json:"progress,omitempty"`

	// decision
	Prompt    string           `json:"prompt,omitempty"`
	Options   []decisionOption `json:"options,omitempty"`
	ChosenKey *string          `json:"chosenKey,omitempty"`

	// approval
	ApproveLabel string  `json:"approveLabel,omitempty"`
	DenyLabel    string  `json:"denyLabel,omitempty"`
	Resolved     *string `json:"resolved,omitempty"`

	// checklist
	Items []checklistItem `json:"items,omitempty"`
}

// validateElements decodes and validates a message's raw elements per SPEC
// §1.1, returning the canonical typed elements or a precise error naming the
// offending element index and the rule it broke. A nil/empty input is a
// no-error no-op (nil elements).
func validateElements(raw []json.RawMessage) ([]Element, error) {
	if len(raw) == 0 {
		return nil, nil
	}
	if len(raw) > maxElementsPerMessage {
		return nil, fmt.Errorf("at most %d elements per message, got %d", maxElementsPerMessage, len(raw))
	}
	out := make([]Element, 0, len(raw))
	seen := make(map[string]bool, len(raw))
	for i, rawEl := range raw {
		var el Element
		dec := json.NewDecoder(bytes.NewReader(rawEl))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&el); err != nil {
			return nil, fmt.Errorf("element %d: invalid element JSON (%v)", i, err)
		}
		if err := validateOneElement(i, el, seen); err != nil {
			return nil, err
		}
		seen[el.ID] = true
		canon, err := json.Marshal(el)
		if err != nil {
			return nil, fmt.Errorf("element %d (%s): could not serialize", i, el.ID)
		}
		if len(canon) > maxElementBytes {
			return nil, fmt.Errorf("element %d (%s): serialized size %d exceeds %d bytes", i, el.ID, len(canon), maxElementBytes)
		}
		out = append(out, el)
	}
	return out, nil
}

// synthesizedText is the E6 old-client text for an element-only message or
// edit: the elements' fallbacks joined by elementFallbackSep. Both sides
// compute it identically; the app suppresses a frame's text iff it equals it.
func synthesizedText(els []Element) string {
	fbs := make([]string, len(els))
	for i, el := range els {
		fbs[i] = el.Fallback
	}
	return joinFallbacks(fbs)
}

func joinFallbacks(fbs []string) string { return strings.Join(fbs, elementFallbackSep) }

// probeSeq / probeID are the widest possible seq and message id: E9's complete
// payload limit is checked on a probe frame built with them, so the probe is
// always at least as large as the frame the outbox will emit.
const probeSeq = uint64(1<<64 - 1)

const probeID = "a-18446744073709551615"

// validatePayloadSize enforces E9: the canonical serialized bytes of the
// complete msg/edit payload must fit maxMessagePayloadBytes.
func validatePayloadSize(frame []byte) error {
	if len(frame) > maxMessagePayloadBytes {
		return fmt.Errorf("message payload is %d bytes serialized; the limit is %d bytes total (text + elements + all fields)", len(frame), maxMessagePayloadBytes)
	}
	return nil
}

// elementIndexCap bounds how many element-carrying messages the index tracks
// (FIFO eviction). Messages emitted before a restart are unknown to the index:
// for those the edit's own ≤4 cap is the only enforceable bound, which is the
// most the box can honestly check.
const elementIndexCap = 512

// elementIndex tracks which element ids live on which message, so the box can
// reject an edit whose id-matched merge would grow a message past
// maxElementsPerMessage (ERRATA E8 belt; the app-side merge also drops excess
// appends).
type elementIndex struct {
	mu    sync.Mutex
	byMsg map[string]map[string]bool
	order []string
}

func newElementIndex() *elementIndex { return &elementIndex{byMsg: map[string]map[string]bool{}} }

// record notes the element ids a freshly emitted message carries.
func (x *elementIndex) record(msgID string, els []Element) {
	if len(els) == 0 {
		return
	}
	x.mu.Lock()
	defer x.mu.Unlock()
	x.ensureLocked(msgID)
	for _, el := range els {
		x.byMsg[msgID][el.ID] = true
	}
}

// applyEdit checks that merging els into msgID's known element set stays
// within maxElementsPerMessage, then records the merge. Same-id elements
// replace (no growth); new ids append only while the merged count fits.
func (x *elementIndex) applyEdit(msgID string, els []Element) error {
	if len(els) == 0 {
		return nil
	}
	x.mu.Lock()
	defer x.mu.Unlock()
	existing := x.byMsg[msgID]
	added := 0
	for _, el := range els {
		if !existing[el.ID] {
			added++
		}
	}
	if len(existing)+added > maxElementsPerMessage {
		return fmt.Errorf("edit would grow message %s to %d elements; the merge cap is %d (same ids replace, new ids append while under the cap)", msgID, len(existing)+added, maxElementsPerMessage)
	}
	x.ensureLocked(msgID)
	for _, el := range els {
		x.byMsg[msgID][el.ID] = true
	}
	return nil
}

func (x *elementIndex) ensureLocked(msgID string) {
	if x.byMsg[msgID] != nil {
		return
	}
	if len(x.order) >= elementIndexCap {
		delete(x.byMsg, x.order[0])
		x.order = x.order[1:]
	}
	x.byMsg[msgID] = map[string]bool{}
	x.order = append(x.order, msgID)
}

func validateOneElement(i int, el Element, seen map[string]bool) error {
	where := func(rule string) error { return fmt.Errorf("element %d (%s): %s", i, nonBlank(el.ID, "no-id"), rule) }
	if !elementIDRe.MatchString(el.ID) {
		return fmt.Errorf("element %d: id %q must match ^el-[A-Za-z0-9_-]{1,32}$", i, el.ID)
	}
	if seen[el.ID] {
		return where("duplicate id within message")
	}
	if el.Fallback == "" {
		return where("fallback is required")
	}
	if utf8.RuneCountInString(el.Fallback) > maxFallbackRunes {
		return where(fmt.Sprintf("fallback exceeds %d chars", maxFallbackRunes))
	}
	switch el.El {
	case "chip":
		if !chipKinds[el.Kind] {
			return where("chip kind must be one of ok|warn|err|info")
		}
		if el.Label == "" {
			return where("chip requires label")
		}
	case "job":
		if el.Title == "" {
			return where("job requires title")
		}
		if !jobStates[el.State] {
			return where("job state must be one of running|ok|err|cancelled")
		}
		if el.Progress != nil && (*el.Progress < 0 || *el.Progress > 1) {
			return where("job progress must be within 0..1")
		}
	case "decision":
		if el.Prompt == "" {
			return where("decision requires prompt")
		}
		if len(el.Options) == 0 || len(el.Options) > maxDecisionOptions {
			return where(fmt.Sprintf("decision needs 1..%d options, got %d", maxDecisionOptions, len(el.Options)))
		}
		keys := map[string]bool{}
		for j, opt := range el.Options {
			if opt.Label == "" {
				return where(fmt.Sprintf("decision option %d needs key and label", j))
			}
			if !elementKeyRe.MatchString(opt.Key) {
				return where(fmt.Sprintf("decision option %d key %q must match ^[A-Za-z0-9_-]{1,32}$", j, opt.Key))
			}
			if keys[opt.Key] {
				return where(fmt.Sprintf("decision option key %q is duplicated", opt.Key))
			}
			keys[opt.Key] = true
			if opt.Thumb != nil {
				if opt.Thumb.Xfer == "" {
					return where(fmt.Sprintf("decision option %d thumb requires xfer", j))
				}
				if opt.Thumb.Size < 0 || opt.Thumb.Size > maxThumbBytes {
					return where(fmt.Sprintf("decision option %d thumb size exceeds %d bytes", j, maxThumbBytes))
				}
			}
		}
	case "approval":
		if el.Title == "" {
			return where("approval requires title")
		}
		if el.Resolved != nil && !approvalResolve[*el.Resolved] {
			return where("approval resolved must be one of approved|denied")
		}
	case "checklist":
		if el.Title == "" {
			return where("checklist requires title")
		}
		if len(el.Items) == 0 || len(el.Items) > maxChecklistItems {
			return where(fmt.Sprintf("checklist needs 1..%d items, got %d", maxChecklistItems, len(el.Items)))
		}
		keys := map[string]bool{}
		for j, it := range el.Items {
			if it.Label == "" {
				return where(fmt.Sprintf("checklist item %d needs key and label", j))
			}
			if !elementKeyRe.MatchString(it.Key) {
				return where(fmt.Sprintf("checklist item %d key %q must match ^[A-Za-z0-9_-]{1,32}$", j, it.Key))
			}
			if keys[it.Key] {
				return where(fmt.Sprintf("checklist item key %q is duplicated", it.Key))
			}
			keys[it.Key] = true
		}
	default:
		return where(fmt.Sprintf("unknown el %q (chip|job|decision|approval|checklist)", el.El))
	}
	return nil
}
