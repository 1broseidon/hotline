// Package harness defines the thin seam between hotline and a coding-agent
// harness. It captures exactly the two things that differ between harnesses —
// pushing an inbound user turn IN, and relaying a permission prompt back OUT —
// and nothing else. Everything a harness shares (the outbound
// reply/react/edit_message/download_attachment MCP tool surface) stays on the
// existing mcpchan.ToolSet and is untouched.
//
// Claude Code is deliberately NOT expressed through this seam: it rides the MCP
// claude/channel notifications (internal/mcpchan) and stays exactly as it was.
// The seam exists so a second harness — OpenCode, the first concrete Link — can
// be driven over its own HTTP+SSE control plane without disturbing the Claude
// Code path or the messaging providers. Unifying Claude Code into a Link would
// be a larger, riskier refactor with no payoff today, so it is left out.
package harness

import "context"

// Inbound is a user turn to inject into the harness's active session. Content
// is the rendered message text; Meta carries the same normalized fields the
// providers stamp (chat_id, user, source, …) for a Link that wants them. A Link
// that only speaks plain text (OpenCode) may ignore Meta.
type Inbound struct {
	Content string
	Meta    map[string]string
}

// PermissionRequest is a harness-emitted permission prompt, normalized so the
// existing provider relay (which speaks mcpchan.PermissionRequestParams) can
// fan it out unchanged. ID is a short code the relay echoes back verbatim in
// AnswerPermission — a Link whose harness uses longer native permission ids
// maps them to a relay-friendly code itself.
type PermissionRequest struct {
	ID           string
	ToolName     string
	Description  string
	InputPreview string
}

// Link is the harness-side seam: inbound push + permission relay. A Link owns
// its own transport to the harness (HTTP+SSE for OpenCode); it does NOT serve
// the outbound MCP tools, which remain on mcpchan.ToolSet.
type Link interface {
	// Start runs the Link's control-plane loop (resolve the target session,
	// tail the event stream, …) until ctx is cancelled. It blocks and returns
	// nil on a clean ctx-driven stop.
	Start(ctx context.Context) error

	// PushInbound injects a user turn into the harness's active session.
	PushInbound(ctx context.Context, in Inbound) error

	// Permissions is the stream of permission prompts the harness raised. It is
	// closed when the Link's Start loop returns.
	Permissions() <-chan PermissionRequest

	// AnswerPermission answers a pending prompt by its relayed ID.
	AnswerPermission(ctx context.Context, id string, allow bool) error
}
