package mcpchan

import (
	"strings"
	"testing"
)

func TestPermPromptText(t *testing.T) {
	// Known tools read as a warm ask AND always surface the concrete target — the
	// safety rule: humanizing must never hide what an action touches.
	cases := []struct {
		tool, preview, wantVerb, wantTarget string
	}{
		{"Bash", "go test ./...", "run", "go test ./..."},
		{"Edit", "config.go", "edit", "config.go"},
		{"Write", "main.go", "edit", "main.go"},
		{"Read", "secrets.env", "read", "secrets.env"},
		{"WebFetch", "https://x.com", "fetch", "https://x.com"},
	}
	for _, c := range cases {
		got := PermPromptText(PermissionRequestParams{ToolName: c.tool, InputPreview: c.preview})
		if !strings.Contains(got, c.wantVerb) {
			t.Errorf("%s: want verb %q in %q", c.tool, c.wantVerb, got)
		}
		if !strings.Contains(got, c.wantTarget) {
			t.Errorf("%s: target %q must always be shown, got %q", c.tool, c.wantTarget, got)
		}
		if !strings.HasSuffix(got, "?") {
			t.Errorf("%s: humanized prompt should read as a question, got %q", c.tool, got)
		}
	}

	// Empty InputPreview falls back to Description.
	if got := PermPromptText(PermissionRequestParams{ToolName: "Edit", Description: "config.yaml"}); !strings.Contains(got, "config.yaml") {
		t.Errorf("expected description fallback, got %q", got)
	}

	// Unknown tools keep the explicit 🔐 <ToolName> form (no invented verb).
	got := PermPromptText(PermissionRequestParams{ToolName: "external_directory", InputPreview: "/etc"})
	if !strings.Contains(got, "🔐") || !strings.Contains(got, "external_directory") || !strings.Contains(got, "/etc") {
		t.Errorf("unknown tool should keep 🔐 name + target, got %q", got)
	}

	// Long previews are truncated with an ellipsis (full detail is behind "See more").
	if got := PermPromptText(PermissionRequestParams{ToolName: "Bash", InputPreview: strings.Repeat("a", 300)}); !strings.Contains(got, "…") {
		t.Errorf("expected truncation ellipsis, got %q", got)
	}
}

func TestPermVerdictLine(t *testing.T) {
	// Keeps the prompt's lead line so an answered bubble shows what was decided.
	if got := PermVerdictLine("✏️ edit config.go?", true); got != "✏️ edit config.go? — ✅ Allowed" {
		t.Errorf("allow: got %q", got)
	}
	if got := PermVerdictLine("⚙️ run rm -rf /?", false); got != "⚙️ run rm -rf /? — ❌ Denied" {
		t.Errorf("deny: got %q", got)
	}
	// Only the first line is kept (a "See more"-expanded prompt collapses to its lead).
	if got := PermVerdictLine("🔐 external_directory\n/etc/passwd", true); got != "🔐 external_directory — ✅ Allowed" {
		t.Errorf("multiline: got %q", got)
	}
	// Empty prompt text degrades to a bare verdict.
	if got := PermVerdictLine("", false); got != "❌ Denied" {
		t.Errorf("empty: got %q", got)
	}
}
