package app

import (
	"bytes"
	"encoding/json"
	"io"
	"regexp"
	"sort"
	"strings"
)

// Element-action bridge (SPEC §1.3, hardened per ERRATA E1-E3). A tap on an
// element rides the existing user-send path: the app sends a normal
// device_send whose text is a zero-width space + "/el " + one single-line JSON
// object. The box recognizes it at the inbound boundary and delivers it to the
// harness as a structured element_action; ANY malformation — over the byte
// cap, CR/LF anywhere, duplicate or unknown JSON keys, wrong types, an ID or
// value failing its grammar — falls open to a plain message (visible, inert).
// Existence of the referenced message/element is deliberately NOT validated
// (grammar only, E2): the harness already treats actions as untrusted input.
const (
	elementActionMarker = "\u200b/el " // zero-width space (U+200B) + "/el "

	// maxElementActionLen is measured over the COMPLETE canonical line in
	// UTF-8 bytes (marker + JSON), before any slicing (E1).
	maxElementActionLen = 512
)

var (
	// actionMsgRe is the existing frame-id grammar: a-<seq> (agent) or
	// u-<seq> (user) decimal ids.
	actionMsgRe = regexp.MustCompile(`^[au]-[0-9]{1,19}$`)
	// actionActRe is the action-verb grammar (E2).
	actionActRe = regexp.MustCompile(`^[a-z]{1,16}$`)
)

// elementAction is the decoded action payload. V is left raw so pick (a key
// string) and toggle (an object of bools) share one shape.
type elementAction struct {
	Msg string          `json:"msg"`
	El  string          `json:"el"`
	Act string          `json:"act"`
	V   json.RawMessage `json:"v"`
}

// parseElementAction recognizes an element-action send. On success it returns
// the decoded action, a control-character-free one-line summary for the
// harness turn body, a compacted value string for meta, and ok=true. On ANY
// malformation it returns ok=false so the caller treats the original text as a
// plain message (fail open).
func parseElementAction(text string) (a elementAction, summary, value string, ok bool) {
	if !strings.HasPrefix(text, elementActionMarker) {
		return elementAction{}, "", "", false
	}
	// E1: cap the COMPLETE line before any slicing; a canonical line is
	// single-line, so any CR/LF anywhere disqualifies it. No trimming.
	if len(text) > maxElementActionLen || strings.ContainsAny(text, "\r\n") {
		return elementAction{}, "", "", false
	}
	payload := []byte(text[len(elementActionMarker):])
	if len(payload) == 0 {
		return elementAction{}, "", "", false
	}
	// E2: strict decode — duplicate keys anywhere, unknown keys, wrong types,
	// or trailing content all disqualify.
	if hasDuplicateJSONKeys(payload) {
		return elementAction{}, "", "", false
	}
	dec := json.NewDecoder(bytes.NewReader(payload))
	dec.DisallowUnknownFields()
	if dec.Decode(&a) != nil {
		return elementAction{}, "", "", false
	}
	if _, err := dec.Token(); err != io.EOF { // exactly one JSON value
		return elementAction{}, "", "", false
	}
	// E2: ID/verb grammars. No existence validation.
	if !actionMsgRe.MatchString(a.Msg) || !elementIDRe.MatchString(a.El) || !actionActRe.MatchString(a.Act) {
		return elementAction{}, "", "", false
	}
	summary, ok = elementActionSummary(a)
	if !ok {
		return elementAction{}, "", "", false
	}
	return a, summary, compactJSON(a.V), true
}

// elementActionSummary validates the value against the action's exact shape
// and renders a terse readable line. String values are key-grammar-checked
// (E4's ^[A-Za-z0-9_-]{1,32}$ — the only strings an action may legally carry
// are element keys), which also guarantees no control characters can reach the
// harness summary (E3). Any mismatch = unrecognized (fail open).
func elementActionSummary(a elementAction) (string, bool) {
	switch a.Act {
	case "pick":
		var v string
		if json.Unmarshal(a.V, &v) != nil || !elementKeyRe.MatchString(v) {
			return "", false
		}
		return "chose " + v, true
	case "approve", "deny":
		// approve/deny carry no value: v absent or null only.
		if len(a.V) != 0 && !bytes.Equal(a.V, []byte("null")) {
			return "", false
		}
		if a.Act == "approve" {
			return "approved", true
		}
		return "denied", true
	case "toggle":
		var m map[string]bool
		if json.Unmarshal(a.V, &m) != nil || len(m) == 0 || len(m) > maxChecklistItems {
			return "", false
		}
		keys := make([]string, 0, len(m))
		for k := range m {
			if !elementKeyRe.MatchString(k) {
				return "", false
			}
			keys = append(keys, k)
		}
		sort.Strings(keys)
		var on, off []string
		for _, k := range keys {
			if m[k] {
				on = append(on, k)
			} else {
				off = append(off, k)
			}
		}
		var parts []string
		if len(on) > 0 {
			parts = append(parts, "ticked "+strings.Join(on, ", "))
		}
		if len(off) > 0 {
			parts = append(parts, "unticked "+strings.Join(off, ", "))
		}
		return strings.Join(parts, "; "), true
	default:
		return "", false
	}
}

// hasDuplicateJSONKeys reports whether raw contains a repeated key in any
// object at any depth (encoding/json silently keeps the last one — a smuggling
// vector under a strict contract, so duplicates disqualify the action).
// Malformed JSON reports true (the caller fails open either way).
func hasDuplicateJSONKeys(raw []byte) bool {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	type frame struct {
		isObj     bool
		keys      map[string]bool
		expectKey bool
	}
	var stack []*frame
	top := func() *frame {
		if len(stack) == 0 {
			return nil
		}
		return stack[len(stack)-1]
	}
	for {
		tok, err := dec.Token()
		if err == io.EOF {
			return false
		}
		if err != nil {
			return true
		}
		switch t := tok.(type) {
		case json.Delim:
			switch t {
			case '{':
				stack = append(stack, &frame{isObj: true, keys: map[string]bool{}, expectKey: true})
			case '[':
				stack = append(stack, &frame{})
			case '}', ']':
				stack = stack[:len(stack)-1]
				if f := top(); f != nil && f.isObj {
					f.expectKey = true // the closed container was a value
				}
			}
		case string:
			if f := top(); f != nil && f.isObj {
				if f.expectKey {
					if f.keys[t] {
						return true
					}
					f.keys[t] = true
					f.expectKey = false
				} else {
					f.expectKey = true // string value consumed
				}
			}
		default:
			if f := top(); f != nil && f.isObj {
				f.expectKey = true // scalar value consumed
			}
		}
	}
}

// setNameFromDeviceSend recognizes an FB21 rename control ride: a device_send
// whose "send" text payload is the serialized `{"t":"set_name","name":"…"}`
// line (the app's sendElementActionTo → sendTextTo mechanism, since it cannot
// emit a top-level websocket frame). It returns the trimmed name and ok=true
// when the text is a set_name object; the caller validates and consumes it
// silently. Anything else returns ok=false and flows on as a normal send.
func setNameFromDeviceSend(raw []byte) (name string, ok bool) {
	var f deviceSendFrame
	if json.Unmarshal(raw, &f) != nil {
		return "", false
	}
	pt, ok := exactType(f.Payload)
	if !ok || pt != "send" {
		return "", false
	}
	var p struct {
		Text string `json:"text"`
	}
	if json.Unmarshal(f.Payload, &p) != nil {
		return "", false
	}
	return parseSetName(p.Text)
}

// parseSetName reports whether text is a `{"t":"set_name","name":"…"}` control
// object and returns its trimmed name. ok=true means it IS a set_name control
// (even if the name is empty/invalid, so the caller consumes it rather than
// leaking the raw JSON to the harness); the caller enforces the 1..64 bound.
func parseSetName(text string) (name string, ok bool) {
	t := strings.TrimSpace(text)
	if !strings.HasPrefix(t, "{") {
		return "", false
	}
	var m struct {
		T    string `json:"t"`
		Name string `json:"name"`
	}
	if json.Unmarshal([]byte(t), &m) != nil || m.T != "set_name" {
		return "", false
	}
	return strings.TrimSpace(m.Name), true
}

// setPushPreviewFromDeviceSend recognizes an FB23 push-preview control ride: a
// device_send whose "send" text payload is the serialized
// `{"t":"set_push_preview","clear":<bool>}` line (same text-payload mechanism as
// set_name, since the app cannot emit a top-level websocket frame). isControl=true
// means the text IS a set_push_preview object, so the caller consumes it SILENTLY
// regardless of whether clear parsed. valid=true means clear was a proper bool and
// carries the device's preference. Anything else returns isControl=false and flows
// on as a normal send.
func setPushPreviewFromDeviceSend(raw []byte) (clear, valid, isControl bool) {
	var f deviceSendFrame
	if json.Unmarshal(raw, &f) != nil {
		return false, false, false
	}
	pt, ok := exactType(f.Payload)
	if !ok || pt != "send" {
		return false, false, false
	}
	var p struct {
		Text string `json:"text"`
	}
	if json.Unmarshal(f.Payload, &p) != nil {
		return false, false, false
	}
	return parseSetPushPreview(p.Text)
}

// parseSetPushPreview reports whether text is a
// `{"t":"set_push_preview","clear":<bool>}` control object. isControl=true means
// it IS a set_push_preview control (even if clear is missing/not a bool, so the
// caller consumes it rather than leaking the raw JSON to the harness). valid=true
// with clear set means the preference parsed cleanly and should be persisted.
func parseSetPushPreview(text string) (clear, valid, isControl bool) {
	t := strings.TrimSpace(text)
	if !strings.HasPrefix(t, "{") {
		return false, false, false
	}
	var probe struct {
		T string `json:"t"`
	}
	if json.Unmarshal([]byte(t), &probe) != nil || probe.T != "set_push_preview" {
		return false, false, false
	}
	// It IS a set_push_preview control: consume silently from here on.
	var m struct {
		Clear *bool `json:"clear"`
	}
	if json.Unmarshal([]byte(t), &m) != nil || m.Clear == nil {
		// Malformed / missing clear: ignore the value but still consume the frame.
		return false, false, true
	}
	return *m.Clear, true, true
}

// setJobCompletionPushFromDeviceSend recognizes the FB44 per-device completion
// notification control riding as a device_send "send" text payload, beside the
// FB23 set_push_preview control. Recognized controls are always consumed; valid
// reports whether enabled was a bool that should be persisted.
func setJobCompletionPushFromDeviceSend(raw []byte) (enabled, valid, isControl bool) {
	var f deviceSendFrame
	if json.Unmarshal(raw, &f) != nil {
		return false, false, false
	}
	pt, ok := exactType(f.Payload)
	if !ok || pt != "send" {
		return false, false, false
	}
	var p struct {
		Text string `json:"text"`
	}
	if json.Unmarshal(f.Payload, &p) != nil {
		return false, false, false
	}
	return parseSetJobCompletionPush(p.Text)
}

// parseSetJobCompletionPush reports whether text has the FB44 control shape
// `{"t":"set_job_completion_push","enabled":<bool>}`. A recognized
// control with missing or malformed enabled remains silently consumed while
// preserving the prior preference.
func parseSetJobCompletionPush(text string) (enabled, valid, isControl bool) {
	t := strings.TrimSpace(text)
	if !strings.HasPrefix(t, "{") {
		return false, false, false
	}
	var probe struct {
		T string `json:"t"`
	}
	if json.Unmarshal([]byte(t), &probe) != nil || probe.T != "set_job_completion_push" {
		return false, false, false
	}
	var m struct {
		Enabled *bool `json:"enabled"`
	}
	if json.Unmarshal([]byte(t), &m) != nil || m.Enabled == nil {
		return false, false, true
	}
	return *m.Enabled, true, true
}

// compactJSON returns raw with insignificant whitespace removed, or the input
// unchanged if it is not valid JSON.
func compactJSON(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var buf bytes.Buffer
	if json.Compact(&buf, raw) != nil {
		return string(raw)
	}
	return buf.String()
}
