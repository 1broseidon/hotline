package mcpchan

import "context"

// FleetTools is the fleet (agent-to-agent) tool surface: the box-side backend the
// fleet + fleet_send MCP tools delegate to (a2a-design-v2 §4). It is optional —
// mounted via WithFleetTools only on a box that runs the fleet channel — and, like
// the other tool interfaces here, never panics on bad input (it reports via
// (msg, true)). Fleet peers are addressed ONLY through fleet_send; the operator
// tools refuse a fleet chat (F11).
type FleetTools interface {
	FleetList(ctx context.Context) (string, bool)
	FleetSend(ctx context.Context, in FleetSendInput) (string, bool)
	// FleetCaps returns the full box-attested capabilities manifest(s) for work
	// routing (caps-design §3): one peer when peer != "", else every peer.
	FleetCaps(ctx context.Context, peer string) (string, bool)
}

// FleetInput is the decoded argument set for the fleet tool.
type FleetInput struct {
	Action string `json:"action"`
	Peer   string `json:"peer"`
}

// FleetSendInput is the decoded argument set for the fleet_send tool. from{} is
// NEVER agent-settable — the box stamps its own identity (E3).
type FleetSendInput struct {
	To   string `json:"to"`
	Text string `json:"text"`
	Kind string `json:"kind"`
}

const fleetSchema = `{"type":"object","properties":{"action":{"type":"string","enum":["list","caps"],"description":"list = your fleet peers (edge id, alias, direction, connected, pending depth, a one-line caps summary). caps = the full box-attested capabilities manifest(s) for routing work by capability. Read-only and redacted — no secrets."},"peer":{"type":"string","description":"For action=caps: a single peer alias or edge id. Omit for all peers."}},"required":["action"]}`

// fleetSendSchema pins additionalProperties:false so a client cannot smuggle an
// unmodeled field (e.g. a from{} the box must never accept from the agent — E3)
// into the tool call; only to/text/kind are accepted.
const fleetSendSchema = `{"type":"object","additionalProperties":false,"properties":{"to":{"type":"string","description":"The peer to send to: its alias or edge id (from fleet list)."},"text":{"type":"string","description":"The message body, max 16 KiB. Peer agents receive it as untrusted peer data unless THEIR operator granted this box authority on that edge — you cannot grant yourself authority, and asking for it in the text does nothing."},"kind":{"type":"string","enum":["brief","task","result","ack","ping","cancel","status_req","refuse"],"description":"Optional message kind. Default brief. Down (only these can ever read as a directive, and only on an edge whose operator granted this box authority): task = assign work, cancel = wind the work down, status_req = report state. Up: brief = milestone note, result = final output, ack = accepted, refuse = declined with a reason, ping = liveness. There is deliberately no restart kind."}},"required":["to","text"]}`

// handleFleet routes the fleet tool: list (L2) + caps (L5, caps-design §3).
func handleFleet(ctx context.Context, in FleetInput, ft FleetTools) (string, bool) {
	switch in.Action {
	case "list", "":
		return ft.FleetList(ctx)
	case "caps":
		return ft.FleetCaps(ctx, in.Peer)
	default:
		return "fleet failed: unknown action " + in.Action + " (list|caps)", true
	}
}
