package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// sdkTestState points the state dir at a temp root and truly unsets every
// SDK knob (t.Setenv registers the restore; a set-but-empty real env would
// win over the .env in mergedEnv).
func sdkTestState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	for _, k := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_SDK_SETTING_SOURCES", "HOTLINE_CLAUDE_SDK_MODEL"} {
		t.Setenv(k, "")
		if err := os.Unsetenv(k); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func writeSDKEnv(t *testing.T, dir, body string) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestLoadSDKDefaultsEmpty(t *testing.T) {
	sdkTestState(t)
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "" || cfg.Effort != "" || cfg.MaxTurns != 0 || cfg.SettingSources != "" {
		t.Errorf("cfg = %+v, want zero values", cfg)
	}
}

func TestLoadSDKSettingSources(t *testing.T) {
	sdkTestState(t)
	t.Setenv("HOTLINE_SDK_SETTING_SOURCES", "Project, USER ,project") // case-insensitive, dedup, order preserved
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.SettingSources != "project,user" {
		t.Errorf("SettingSources = %q, want %q", cfg.SettingSources, "project,user")
	}
}

func TestLoadSDKInvalidSettingSourcesLoud(t *testing.T) {
	for _, bad := range []string{"projekt", "project,global", "managed"} {
		sdkTestState(t)
		t.Setenv("HOTLINE_SDK_SETTING_SOURCES", bad)
		_, err := LoadSDK()
		if err == nil {
			t.Errorf("LoadSDK() with HOTLINE_SDK_SETTING_SOURCES=%q: want error, got nil", bad)
			continue
		}
		if !strings.Contains(err.Error(), "HOTLINE_SDK_SETTING_SOURCES") {
			t.Errorf("error %q does not name the env var", err)
		}
	}
}

func TestParseSDKSettingSources(t *testing.T) {
	cases := map[string]string{"": "", "  ": "", " , ,": "", "local": "local", "user,project,local": "user,project,local"}
	for in, want := range cases {
		got, err := parseSDKSettingSources(in)
		if err != nil {
			t.Errorf("parseSDKSettingSources(%q) errored: %v", in, err)
			continue
		}
		if got != want {
			t.Errorf("parseSDKSettingSources(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestLoadSDKFromDotEnv(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\nHOTLINE_SDK_MAX_TURNS=40\n")
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-opus-4-8" || cfg.Effort != "xhigh" || cfg.MaxTurns != 40 {
		t.Errorf("cfg = %+v", cfg)
	}
}

func TestLoadSDKRealEnvWinsOverDotEnv(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_SDK_MODEL=claude-sonnet-4-6\nHOTLINE_SDK_EFFORT=low\n")
	t.Setenv("HOTLINE_SDK_MODEL", "claude-opus-4-8")
	t.Setenv("HOTLINE_SDK_EFFORT", "MAX") // case-insensitive names
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-opus-4-8" {
		t.Errorf("Model = %q, want real-env value", cfg.Model)
	}
	if cfg.Effort != "max" {
		t.Errorf("Effort = %q, want normalized max", cfg.Effort)
	}
}

func TestLoadSDKLegacyModelFallback(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_CLAUDE_SDK_MODEL=claude-opus-4-8\n")
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-opus-4-8" {
		t.Errorf("Model = %q, want legacy fallback honored", cfg.Model)
	}

	// The canonical name wins over the legacy one when both are set.
	t.Setenv("HOTLINE_SDK_MODEL", "claude-fable-5")
	cfg, err = LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-fable-5" {
		t.Errorf("Model = %q, want canonical over legacy", cfg.Model)
	}
}

func TestLoadSDKNumericEffort(t *testing.T) {
	sdkTestState(t)
	t.Setenv("HOTLINE_SDK_EFFORT", "12000")
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Effort != "12000" {
		t.Errorf("Effort = %q", cfg.Effort)
	}
}

func TestLoadSDKInvalidEffortLoud(t *testing.T) {
	sdkTestState(t)
	for _, bad := range []string{"xtreme", "-5", "0", "4k"} {
		t.Setenv("HOTLINE_SDK_EFFORT", bad)
		_, err := LoadSDK()
		if err == nil || !strings.Contains(err.Error(), "HOTLINE_SDK_EFFORT") {
			t.Errorf("effort %q: err = %v, want loud error naming the env var", bad, err)
		}
	}
}

func TestLoadSDKInvalidMaxTurnsLoud(t *testing.T) {
	sdkTestState(t)
	for _, bad := range []string{"many", "-1", "0", "1.5"} {
		t.Setenv("HOTLINE_SDK_MAX_TURNS", bad)
		_, err := LoadSDK()
		if err == nil || !strings.Contains(err.Error(), "HOTLINE_SDK_MAX_TURNS") {
			t.Errorf("maxTurns %q: err = %v, want loud error naming the env var", bad, err)
		}
	}
}

func TestValidSDKEffort(t *testing.T) {
	for _, good := range []string{"low", "medium", "high", "xhigh", "max", "1", "12000"} {
		if !ValidSDKEffort(good) {
			t.Errorf("ValidSDKEffort(%q) = false, want true", good)
		}
	}
	for _, bad := range []string{"", "xtreme", "HIGH", "-5", "0", "4k", "1.5", " high"} {
		if ValidSDKEffort(bad) {
			t.Errorf("ValidSDKEffort(%q) = true, want false", bad)
		}
	}
}

func strPtr(s string) *string { return &s }

// TestUpdateSDKEnvSetAndReload: UpdateSDKEnv writes into the exact file
// LoadSDK reads, so a set round-trips through the production read path.
func TestUpdateSDKEnvSetAndReload(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "# knobs\nHOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_MAX_TURNS=40\n")

	if err := UpdateSDKEnv(strPtr("claude-sonnet-4-6"), strPtr("high")); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-sonnet-4-6" || cfg.Effort != "high" || cfg.MaxTurns != 40 {
		t.Errorf("cfg = %+v, want updated model/effort with MaxTurns untouched", cfg)
	}
	raw, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "# knobs\n") {
		t.Errorf("comment not preserved:\n%s", raw)
	}
}

// TestUpdateSDKEnvNilLeavesKnobUntouched: a nil pointer means "leave that
// knob alone" — only the non-nil field's line changes.
func TestUpdateSDKEnvNilLeavesKnobUntouched(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\n")

	if err := UpdateSDKEnv(nil, strPtr("low")); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-opus-4-8" || cfg.Effort != "low" {
		t.Errorf("cfg = %+v, want model untouched + effort low", cfg)
	}

	if err := UpdateSDKEnv(nil, nil); err != nil {
		t.Fatal(err)
	}
}

// TestUpdateSDKEnvClearRemovesLinesAndLegacy: clearing (pointer to "")
// REMOVES the knob's line rather than writing KEY=, and any model write —
// set or clear — also removes the deprecated HOTLINE_CLAUDE_SDK_MODEL line,
// which would otherwise resurrect an old model through LoadSDK's fallback.
func TestUpdateSDKEnvClearRemovesLinesAndLegacy(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_CLAUDE_SDK_MODEL=claude-legacy\nHOTLINE_SDK_EFFORT=xhigh\nOTHER=kept\n")

	if err := UpdateSDKEnv(strPtr(""), strPtr("")); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	for _, gone := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_CLAUDE_SDK_MODEL", "HOTLINE_SDK_EFFORT"} {
		if strings.Contains(string(raw), gone) {
			t.Errorf("%s line survived a clear:\n%s", gone, raw)
		}
	}
	if !strings.Contains(string(raw), "OTHER=kept") {
		t.Errorf("unrelated key lost:\n%s", raw)
	}
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "" || cfg.Effort != "" {
		t.Errorf("cfg = %+v, want cleared to SDK defaults", cfg)
	}
}

// TestUpdateSDKEnvSetModelDropsLegacy: setting a model (not just clearing)
// also drops the legacy line, so a later clear can't resurrect it.
func TestUpdateSDKEnvSetModelDropsLegacy(t *testing.T) {
	dir := sdkTestState(t)
	writeSDKEnv(t, dir, "HOTLINE_CLAUDE_SDK_MODEL=claude-legacy\n")

	if err := UpdateSDKEnv(strPtr("claude-fable-5"), nil); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "HOTLINE_CLAUDE_SDK_MODEL") {
		t.Errorf("legacy line survived a model set:\n%s", raw)
	}
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Model != "claude-fable-5" {
		t.Errorf("Model = %q", cfg.Model)
	}
}

// TestUpdateSDKEnvCreatesMissingFile: a box that never had an .env (all
// knobs from real env) still persists — the state dir and file are created.
func TestUpdateSDKEnvCreatesMissingFile(t *testing.T) {
	dir := sdkTestState(t)
	sub := filepath.Join(dir, "deeper")
	t.Setenv("HOTLINE_STATE_DIR", sub) // not yet created

	if err := UpdateSDKEnv(strPtr("claude-sonnet-4-6"), nil); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(sub, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Errorf("content = %q", raw)
	}
}

// TestIntegerKnobParityWithTheHarness (sol review #12). The TS harness is what
// APPLIES these values, and it uses /^[0-9]+$/ plus Number.isSafeInteger. Go's
// strconv.Atoi was wider in two directions that both ended in a silent no-op:
// a leading sign passed here and failed there (effort quietly fell back to the
// SDK default), and a value above 2^53 passed here and made maxTurns UNLIMITED
// there — the opposite of what setting a huge cap intends.
//
// The mirror of this table lives in harness/claude-sdk/test/options.test.mjs;
// the two must agree case for case.
func TestIntegerKnobParityWithTheHarness(t *testing.T) {
	for _, tc := range []struct {
		in   string
		want bool
		why  string
	}{
		{"1", true, "the smallest positive budget"},
		{"32000", true, "an ordinary budget"},
		{"9007199254740991", true, "exactly Number.MAX_SAFE_INTEGER"},
		{"+5", false, "a leading plus: strconv.Atoi took it, the harness's digits-only regex does not"},
		{"-5", false, "negative"},
		{"0", false, "zero is not a positive budget"},
		{"9007199254740992", false, "one past MAX_SAFE_INTEGER: the harness treats it as unset"},
		{"99999999999999999999", false, "far past what JavaScript represents exactly"},
		{" 5", false, "callers trim before validating; an untrimmed value is not valid"},
		{"5.0", false, "not an integer"},
		{"1e3", false, "exponent form"},
		{"", false, "empty"},
		{"abc", false, "junk"},
	} {
		if got := validIntegerKnob(tc.in); got != tc.want {
			t.Errorf("validIntegerKnob(%q) = %v, want %v — %s", tc.in, got, tc.want, tc.why)
		}
		// The effort knob delegates to the same rule, so a numeric effort must
		// agree with it exactly (the symbolic names are tested separately).
		if got := ValidSDKEffort(tc.in); !sdkEffortNames[tc.in] && got != tc.want {
			t.Errorf("ValidSDKEffort(%q) = %v, want %v — %s", tc.in, got, tc.want, tc.why)
		}
	}
}

// TestMaxTurnsRejectsWhatTheHarnessWouldIgnore: the loud-failure guarantee has
// to cover the range mismatch too, or `up` starts a box whose turn cap silently
// does not exist.
func TestMaxTurnsRejectsWhatTheHarnessWouldIgnore(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", root)
	for _, k := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_SDK_SETTING_SOURCES", "HOTLINE_CLAUDE_SDK_MODEL"} {
		os.Unsetenv(k)
	}
	for _, bad := range []string{"+5", "9007199254740992", "-1", "0"} {
		if err := os.WriteFile(filepath.Join(root, ".env"), []byte("HOTLINE_SDK_MAX_TURNS="+bad+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := LoadSDK(); err == nil {
			t.Errorf("HOTLINE_SDK_MAX_TURNS=%q was accepted; the harness would ignore it and run unlimited", bad)
		}
	}
	if err := os.WriteFile(filepath.Join(root, ".env"), []byte("HOTLINE_SDK_MAX_TURNS=9007199254740991\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadSDK()
	if err != nil {
		t.Fatalf("MAX_SAFE_INTEGER rejected: %v", err)
	}
	if cfg.MaxTurns != 9007199254740991 {
		t.Fatalf("MaxTurns = %d", cfg.MaxTurns)
	}
}
