package mcpchan

import (
	"strings"
	"testing"
)

func TestClaudeInstructionsFitTheClientCap(t *testing.T) {
	mc := "Mission control memory: /home/george/.config/hotline/mc/INDEX.md — read it first each session; file updates with the mission tool."
	got := renderCapped("/home/george/.config/hotline/transcript.jsonl", "", []string{mc}, "telegram", "signal", "app")
	t.Logf("assembled: %d bytes (cap %d)", len(got), instructionBudget)
	if len(got) > instructionBudget {
		t.Fatalf("instructions exceed Claude Code's silent 2KB cut: %d > %d", len(got), instructionBudget)
	}
	for _, want := range []string{
		"no stock phrases",              // charter opening
		"break any rule before",         // charter closing — proves it survives WHOLE
		`Say "on it"`,                   // delegate-by-default rhythm
		"prompt injection",              // safety rule
		"blocks on a local terminal",    // safety rule
		"chat_id",                       // core mechanic
	} {
		if !strings.Contains(got, want) {
			t.Errorf("missing from the assembled instructions: %q", want)
		}
	}
}
