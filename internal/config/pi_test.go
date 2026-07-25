package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// withPiState points the resolver at a fresh state dir and clears the pi knob
// env for the test.
func withPiState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING"} {
		t.Setenv(k, "")
		os.Unsetenv(k)
	}
	return dir
}

func TestLoadPiModelUnset(t *testing.T) {
	withPiState(t)
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km != (PiModel{}) {
		t.Fatalf("want zero PiModel with no env/.env, got %+v", km)
	}
}

func TestLoadPiModelFromDotEnv(t *testing.T) {
	dir := withPiState(t)
	content := "HOTLINE_PI_PROVIDER=anthropic\nHOTLINE_PI_MODEL=sonnet:high\nHOTLINE_PI_THINKING=high\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	want := PiModel{Provider: "anthropic", Model: "sonnet:high", Thinking: "high"}
	if km != want {
		t.Fatalf("LoadPiModel from .env = %+v, want %+v", km, want)
	}
}

func TestLoadPiModelRealEnvWins(t *testing.T) {
	dir := withPiState(t)
	content := "HOTLINE_PI_PROVIDER=anthropic\nHOTLINE_PI_MODEL=sonnet\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	// Real environment overrides the .env value, matching hotline's convention.
	t.Setenv("HOTLINE_PI_PROVIDER", "openai")
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km.Provider != "openai" {
		t.Errorf("Provider = %q, want openai (real env wins)", km.Provider)
	}
	if km.Model != "sonnet" {
		t.Errorf("Model = %q, want sonnet (from .env, no real-env override)", km.Model)
	}
	if km.Thinking != "" {
		t.Errorf("Thinking = %q, want empty", km.Thinking)
	}
}

// pi hot-apply amendment 2026-07-20 — the remote-knob half of the pi family.

func TestValidPiThinking(t *testing.T) {
	for _, ok := range []string{"off", "minimal", "low", "medium", "high", "xhigh", "max"} {
		if !ValidPiThinking(ok) {
			t.Errorf("ValidPiThinking(%q) = false, want true", ok)
		}
	}
	// Pi's setThinkingLevel takes a LEVEL, never a token budget: the raw
	// integer form ValidSDKEffort accepts must be refused here, or the box
	// would forward a value the harness can only fail on.
	for _, bad := range []string{"32000", "0", "-5", "HIGH", "", "xtreme", "1"} {
		if ValidPiThinking(bad) {
			t.Errorf("ValidPiThinking(%q) = true, want false", bad)
		}
		if bad == "32000" && !ValidSDKEffort(bad) {
			t.Error("guard: ValidSDKEffort should still accept a raw token budget")
		}
	}
}

func TestUpdatePiEnvSetAndReload(t *testing.T) {
	dir := withPiState(t)
	if err := os.WriteFile(filepath.Join(dir, ".env"),
		[]byte("# knobs\nHOTLINE_MC_CONTEXT_CAP=300000\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	model, thinking := "openai-codex/gpt-5.6-sol", "high"
	if err := UpdatePiEnv(&model, &thinking); err != nil {
		t.Fatal(err)
	}
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km.Model != model || km.Thinking != thinking {
		t.Fatalf("reloaded %+v, want model=%q thinking=%q", km, model, thinking)
	}
	raw, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_MC_CONTEXT_CAP=300000") {
		t.Errorf("unrelated key lost:\n%s", raw)
	}
	info, err := os.Stat(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Errorf(".env mode = %v, want 0600", info.Mode().Perm())
	}
}

// A model write retires HOTLINE_PI_PROVIDER: the persisted id carries its own
// provider, and a leftover explicit --provider fights that prefix when
// piModelArgs rebuilds the argv at respawn.
func TestUpdatePiEnvModelWriteDropsProvider(t *testing.T) {
	dir := withPiState(t)
	if err := os.WriteFile(filepath.Join(dir, ".env"),
		[]byte("HOTLINE_PI_PROVIDER=zai\nHOTLINE_PI_MODEL=glm-5.2\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	model := "openai-codex/gpt-5.6-sol"
	if err := UpdatePiEnv(&model, nil); err != nil {
		t.Fatal(err)
	}
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km.Provider != "" {
		t.Errorf("provider = %q, want it retired by the canonical model write", km.Provider)
	}
	if km.Model != model {
		t.Errorf("model = %q, want %q", km.Model, model)
	}
}

func TestUpdatePiEnvNilLeavesKnobUntouched(t *testing.T) {
	dir := withPiState(t)
	if err := os.WriteFile(filepath.Join(dir, ".env"),
		[]byte("HOTLINE_PI_MODEL=zai/glm-5.2\nHOTLINE_PI_THINKING=low\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	thinking := "xhigh"
	if err := UpdatePiEnv(nil, &thinking); err != nil {
		t.Fatal(err)
	}
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km.Model != "zai/glm-5.2" {
		t.Errorf("a thinking-only write disturbed the model: %q", km.Model)
	}
	if km.Thinking != "xhigh" {
		t.Errorf("thinking = %q, want xhigh", km.Thinking)
	}
}

// A clear REMOVES the line rather than writing an empty one: a set-but-empty
// key is still a real value to piModelArgs and would emit `--model ""`.
func TestUpdatePiEnvClearRemovesLines(t *testing.T) {
	dir := withPiState(t)
	if err := os.WriteFile(filepath.Join(dir, ".env"),
		[]byte("HOTLINE_PI_PROVIDER=zai\nHOTLINE_PI_MODEL=zai/glm-5.2\nHOTLINE_PI_THINKING=low\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	empty := ""
	if err := UpdatePiEnv(&empty, &empty); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	for _, k := range []string{"HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_PROVIDER"} {
		if strings.Contains(string(raw), k) {
			t.Errorf("%s survived a clear:\n%s", k, raw)
		}
	}
	km, err := LoadPiModel()
	if err != nil {
		t.Fatal(err)
	}
	if km != (PiModel{}) {
		t.Fatalf("cleared knob still loads as %+v", km)
	}
}

func TestUpdatePiEnvCreatesMissingFile(t *testing.T) {
	dir := withPiState(t)
	model := "zai/glm-5.2"
	if err := UpdatePiEnv(&model, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, ".env")); err != nil {
		t.Fatalf(".env not created: %v", err)
	}
}
