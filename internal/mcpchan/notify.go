// Package mcpchan wires the channel to Claude Code over MCP. It builds the MCP
// server (initialize handshake, capabilities, tool registration) using the
// official Go SDK, and adds the one piece the SDK can't do natively: sending
// and receiving the custom claude/channel JSON-RPC notifications. Those are
// routed through a custom Transport/Connection so raw notification frames are
// serialized with the SDK's own writes and never interleave mid-line.
package mcpchan

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// JSON-RPC method names for the claude/channel protocol.
const (
	// MethodChannel delivers an inbound message to Claude.
	MethodChannel = "notifications/claude/channel"
	// MethodPermissionRequest is an inbound notification from Claude asking the
	// channel to relay a permission prompt.
	MethodPermissionRequest = "notifications/claude/channel/permission_request"
	// MethodPermissionVerdict is the outbound allow/deny answer to a permission
	// request.
	MethodPermissionVerdict = "notifications/claude/channel/permission"
	// MethodHarnessInfo is an inbound notification from an injected-harness
	// host (claude-sdk today) reporting the harness kind and the RESOLVED
	// model/effort for wire metadata (welcome §3.2 / agent_state §1.2).
	MethodHarnessInfo = "notifications/hotline/harness_info"
	// MethodSDKApply is an outbound notification to an injected-harness host
	// (claude-sdk) asking it to hot-apply a model change to the live SDK
	// session (SDK hot-model amendment 2026-07-19). Internal box↔harness
	// stdio contract only — never app wire.
	MethodSDKApply = "notifications/hotline/sdk_apply"
	// MethodSDKApplyResult is the inbound answer to a MethodSDKApply,
	// correlated by rid.
	MethodSDKApplyResult = "notifications/hotline/sdk_apply_result"
	// MethodHarnessCatalog is an inbound notification from an injected-harness
	// host reporting the model CATALOG the app should offer in its model row
	// (model catalog amendment 2026-07-20). Distinct from MethodHarnessInfo,
	// which reports the one live model: this reports the SELECTABLE set, and is
	// sent once per child-ready rather than on every identity restamp — the
	// whole reason it is not a harness_info field.
	MethodHarnessCatalog = "notifications/hotline/harness_catalog"
)

// AgentInfoParams is the payload of a MethodHarnessInfo notification.
//
// Model and Effort are POINTERS so the frame can say three different things,
// which a plain string cannot (hot-clear amendment):
//
//	nil          the harness did not report this field — leave it alone.
//	pointer ""   the harness reports it as CLEARED (back to the harness default,
//	             or unpinned on pi).
//	pointer "x"  the live value.
//
// The distinction is load-bearing. Clearing a model used to delete the env
// value and then omit the field, and the box merged only non-empty values — so
// the OLD identity stuck. The app kept displaying a model the session was no
// longer running, and, worse, the box's no-op check still believed that model
// was live, so re-selecting it later was answered "already effective" and never
// applied. An explicit clear has to be distinguishable from silence.
//
// Harness stays a plain string: it is always sent and never cleared.
type AgentInfoParams struct {
	Harness string  `json:"harness,omitempty"`
	Model   *string `json:"model,omitempty"`
	Effort  *string `json:"effort,omitempty"`
	// FallbackCount is the claude-sdk 0.2.0 delivery-lane counter (design §3.7):
	// how many operator turns the harness had to answer itself because the model
	// ended a turn without a reply call. It is the option-c decision input — if
	// it stays ~0 with the fixed instruction profile the model can be trusted;
	// if it climbs, harness-owned delivery gets revisited. Presence-pointer like
	// Model/Effort: nil = not reported (old harness, or lane never fired). Old
	// binaries unmarshal-ignore the unknown field.
	FallbackCount *int `json:"fallback_count,omitempty"`
}

// HarnessCatalogModel is one selectable model in a MethodHarnessCatalog
// payload. ID is the CANONICAL id the app sends back in a set_sdk_config
// (pi: "provider/id"); Label is a short display name; Available reports
// whether the box holds a usable credential for it.
type HarnessCatalogModel struct {
	ID        string `json:"id"`
	Label     string `json:"label,omitempty"`
	Available bool   `json:"available,omitempty"`
}

// HarnessCatalogParams is the payload of a MethodHarnessCatalog notification.
//
// Deliberately harness-agnostic: pi fills it from its ModelRegistry scoped by
// --models, and the claude-sdk harness can later fill the SAME shape from
// supportedModels() with no wire change. A harness that sends nothing leaves
// the app on its curated fallback list, which is exactly how every box that
// predates this amendment behaves.
//
// Source names where the list came from, so the app (and a human reading a log)
// can tell a deliberate operator scope from the unscoped default:
// "models" = the --models/HOTLINE_PI_MODELS scope, "settings" = pi's own
// enabledModels setting, "available" = everything with auth configured.
type HarnessCatalogParams struct {
	Harness string                `json:"harness,omitempty"`
	Source  string                `json:"source,omitempty"`
	Models  []HarnessCatalogModel `json:"models,omitempty"`
	// Truncated reports that the harness had more models than the cap and the
	// list was cut. The app still shows its free-text escape, which reaches the
	// full registry, so a truncated catalog is never a dead end.
	Truncated bool `json:"truncated,omitempty"`
}

// SDKApplyParams is the payload of a MethodSDKApply notification. Model and
// Effort are optional pointers with the same absent-vs-clear semantics as the
// app→box request: nil = the field was omitted (unchanged), a pointer to "" =
// an explicit clear to the SDK default (serializes as "model":""/"effort":"").
// At least one is non-nil (the box only forwards a hot-capable request).
type SDKApplyParams struct {
	RID    string  `json:"rid"`
	Model  *string `json:"model,omitempty"`
	Effort *string `json:"effort,omitempty"`
}

// SDKApplyResultParams is the payload of a MethodSDKApplyResult notification.
// On ok, Model/Effort echo the applied values for exactly the fields the
// request carried ("" for clear). On failure, Code is one of
// unknown_model|invalid_effort|apply_failed|no_session and Detail is a single
// line ≤ 200 chars.
type SDKApplyResultParams struct {
	RID    string `json:"rid"`
	OK     bool   `json:"ok"`
	Model  string `json:"model,omitempty"`
	Effort string `json:"effort,omitempty"`
	// Unverified is true on ok when the applied model was syntactically valid
	// but absent from the CLI's under-enumerating supportedModels() catalog
	// (Tier 2): the apply landed, but an API rejection could surface on the next
	// turn. Omitted (verified) for catalog hits and clears — additive, so old
	// harnesses' frames stay byte-identical.
	Unverified bool   `json:"unverified,omitempty"`
	Code       string `json:"code,omitempty"`
	Detail     string `json:"detail,omitempty"`
}

// InboundParams is the payload of a MethodChannel notification.
type InboundParams struct {
	Content string            `json:"content"`
	Meta    map[string]string `json:"meta"`
}

// Notifier sends custom claude/channel notifications over the same logical
// JSON-RPC connection the SDK uses, so raw frames can't interleave mid-line
// with SDK frames.
type Notifier struct {
	conn mcp.Connection
}

// SendChannel delivers an inbound message to Claude. Meta keys with empty
// values are dropped by the caller; this method emits whatever map it's given.
func (n *Notifier) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	return n.send(ctx, MethodChannel, InboundParams{Content: content, Meta: meta})
}

// SendSDKApply asks the injected-harness host to hot-apply model and/or effort
// to the live SDK session (rid correlates the MethodSDKApplyResult answer; a
// nil field is unchanged, a pointer to "" clears to the SDK default). Same send
// primitive as SendChannel, so the frame shares the connection's write
// serialization and can never interleave mid-line with SDK frames.
func (n *Notifier) SendSDKApply(ctx context.Context, rid string, model, effort *string) error {
	return n.send(ctx, MethodSDKApply, SDKApplyParams{RID: rid, Model: model, Effort: effort})
}

// send marshals params and writes a zero-ID JSON-RPC request (a notification)
// to the connection.
func (n *Notifier) send(ctx context.Context, method string, params any) error {
	if n == nil || n.conn == nil {
		return fmt.Errorf("notifier not connected")
	}
	raw, err := json.Marshal(params)
	if err != nil {
		return fmt.Errorf("marshaling %s params: %w", method, err)
	}
	// A notification is a *jsonrpc.Request with a zero (invalid) ID.
	req := &jsonrpc.Request{Method: method, Params: raw}
	return n.conn.Write(ctx, req)
}
