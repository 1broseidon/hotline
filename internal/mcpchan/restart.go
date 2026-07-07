package mcpchan

import (
	"github.com/1broseidon/hotline/internal/supervise"
)

// restartSchema is the verbatim InputSchema for the restart tool.
const restartSchema = `{"type":"object","properties":{"reason":{"type":"string","description":"Why the restart was requested — one short line, recorded in the supervisor log."}},"required":["reason"]}`

// RestartInput is the decoded argument set for the restart tool.
type RestartInput struct {
	Reason string `json:"reason"`
}

// handleRestart implements the restart tool: it writes the restart.request
// control file that the supervisor (`hotline up`) polls. The tool carries no
// argument that changes WHAT runs — argv, env, and cwd were fixed by the
// operator at `up` time; the reason is only ever logged. Worst case a
// prompt-injected "restart yourself" bounces the session: in-flight context
// is lost, but the transcript, schedules, and access state persist. That is
// a smaller blast radius than tools the channel already exposes, so the tool
// rides the same pairing gate (and, on the non-yolo path, Claude's
// permission gate) as everything else.
func handleRestart(in RestartInput, supervisorDir string) (string, bool) {
	if err := supervise.RequestRestart(supervisorDir, in.Reason); err != nil {
		return "restart failed: " + err.Error(), true
	}
	return "Restart requested. The supervisor will relaunch this session within a few seconds — anything you send after this tool call may not be delivered, so if you owe the user a goodbye, it should already have been sent.", false
}
