package config

// Per-box knob resolution (sol review #10). The app presents model/effort as a
// per-box setting, but reads and writes both landed on StateRoot()/.env — so a
// named box's model change silently retargeted every other box on the machine
// at its next respawn.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// sandbox points StateRoot at a temp dir and clears the real-env knobs, which
// otherwise win over both .env tiers.
func sandbox(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", root)
	for _, k := range []string{
		"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS",
		"HOTLINE_SDK_SETTING_SOURCES", "HOTLINE_CLAUDE_SDK_MODEL",
		"HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODELS",
	} {
		os.Unsetenv(k)
	}
	return root
}

func TestPerBoxSDKKnobsDoNotLeakBetweenBoxes(t *testing.T) {
	base := sandbox(t)
	boxA := filepath.Join(base, "bots", "boxa")
	boxB := filepath.Join(base, "bots", "boxb")
	for _, d := range []string{boxA, boxB} {
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	// A machine-wide default plus a shared credential.
	if err := os.WriteFile(filepath.Join(base, ".env"),
		[]byte("TELEGRAM_BOT_TOKEN=shared-secret\nHOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	// Box A changes its model, the way a hot apply does.
	sonnet := "claude-sonnet-4-6"
	if err := UpdateSDKEnvForBox(boxA, &sonnet, nil); err != nil {
		t.Fatal(err)
	}

	got, err := LoadSDKForBox(boxA)
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "claude-sonnet-4-6" {
		t.Fatalf("box A model = %q, want its own change", got.Model)
	}

	// Box B must be untouched — this is the whole finding.
	got, err = LoadSDKForBox(boxB)
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "claude-opus-4-8" {
		t.Fatalf("box B model = %q — box A's change moved another box's model", got.Model)
	}

	// The shared credential stays in the base file and is still readable from
	// every box; it was never a per-box thing.
	baseRaw, err := os.ReadFile(filepath.Join(base, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(baseRaw), "TELEGRAM_BOT_TOKEN=shared-secret") {
		t.Fatalf("the shared credential file was disturbed:\n%s", baseRaw)
	}
	if strings.Contains(string(baseRaw), "claude-sonnet-4-6") {
		t.Fatalf("a per-box change was written to the machine-wide file:\n%s", baseRaw)
	}
	boxRaw, err := os.ReadFile(filepath.Join(boxA, ".env"))
	if err != nil {
		t.Fatalf("box A got no .env of its own: %v", err)
	}
	if !strings.Contains(string(boxRaw), "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Fatalf("box A's .env:\n%s", boxRaw)
	}
}

func TestPerBoxPiKnobsDoNotLeakBetweenBoxes(t *testing.T) {
	base := sandbox(t)
	boxA := filepath.Join(base, "bots", "boxa")
	boxB := filepath.Join(base, "bots", "boxb")
	for _, d := range []string{boxA, boxB} {
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(base, ".env"),
		[]byte("HOTLINE_PI_MODEL=openai-codex/gpt-5.5\nHOTLINE_PI_THINKING=medium\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	sol := "openai-codex/gpt-5.6-sol"
	xhigh := "xhigh"
	if err := UpdatePiEnvForBox(boxA, &sol, &xhigh); err != nil {
		t.Fatal(err)
	}

	a, err := LoadPiModelForBox(boxA)
	if err != nil {
		t.Fatal(err)
	}
	if a.Model != sol || a.Thinking != "xhigh" {
		t.Fatalf("box A = %+v", a)
	}
	b, err := LoadPiModelForBox(boxB)
	if err != nil {
		t.Fatal(err)
	}
	if b.Model != "openai-codex/gpt-5.5" || b.Thinking != "medium" {
		t.Fatalf("box B = %+v — box A's change moved another box's pi knobs", b)
	}
}

// TestDefaultBoxIsByteIdenticalToTheOldBehaviour: the uninstanced box's root IS
// the base, so nothing about it changes. This is the compatibility guarantee.
func TestDefaultBoxResolvesAgainstTheBaseExactlyAsBefore(t *testing.T) {
	base := sandbox(t)
	if err := os.WriteFile(filepath.Join(base, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	// Both spellings of "the default box".
	for _, root := range []string{"", base} {
		got, err := LoadSDKForBox(root)
		if err != nil {
			t.Fatal(err)
		}
		if got.Model != "claude-opus-4-8" {
			t.Fatalf("root %q: model = %q", root, got.Model)
		}
	}

	sonnet := "claude-sonnet-4-6"
	if err := UpdateSDKEnvForBox("", &sonnet, nil); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(base, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Fatalf("the default box did not write the base .env:\n%s", raw)
	}
	if _, err := os.Stat(filepath.Join(base, "bots")); !os.IsNotExist(err) {
		t.Fatal("the default box invented a per-box directory")
	}
	// And LoadSDK (the no-arg form every non-box-aware caller still uses)
	// agrees with it.
	got, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "claude-sonnet-4-6" {
		t.Fatalf("LoadSDK diverged from LoadSDKForBox(\"\"): %q", got.Model)
	}
}

// TestBoxKnobsFallBackToTheMachineDefault: a box that has never had a knob set
// still inherits the operator's machine-wide default. Per-box must mean
// "overridable", not "isolated from the base".
func TestBoxKnobsFallBackToTheMachineDefault(t *testing.T) {
	base := sandbox(t)
	box := filepath.Join(base, "bots", "fresh")
	if err := os.MkdirAll(box, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(base, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// Only the model is overridden for this box.
	sonnet := "claude-sonnet-4-6"
	if err := UpdateSDKEnvForBox(box, &sonnet, nil); err != nil {
		t.Fatal(err)
	}
	got, err := LoadSDKForBox(box)
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "claude-sonnet-4-6" {
		t.Fatalf("model = %q, want the box override", got.Model)
	}
	if got.Effort != "xhigh" {
		t.Fatalf("effort = %q, want the machine default to still show through", got.Effort)
	}
}

// TestRealEnvStillWins: the real environment beats both tiers, as everywhere
// else in hotline. This is the documented reason a hot apply can look like it
// did nothing on a box launched from a shell that exported the knob.
func TestRealEnvStillWinsOverBothTiers(t *testing.T) {
	base := sandbox(t)
	box := filepath.Join(base, "bots", "boxa")
	if err := os.MkdirAll(box, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(base, ".env"), []byte("HOTLINE_SDK_MODEL=from-base\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(box, ".env"), []byte("HOTLINE_SDK_MODEL=from-box\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := LoadSDKForBox(box)
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "from-box" {
		t.Fatalf("box tier did not beat base: %q", got.Model)
	}
	t.Setenv("HOTLINE_SDK_MODEL", "from-real-env")
	got, err = LoadSDKForBox(box)
	if err != nil {
		t.Fatal(err)
	}
	if got.Model != "from-real-env" {
		t.Fatalf("real env did not win: %q", got.Model)
	}
}
