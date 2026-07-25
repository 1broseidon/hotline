package config

// Model catalog amendment 2026-07-20 — the HOTLINE_PI_MODELS scope knob.
//
// ParsePiModels is mirrored byte-for-byte by the harness's own parsePiModels
// (harness/pi/src/modelcatalog.ts), because both ends read the same string: the
// box to build --models, the extension to resolve the catalog. Anywhere the two
// disagree, the app would show a list Ctrl+P does not cycle.

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestParsePiModels(t *testing.T) {
	tests := []struct {
		name string
		in   string
		want []string
	}{
		{name: "unset", in: "", want: nil},
		{name: "single", in: "anthropic/*", want: []string{"anthropic/*"}},
		{name: "comma split with spaces", in: "a, b ,c", want: []string{"a", "b", "c"}},
		{name: "blanks dropped", in: "a,,b,", want: []string{"a", "b"}},
		{name: "only blanks is nothing", in: " , , ", want: nil},
		{
			name: "globs and :level suffixes survive verbatim",
			in:   "anthropic/*,*sonnet*,glm-5.2:high",
			want: []string{"anthropic/*", "*sonnet*", "glm-5.2:high"},
		},
		{
			// Dropped, never truncated: a truncated glob would silently match a
			// DIFFERENT set of models, which is worse than matching none.
			name: "an over-long pattern is dropped, not truncated",
			in:   "ok," + strings.Repeat("x", piModelsMaxPatternLen+1),
			want: []string{"ok"},
		},
		{
			name: "a pattern exactly at the length cap survives",
			in:   strings.Repeat("x", piModelsMaxPatternLen),
			want: []string{strings.Repeat("x", piModelsMaxPatternLen)},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := ParsePiModels(tc.in); !reflect.DeepEqual(got, tc.want) {
				t.Errorf("ParsePiModels(%q) = %v, want %v", tc.in, got, tc.want)
			}
		})
	}
}

func TestParsePiModelsCountCap(t *testing.T) {
	var parts []string
	for i := 0; i < piModelsMaxPatterns+8; i++ {
		parts = append(parts, "m")
	}
	got := ParsePiModels(strings.Join(parts, ","))
	if len(got) != piModelsMaxPatterns {
		t.Errorf("ParsePiModels capped at %d, want %d", len(got), piModelsMaxPatterns)
	}
}

// TestNormalizePiModelsIdempotent: the box normalizes when it loads the knob and
// again when it exports it, so a sloppy .env cannot make the launch line and the
// harness env differ.
func TestNormalizePiModelsIdempotent(t *testing.T) {
	for _, in := range []string{"", "a", " a , b ", "a,,b,", "anthropic/*, *sonnet* "} {
		once := NormalizePiModels(in)
		if twice := NormalizePiModels(once); twice != once {
			t.Errorf("NormalizePiModels not idempotent for %q: %q then %q", in, once, twice)
		}
	}
	if got := NormalizePiModels(" openai-codex/* , zai/glm-5.2 ,"); got != "openai-codex/*,zai/glm-5.2" {
		t.Errorf("NormalizePiModels = %q, want the canonical single-line form", got)
	}
}

// TestLoadPiModelModels: the knob round-trips out of the shared state-dir .env
// normalized, next to the existing three, and the real environment still wins.
func TestLoadPiModelModels(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_MODELS"} {
		t.Setenv(k, "")
		os.Unsetenv(k)
	}
	content := "HOTLINE_PI_MODEL=openai-codex/gpt-5.6-luna\nHOTLINE_PI_THINKING=high\nHOTLINE_PI_MODELS=openai-codex/* , zai/glm-5.2\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	knob, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if knob.Models != "openai-codex/*,zai/glm-5.2" {
		t.Errorf("Models = %q, want the normalized form", knob.Models)
	}
	// The scope knob must not disturb the existing three.
	if knob.Model != "openai-codex/gpt-5.6-luna" || knob.Thinking != "high" {
		t.Errorf("model/thinking knobs = %q/%q, want them untouched", knob.Model, knob.Thinking)
	}

	// Real env wins, same as every other knob in this family.
	t.Setenv("HOTLINE_PI_MODELS", "anthropic/*")
	knob, err = LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if knob.Models != "anthropic/*" {
		t.Errorf("Models = %q, want the real-env value to win", knob.Models)
	}
}

// TestLoadPiModelNoModels: an absent knob is empty, which is how "no scope" is
// expressed all the way down — the box exports nothing and the harness falls
// through to pi's own precedence.
func TestLoadPiModelNoModels(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_MODELS"} {
		t.Setenv(k, "")
		os.Unsetenv(k)
	}
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("HOTLINE_PI_MODEL=sonnet\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	knob, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if knob.Models != "" {
		t.Errorf("Models = %q, want empty", knob.Models)
	}
}

// TestUpdatePiEnvLeavesModelsAlone: a hot model apply from the app must never
// touch the operator's scope. The scope is the MENU (operator-owned, set on the
// box); the model is the SELECTION (app-owned). A model write that rewrote the
// scope would silently re-cut Ctrl+P behind the operator's back.
func TestUpdatePiEnvLeavesModelsAlone(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("HOTLINE_PI_MODELS=openai-codex/*,zai/glm-5.2\nHOTLINE_PI_MODEL=zai/glm-5.2\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	model := "openai-codex/gpt-5.6-sol"
	if err := UpdatePiEnv(&model, nil); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	got := string(raw)
	if !strings.Contains(got, "HOTLINE_PI_MODELS=openai-codex/*,zai/glm-5.2") {
		t.Errorf(".env lost or rewrote the scope knob after a model apply:\n%s", got)
	}
	if !strings.Contains(got, "HOTLINE_PI_MODEL=openai-codex/gpt-5.6-sol") {
		t.Errorf(".env did not take the new model:\n%s", got)
	}

	// And the reloaded knob agrees, so the next respawn rebuilds both flags.
	for _, k := range []string{"HOTLINE_PI_MODEL", "HOTLINE_PI_MODELS", "HOTLINE_PI_PROVIDER", "HOTLINE_PI_THINKING"} {
		t.Setenv(k, "")
		os.Unsetenv(k)
	}
	knob, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if knob.Models != "openai-codex/*,zai/glm-5.2" || knob.Model != "openai-codex/gpt-5.6-sol" {
		t.Errorf("reloaded knob = %+v, want the scope preserved and the model updated", knob)
	}
}
