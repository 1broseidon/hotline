package main

// Model catalog amendment 2026-07-20 — the box half of "one knob, one list".
//
// The extension cannot read pi's resolved cycling list (AgentSession.scopedModels
// is not on the ExtensionContext/ExtensionAPI surface), so the box is the only
// place the launch line, Ctrl+P, and the app can be made to agree. That makes
// two behaviors load-bearing and worth pinning:
//
//   1. HOTLINE_PI_MODELS reaches pi as --models, with the same passthrough-wins
//      rule as the other knobs.
//   2. The EFFECTIVE scope — whichever side won — is re-exported into pi's env,
//      so the extension resolves the list that is actually on the launch line
//      and never advertises one Ctrl+P does not cycle.

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/config"
)

// TestPiModelArgsModels: the scope knob becomes --models, and an explicit
// passthrough --models suppresses it exactly like the other three flags.
func TestPiModelArgsModels(t *testing.T) {
	tests := []struct {
		name        string
		knob        config.PiModel
		passthrough []string
		want        []string
	}{
		{
			name: "scope only",
			knob: config.PiModel{Models: "openai-codex/*,zai/glm-5.2"},
			want: []string{"--models", "openai-codex/*,zai/glm-5.2"},
		},
		{
			name: "scope rides after the frozen three",
			knob: config.PiModel{Model: "sonnet", Thinking: "high", Models: "anthropic/*"},
			want: []string{"--model", "sonnet", "--thinking", "high", "--models", "anthropic/*"},
		},
		{
			name:        "passthrough --models wins (space form)",
			knob:        config.PiModel{Model: "sonnet", Models: "anthropic/*"},
			passthrough: []string{"--models", "zai/*"},
			want:        []string{"--model", "sonnet"},
		},
		{
			name:        "passthrough --models wins (equals form)",
			knob:        config.PiModel{Models: "anthropic/*"},
			passthrough: []string{"--models=zai/*"},
			want:        nil,
		},
		{
			name:        "--model is NOT --models (prefix must not collide)",
			knob:        config.PiModel{Models: "anthropic/*"},
			passthrough: []string{"--model", "sonnet"},
			want:        []string{"--models", "anthropic/*"},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := piModelArgs(tc.knob, tc.passthrough)
			if !reflect.DeepEqual(got, tc.want) {
				t.Errorf("piModelArgs(%+v, %v) = %v, want %v", tc.knob, tc.passthrough, got, tc.want)
			}
		})
	}
}

// TestPassthroughFlagValue: both spellings pi's parser accepts, last-wins, and
// the valueless trailing flag.
func TestPassthroughFlagValue(t *testing.T) {
	tests := []struct {
		name  string
		args  []string
		want  string
		found bool
	}{
		{name: "absent", args: []string{"--verbose"}, want: "", found: false},
		{name: "space form", args: []string{"--models", "a,b"}, want: "a,b", found: true},
		{name: "equals form", args: []string{"--models=a,b"}, want: "a,b", found: true},
		{name: "last wins, like pi", args: []string{"--models", "a", "--models=b"}, want: "b", found: true},
		{name: "valueless trailing flag scopes nothing", args: []string{"--models"}, want: "", found: true},
		{name: "--model must not match --models", args: []string{"--model", "sonnet"}, want: "", found: false},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, found := passthroughFlagValue(tc.args, "--models")
			if got != tc.want || found != tc.found {
				t.Errorf("passthroughFlagValue(%v) = (%q,%v), want (%q,%v)", tc.args, got, found, tc.want, tc.found)
			}
		})
	}
}

// TestEffectivePiModels: the re-exported value is whichever side WON, never the
// knob that piModelArgs just dropped. This is the bug the whole helper exists to
// prevent — exporting an inert knob would have the app advertise a filtered list
// that Ctrl+P does not cycle.
func TestEffectivePiModels(t *testing.T) {
	tests := []struct {
		name        string
		knob        config.PiModel
		passthrough []string
		want        string
	}{
		{name: "neither side scopes", want: ""},
		{name: "knob only", knob: config.PiModel{Models: "anthropic/*"}, want: "anthropic/*"},
		{
			name:        "passthrough won, so the passthrough is exported",
			knob:        config.PiModel{Models: "anthropic/*"},
			passthrough: []string{"--models", "zai/*,openai-codex/*"},
			want:        "zai/*,openai-codex/*",
		},
		{
			name:        "a passthrough scope is normalized like the knob",
			passthrough: []string{"--models", " a , , b "},
			want:        "a,b",
		},
		{
			name:        "a valueless passthrough --models scopes nothing",
			knob:        config.PiModel{Models: "anthropic/*"},
			passthrough: []string{"--models"},
			want:        "",
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := effectivePiModels(tc.knob, tc.passthrough); got != tc.want {
				t.Errorf("effectivePiModels(%+v, %v) = %q, want %q", tc.knob, tc.passthrough, got, tc.want)
			}
		})
	}
}

// piModelsEnv reads the HOTLINE_PI_MODELS the box exported to the pi child.
// found=false means the var is absent, which is how "no scope" is expressed —
// distinct from an exported empty value, which would make the extension parse
// an empty scope rather than fall through to pi's own precedence.
func piModelsEnv(env []string) (string, bool) {
	value, found := "", false
	for _, e := range env {
		if v, ok := strings.CutPrefix(e, "HOTLINE_PI_MODELS="); ok {
			value, found = v, true
		}
	}
	return value, found
}

// TestUpExportsPiModelsKnob: the scope reaches pi BOTH ways — as --models on the
// launch line (so Ctrl+P cycles it) and as HOTLINE_PI_MODELS in the child env
// (so the extension resolves the same list for the catalog). One knob, two
// surfaces, and they carry identical bytes.
func TestUpExportsPiModelsKnob(t *testing.T) {
	dir := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_MODELS"} {
		unsetEnv(t, k)
	}
	// Deliberately sloppy spacing: the box normalizes at both ends, so what pi
	// gets and what the extension reads can never drift over whitespace.
	content := "HOTLINE_PI_MODELS=openai-codex/* , zai/glm-5.2 ,\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "pi")

	got := runUpCapturePi(t, nil)
	want := []string{"--mode", "rpc", "--session-id", "hotline", "--models", "openai-codex/*,zai/glm-5.2"}
	if !reflect.DeepEqual(got.argv[1:], want) {
		t.Fatalf("argv[1:] = %v, want %v", got.argv[1:], want)
	}
	env, found := piModelsEnv(got.env)
	if !found {
		t.Fatal("HOTLINE_PI_MODELS was not exported to the pi child")
	}
	if env != "openai-codex/*,zai/glm-5.2" {
		t.Errorf("exported HOTLINE_PI_MODELS = %q, want the normalized knob", env)
	}
	if env != got.argv[len(got.argv)-1] {
		t.Errorf("exported %q but launched with --models %q; the two surfaces must be identical", env, got.argv[len(got.argv)-1])
	}
}

// TestUpExportsPiModelsPassthroughWins: launched with an explicit
// `-- --models …`, the knob is dropped from the launch line AND the exported
// value follows the passthrough. Exporting the inert knob here would make the
// app's rows a list Ctrl+P never cycles.
func TestUpExportsPiModelsPassthroughWins(t *testing.T) {
	dir := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_MODELS"} {
		unsetEnv(t, k)
	}
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("HOTLINE_PI_MODELS=anthropic/*\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "pi")

	got := runUpCapturePi(t, []string{"--models", "zai/*"})
	want := []string{"--mode", "rpc", "--session-id", "hotline", "--models", "zai/*"}
	if !reflect.DeepEqual(got.argv[1:], want) {
		t.Fatalf("argv[1:] = %v, want %v (the knob must not double-apply)", got.argv[1:], want)
	}
	env, found := piModelsEnv(got.env)
	if !found || env != "zai/*" {
		t.Errorf("exported HOTLINE_PI_MODELS = %q (found=%v), want the winning passthrough %q", env, found, "zai/*")
	}
}

// TestUpNoPiModelsExportsNothing: with no scope on either side the var is
// ABSENT, not empty. Absence is what lets the extension fall through to pi's own
// precedence (its enabledModels setting, then every model with auth) instead of
// resolving an empty scope.
func TestUpNoPiModelsExportsNothing(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_MODELS"} {
		unsetEnv(t, k)
	}
	stubBinary(t, "pi")

	got := runUpCapturePi(t, nil)
	for _, a := range got.argv {
		if a == "--models" {
			t.Fatalf("argv carries --models with no scope configured: %v", got.argv)
		}
	}
	if v, found := piModelsEnv(got.env); found {
		t.Errorf("HOTLINE_PI_MODELS exported as %q with no scope configured; it must be absent", v)
	}
}

// TestUpScrubsInheritedPiModels: a stale HOTLINE_PI_MODELS in the operator's
// shell must never outrank the resolved scope. It is scrubbed and replaced, so
// the child's env always reflects the launch line the box just built.
func TestUpScrubsInheritedPiModels(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING"} {
		unsetEnv(t, k)
	}
	// A real-env knob wins over .env by mergedEnv, so this is BOTH the inherited
	// value and the resolved one: the assertion is that it appears exactly once.
	t.Setenv("HOTLINE_PI_MODELS", "shell/*")
	stubBinary(t, "pi")

	got := runUpCapturePi(t, nil)
	count := 0
	for _, e := range got.env {
		if strings.HasPrefix(e, "HOTLINE_PI_MODELS=") {
			count++
		}
	}
	if count != 1 {
		t.Errorf("HOTLINE_PI_MODELS appears %d times in the child env, want exactly 1: a duplicate makes the effective value ambiguous", count)
	}
	if v, _ := piModelsEnv(got.env); v != "shell/*" {
		t.Errorf("exported %q, want the resolved %q", v, "shell/*")
	}
}
