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

// permLead maps common Claude Code tools to a friendly verb + emoji so the prompt
// reads like a person asking ("✏️ edit config.go?") instead of a raw tool dump. The
// concrete target from the preview is always shown alongside (see PermPromptText),
// so humanizing never hides what an action touches.
// Matched case-insensitively so both harnesses humanize: Claude Code sends
// capitalized tool names ("Edit", "Bash"), the OpenCode permission relay sends
// lowercase permission types ("edit", "bash").
func permLead(tool string) (emoji, verb string, ok bool) {
	switch strings.ToLower(tool) {
	case "edit", "write", "multiedit", "notebookedit", "patch":
		return "✏️", "edit", true
	case "read", "notebookread":
		return "📖", "read", true
	case "bash", "bashoutput", "killshell":
		return "⚙️", "run", true
	case "grep", "glob", "ls":
		return "🔍", "search", true
	case "webfetch":
		return "🌐", "fetch", true
	case "websearch":
		return "🌐", "search the web for", true
	case "task":
		return "🤖", "run a subagent on", true
	}
	return "", "", false
}

// PermPromptText renders the collapsed permission prompt as a warm, plain-language
// ask ("✏️ edit config.go?") while always surfacing the concrete target (file /
// command / url), so the person can decide without tapping "See more" and a
// humanized prompt can never make a destructive action look innocuous. Unknown
// tools keep the explicit 🔐 <ToolName> form. Full detail stays behind "See more".
func PermPromptText(p PermissionRequestParams) string {
	preview := strings.Join(strings.Fields(p.InputPreview), " ")
	if preview == "" {
		preview = strings.Join(strings.Fields(p.Description), " ")
	}
	if r := []rune(preview); len(r) > 200 {
		preview = strings.TrimSpace(string(r[:200])) + "…"
	}
	emoji, verb, ok := permLead(p.ToolName)
	if !ok {
		lead := "🔐 " + p.ToolName
		if preview == "" {
			return lead
		}
		return lead + "\n" + preview
	}
	if preview == "" {
		return emoji + " " + verb + "?"
	}
	return emoji + " " + verb + " " + preview + "?"
}

// PermVerdictLine folds an allow/deny outcome back onto the original prompt's lead
// line so an answered bubble keeps its context ("✏️ edit config.go? — ✅ Allowed")
// instead of collapsing to a bare verdict. promptText is the message text as sent
// (possibly "See more"-expanded); only its first line is kept.
func PermVerdictLine(promptText string, allow bool) string {
	verdict := "✅ Allowed"
	if !allow {
		verdict = "❌ Denied"
	}
	lead := strings.TrimSpace(promptText)
	if i := strings.IndexByte(lead, '\n'); i >= 0 {
		lead = strings.TrimSpace(lead[:i])
	}
	if lead == "" {
		return verdict
	}
	return lead + " — " + verdict
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
