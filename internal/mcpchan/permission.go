package mcpchan

import (
	"context"
	"regexp"
	"strings"
)

// PermissionRequestParams is the payload of an inbound permission_request.
type PermissionRequestParams struct {
	RequestID    string `json:"request_id"`
	ToolName     string `json:"tool_name"`
	Description  string `json:"description"`
	InputPreview string `json:"input_preview"`
}

// PermissionVerdictParams is the payload of an outbound permission verdict.
type PermissionVerdictParams struct {
	RequestID string `json:"request_id"`
	Behavior  string `json:"behavior"` // "allow" | "deny"
}

// PermissionHandler is invoked when an inbound permission_request arrives. It
// runs on its own goroutine so it never blocks the transport read loop.
type PermissionHandler func(ctx context.Context, p PermissionRequestParams)

// SendVerdict emits an allow/deny answer for a pending permission request.
func (n *Notifier) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return n.send(ctx, MethodPermissionVerdict, PermissionVerdictParams{
		RequestID: requestID,
		Behavior:  behavior,
	})
}

// PermReplyRe matches a permission text-reply: "yes xxxxx" / "n xxxxx", where
// the code is 5 lowercase letters a-z excluding 'l' (avoids 1/l confusion on
// phone keyboards). Case-insensitive; no surrounding chatter allowed.
var PermReplyRe = regexp.MustCompile(`(?i)^\s*(y|yes|n|no)\s+([a-km-z]{5})\s*$`)

// PermBtnRe matches inline-button callback data: perm:<action>:<code>.
var PermBtnRe = regexp.MustCompile(`^perm:(allow|deny|more):([a-km-z]{5})$`)

// BehaviorFromYesNo maps a y/yes/n/no token to "allow" or "deny".
func BehaviorFromYesNo(g string) string {
	if strings.HasPrefix(strings.ToLower(g), "y") {
		return "allow"
	}
	return "deny"
}
