package app

import (
	"encoding/json"
	"fmt"
)

const ProtocolVersion = 2

type rawEnvelope map[string]json.RawMessage

func exactType(raw []byte) (string, bool) {
	var env rawEnvelope
	if err := json.Unmarshal(raw, &env); err != nil {
		return "", false
	}
	v, ok := env["t"]
	if !ok {
		return "", false
	}
	var t string
	if err := json.Unmarshal(v, &t); err != nil {
		return "", false
	}
	return t, true
}

type helloFrame struct {
	T          string `json:"t"`
	V          int    `json:"v"`
	DeviceID   string `json:"device_id"`
	Secret     string `json:"secret"`
	ResumeFrom string `json:"resume_from"`
	Push       *struct {
		Token    string `json:"token"`
		Platform string `json:"platform"`
	} `json:"push,omitempty"`
	// Caps is the device's optional capability advertisement (L4 post-mortem fix A1):
	// a device lists the additive transient families it can render, e.g. "fleet_state".
	// Absent/unknown on every shipped client, so an un-advertised family is not emitted
	// to it. Additive + old-safe: an old box ignores the field.
	Caps []string `json:"caps,omitempty"`
}

type deviceSendFrame struct {
	T       string          `json:"t"`
	CID     string          `json:"cid"`
	Payload json.RawMessage `json:"payload"`
}

type liveActivityTokenFrame struct {
	JobID string `json:"job_id"`
	Token string `json:"token"`
}

func mustMarshal(v any) []byte {
	b, err := json.Marshal(v)
	if err != nil {
		return []byte(`{"t":"error","code":"bad_frame"}`)
	}
	return b
}

func errorFrame(code, detail string) []byte {
	m := map[string]any{"t": "error", "code": code}
	if detail != "" {
		m["detail"] = detail
	}
	return mustMarshal(m)
}

// welcomeFrame carries the box identity on every (re)connect. The optional
// harness/model/effort metadata (AgentInfo, §3.2) is additive: empty fields
// are omitted, so old boxes' frames are byte-identical and old apps ignore
// the unknown keys.
func welcomeFrame(room RoomRecord, deviceID, floor, head string, info AgentInfo) []byte {
	m := map[string]any{"t": "welcome", "v": ProtocolVersion, "room": room.ID, "name": room.Name, "device_id": deviceID, "floor": floor, "head": head}
	if info.Harness != "" {
		m["harness"] = info.Harness
	}
	if info.Model != "" {
		m["model"] = info.Model
	}
	if info.Effort != "" {
		m["effort"] = info.Effort
	}
	return mustMarshal(m)
}

func mailboxGapFrame(floor string) []byte {
	return mustMarshal(map[string]any{"t": "mailbox_gap", "floor": floor})
}

func pongFrame(n int64) []byte { return mustMarshal(map[string]any{"t": "pong", "n": n}) }

type fileRef struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	Mime string `json:"mime,omitempty"`
	Size int64  `json:"size,omitempty"`
	Xfer string `json:"xfer"`
}

type artifactRef struct {
	Title   string `json:"title"`
	Mime    string `json:"mime"`
	Size    int64  `json:"size"`
	Sandbox string `json:"sandbox"`
	Xfer    string `json:"xfer"`
}

func typingFrame(seq uint64, on bool) []byte {
	return mustMarshal(map[string]any{"t": "typing", "seq": seq, "on": on})
}

func msgFrame(seq uint64, id, text string, buttons []string, replyTo string, file *fileRef, elements []Element) []byte {
	m := map[string]any{"t": "msg", "seq": seq, "id": id, "text": text}
	if len(buttons) > 0 {
		m["buttons"] = buttons
	}
	if replyTo != "" {
		m["reply_to"] = replyTo
	}
	if file != nil {
		m["file"] = file
	}
	if len(elements) > 0 {
		m["elements"] = elements
	}
	return mustMarshal(m)
}

func artifactMsgFrame(seq uint64, id, text string, artifact *artifactRef) []byte {
	return mustMarshal(map[string]any{"t": "msg", "seq": seq, "id": id, "text": text, "artifact": artifact})
}

// editFrame builds an edit. An empty text with non-empty elements is an
// element-only edit (SPEC §1.1): the app leaves the message's text untouched
// and applies the id-matched element merge. text is always present on the wire
// so the empty-text signal is explicit.
func editFrame(seq uint64, id, text string, elements []Element) []byte {
	m := map[string]any{"t": "edit", "seq": seq, "id": id, "text": text}
	if len(elements) > 0 {
		m["elements"] = elements
	}
	return mustMarshal(m)
}

// agentStateFrame builds the transient agent_state snapshot frame (SPEC §1.2).
// Like typing it is never durable, never acked, never replayed.
func agentStateFrame(seq uint64, state agentStateSnapshot) []byte {
	return mustMarshal(map[string]any{"t": "agent_state", "seq": seq, "state": state})
}

// agentCatalogFrame builds the transient agent_catalog frame (model catalog
// amendment 2026-07-20): the SELECTABLE model list the box's harness
// enumerated, sent to a device on subscribe and re-broadcast on change.
//
// A member of the agent_state/typing transient family — seq'd, never durable,
// never replayed — and additive: a box that sends none leaves old and new apps
// alike on their curated list, and an app that doesn't know the type drops it.
// `models` is always an array (never null) so a client can index it without a
// null check.
func agentCatalogFrame(seq uint64, cat AgentCatalog) []byte {
	models := cat.Models
	if models == nil {
		models = []catalogModel{}
	}
	m := map[string]any{"t": "agent_catalog", "seq": seq, "models": models}
	if cat.Harness != "" {
		m["harness"] = cat.Harness
	}
	if cat.Source != "" {
		m["source"] = cat.Source
	}
	if cat.Truncated {
		m["truncated"] = true
	}
	return mustMarshal(m)
}

// sdkConfigResultFrame builds the transient per-device sdk_config_result
// frame (SDK-settings amendment; fixture sdk-config.json SC3). Success echoes
// the accepted post-trim values for exactly the fields the request carried:
// nil = the request omitted the field (omitted here too), pointer-to-"" = an
// explicit clear that MUST serialize as "" — hence pointers, not omitempty.
// hot marks a model-only success applied to the LIVE SDK session (SDK
// hot-model amendment 2026-07-19, fixture SC6): serialized only when true, so
// old boxes' frames are byte-identical and old apps drop the unknown field.
// unverified marks a hot apply whose model was syntactically valid but absent
// from the CLI's under-enumerating catalog (unverified-model amendment
// 2026-07-19, fixture SC11): serialized only when true, so verified applies
// stay byte-identical and old apps drop the unknown field. The apply still
// succeeded; the client surfaces a gentle caution.
// Errors carry the closed-set code and an optional single-line detail.
func sdkConfigResultFrame(seq uint64, rid string, ok bool, model, effort *string, restart, hot, unverified bool, code, detail string) []byte {
	m := map[string]any{"t": "sdk_config_result", "seq": seq, "rid": rid, "ok": ok}
	if ok {
		m["restart"] = restart
		if hot {
			m["hot"] = true
		}
		if unverified {
			m["unverified"] = true
		}
		if model != nil {
			m["model"] = *model
		}
		if effort != nil {
			m["effort"] = *effort
		}
	} else {
		m["code"] = code
		if detail != "" {
			m["detail"] = detail
		}
	}
	return mustMarshal(m)
}

func reactFrame(seq uint64, msgID, emoji, device string) []byte {
	m := map[string]any{"t": "react", "seq": seq, "msg_id": msgID, "emoji": emoji}
	if device != "" {
		m["device"] = device
	}
	return mustMarshal(m)
}

func sentFrame(seq uint64, id, text, cid, device, kind string, file *fileRef) []byte {
	m := map[string]any{"t": "sent", "seq": seq, "id": id, "text": text, "device": device}
	if cid != "" {
		m["cid"] = cid
	}
	if kind != "" && kind != "text" {
		m["kind"] = kind
	}
	if file != nil {
		m["file"] = file
	}
	return mustMarshal(m)
}

func frameID(frame []byte) string {
	var p struct {
		ID string `json:"id"`
	}
	_ = json.Unmarshal(frame, &p)
	return p.ID
}

func quotedFrom(frame []byte) (bool, string) {
	var p struct {
		T    string `json:"t"`
		Text string `json:"text"`
	}
	_ = json.Unmarshal(frame, &p)
	return p.T == "sent", p.Text
}

// readFrame builds the transient read-state frame (§4). j is a decimal string in
// GLOBAL journal-seq space (never per-device mailbox m-space). It carries no seq
// and is never durable, never acked, never replayed — an offline device converges
// via the post-drain snapshot instead. Identical shape both directions.
func readFrame(j string) []byte {
	return mustMarshal(map[string]any{"t": "read", "j": j})
}

func journalString(seq uint64) string { return fmt.Sprint(seq) }
