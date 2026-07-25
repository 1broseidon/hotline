package loop

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/supervise"
)

// YoloEnabled reports hotline's current no-approval posture. Claude exposes it
// as --dangerously-skip-permissions in supervisor state; OpenCode exposes the
// equivalent through opencode.json's permission.bash=allow.
func YoloEnabled(stateRoot string) (bool, error) {
	if truthy(os.Getenv("HOTLINE_YOLO")) {
		return true, nil
	}
	if stateRoot != "" {
		st, err := supervise.ReadState(supervise.Dir(stateRoot))
		if err != nil {
			return false, err
		}
		if st != nil && slices.Contains(st.Argv, "--dangerously-skip-permissions") {
			return true, nil
		}
	}
	h, err := config.Harness()
	if err != nil {
		return false, err
	}
	switch h {
	case "pi":
		// The pi harness runs every tool unguarded, always — Pi has no
		// permission prompts by design, so harness=pi is equivalent to
		// `claude --dangerously-skip-permissions` (see cmd_up.go's loud warning
		// and run_pi.go). Mirror HOTLINE_YOLO semantics here so loop approval
		// posture matches: setup_loop under pi goes live immediately, exactly as
		// it does under claude+yolo, instead of sitting pending-approval.
		return true, nil
	case "opencode":
		return openCodeBashAllowed("."), nil
	default:
		return false, nil
	}
}

func truthy(s string) bool {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "1", "true", "yes", "y", "on":
		return true
	default:
		return false
	}
}

func openCodeBashAllowed(dir string) bool {
	raw, err := os.ReadFile(filepath.Join(dir, "opencode.json"))
	if err != nil {
		return false
	}
	var root map[string]any
	if err := json.Unmarshal(raw, &root); err != nil {
		return false
	}
	perm, _ := root["permission"].(map[string]any)
	return permissionAllows(perm["bash"])
}

func permissionAllows(v any) bool {
	switch x := v.(type) {
	case string:
		return strings.EqualFold(strings.TrimSpace(x), "allow")
	case bool:
		return x
	default:
		return false
	}
}
