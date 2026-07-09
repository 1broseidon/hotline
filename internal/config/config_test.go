package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// envToMap collapses a KEY=VALUE environment slice into a map (last write wins,
// matching exec semantics) for assertions.
func envToMap(env []string) map[string]string {
	m := make(map[string]string, len(env))
	for _, kv := range env {
		if k, v, ok := strings.Cut(kv, "="); ok {
			m[k] = v
		}
	}
	return m
}

// anthropicTestState points state at a temp dir with the given .env content and
// clears any allowlisted keys leaking in from the developer's real environment.
func anthropicTestState(t *testing.T, dotenv string) {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	for _, k := range AnthropicEnvKeys {
		t.Setenv(k, "") // register restore-on-cleanup
		os.Unsetenv(k)  // then actually clear it for the duration of the test
	}
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(dotenv), 0o600); err != nil {
		t.Fatal(err)
	}
}

// TestAnthropicChildEnv: the five allowlisted keys cross from .env into the
// child env; a non-allowlisted key (the telegram token) does not.
func TestAnthropicChildEnv(t *testing.T) {
	anthropicTestState(t, "TELEGRAM_BOT_TOKEN=123:secret\n"+
		"ANTHROPIC_BASE_URL=https://alt.example/v1\n"+
		"ANTHROPIC_AUTH_TOKEN=bearer-xyz\n"+
		"ANTHROPIC_API_KEY=key-abc\n"+
		"ANTHROPIC_MODEL=alt-model\n"+
		"ANTHROPIC_DEFAULT_HAIKU_MODEL=alt-haiku\n"+ // per-role override
		"ANTHROPIC_SMALL_FAST_MODEL=alt-small\n"+ // deprecated, back-compat passthrough
		"ENABLE_TOOL_SEARCH=true\n") // non-ANTHROPIC-prefixed allowlist key

	got, err := AnthropicChildEnv(nil)
	if err != nil {
		t.Fatal(err)
	}
	m := envToMap(got)
	for k, want := range map[string]string{
		"ANTHROPIC_BASE_URL":            "https://alt.example/v1",
		"ANTHROPIC_AUTH_TOKEN":          "bearer-xyz",
		"ANTHROPIC_API_KEY":             "key-abc",
		"ANTHROPIC_MODEL":               "alt-model",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL": "alt-haiku",
		"ANTHROPIC_SMALL_FAST_MODEL":    "alt-small",
		"ENABLE_TOOL_SEARCH":            "true",
	} {
		if m[k] != want {
			t.Errorf("%s = %q, want %q", k, m[k], want)
		}
	}
	if _, ok := m["TELEGRAM_BOT_TOKEN"]; ok {
		t.Error("TELEGRAM_BOT_TOKEN leaked into the child env — only the allowlist may cross over")
	}
}

// TestAnthropicChildEnvRealEnvWins: a key already set in the real process
// environment is not overridden by the .env, and the .env value is not appended
// as a duplicate.
func TestAnthropicChildEnvRealEnvWins(t *testing.T) {
	anthropicTestState(t, "ANTHROPIC_MODEL=file-model\n")
	t.Setenv("ANTHROPIC_MODEL", "real-model")

	got, err := AnthropicChildEnv(os.Environ())
	if err != nil {
		t.Fatal(err)
	}
	if m := envToMap(got); m["ANTHROPIC_MODEL"] != "real-model" {
		t.Errorf("ANTHROPIC_MODEL = %q, want real-model (real env wins)", m["ANTHROPIC_MODEL"])
	}
	n := 0
	for _, kv := range got {
		if strings.HasPrefix(kv, "ANTHROPIC_MODEL=") {
			n++
		}
	}
	if n != 1 {
		t.Errorf("ANTHROPIC_MODEL appears %d times, want 1 (no .env duplicate appended)", n)
	}
}

func TestStateDirPrecedence(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", "/h")
	t.Setenv("TELE_GO_STATE_DIR", "/a")
	t.Setenv("TELEGRAM_STATE_DIR", "/b")
	if got, _ := resolveStateDir(); got != "/h" {
		t.Fatalf("HOTLINE_STATE_DIR should win, got %q", got)
	}
	os.Unsetenv("HOTLINE_STATE_DIR")
	if got, _ := resolveStateDir(); got != "/a" {
		t.Fatalf("legacy TELE_GO_STATE_DIR should win, got %q", got)
	}
	os.Unsetenv("TELE_GO_STATE_DIR")
	if got, _ := resolveStateDir(); got != "/b" {
		t.Fatalf("TELEGRAM_STATE_DIR should win, got %q", got)
	}
}

// TestStateDirDefault pins the hotline-owned default: with no override envs
// set, state resolves to ${XDG_CONFIG_HOME:-~/.config}/hotline.
func TestStateDirDefault(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", "")
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	t.Setenv("HOME", home)

	t.Setenv("XDG_CONFIG_HOME", "")
	got, err := resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(home, ".config", "hotline"); got != want {
		t.Fatalf("default state dir = %q, want %q", got, want)
	}

	xdg := filepath.Join(home, "xdg")
	t.Setenv("XDG_CONFIG_HOME", xdg)
	got, err = resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(xdg, "hotline"); got != want {
		t.Fatalf("XDG state dir = %q, want %q", got, want)
	}
}

func TestLoadDotEnv(t *testing.T) {
	dir := t.TempDir()
	envFile := filepath.Join(dir, ".env")
	content := "# comment\n\nTELEGRAM_BOT_TOKEN=\"123:abc\"\nFOO='bar baz'\nNOEQ\nQUX=plain\n"
	if err := os.WriteFile(envFile, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	m, err := loadDotEnv(envFile)
	if err != nil {
		t.Fatal(err)
	}
	if m["TELEGRAM_BOT_TOKEN"] != "123:abc" {
		t.Fatalf("token = %q", m["TELEGRAM_BOT_TOKEN"])
	}
	if m["FOO"] != "bar baz" {
		t.Fatalf("FOO = %q", m["FOO"])
	}
	if m["QUX"] != "plain" {
		t.Fatalf("QUX = %q", m["QUX"])
	}
	if _, ok := m["NOEQ"]; ok {
		t.Fatal("line without = should be ignored")
	}
}

func TestLoadDotEnvMissing(t *testing.T) {
	m, err := loadDotEnv(filepath.Join(t.TempDir(), "absent.env"))
	if err != nil {
		t.Fatalf("missing file should not error: %v", err)
	}
	if len(m) != 0 {
		t.Fatal("expected empty map")
	}
}

func TestRealEnvWins(t *testing.T) {
	t.Setenv("TELEGRAM_BOT_TOKEN", "real-token")
	got := mergedEnv("TELEGRAM_BOT_TOKEN", map[string]string{"TELEGRAM_BOT_TOKEN": "dotenv-token"})
	if got != "real-token" {
		t.Fatalf("real env should win, got %q", got)
	}
}

func TestMergedEnvFallback(t *testing.T) {
	os.Unsetenv("SOME_UNSET_KEY")
	got := mergedEnv("SOME_UNSET_KEY", map[string]string{"SOME_UNSET_KEY": "fromdot"})
	if got != "fromdot" {
		t.Fatalf("expected dotenv fallback, got %q", got)
	}
}

func TestLoadFull(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	os.Unsetenv("TELEGRAM_STATE_DIR")
	t.Setenv("TELEGRAM_ACCESS_MODE", "static")
	os.Unsetenv("TELEGRAM_BOT_TOKEN")
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("TELEGRAM_BOT_TOKEN=tok\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := Load("")
	if err != nil {
		t.Fatal(err)
	}
	if c.Token != "tok" {
		t.Fatalf("token = %q", c.Token)
	}
	if !c.Static {
		t.Fatal("expected static mode")
	}
	if c.AccessFile != filepath.Join(dir, "access.json") {
		t.Fatalf("access file = %q", c.AccessFile)
	}
	if err := c.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	for _, d := range []string{c.InboxDir, c.ApprovedDir} {
		if fi, err := os.Stat(d); err != nil || !fi.IsDir() {
			t.Fatalf("dir %q not created", d)
		}
	}
}

func TestLoadNamedBotIsolation(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	os.Unsetenv("TELEGRAM_STATE_DIR")
	os.Unsetenv("TELEGRAM_BOT_TOKEN")
	os.Unsetenv("TELEGRAM_BOT_TOKEN_WORK")
	env := "TELEGRAM_BOT_TOKEN=deftok\nTELEGRAM_BOT_TOKEN_WORK=worktok\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(env), 0o600); err != nil {
		t.Fatal(err)
	}

	// Named bot: isolated state dir, per-name token, shared base .env.
	work, err := Load("work")
	if err != nil {
		t.Fatal(err)
	}
	if work.Token != "worktok" {
		t.Errorf("work token = %q, want worktok", work.Token)
	}
	wantState := filepath.Join(dir, "bots", "work")
	if work.StateDir != wantState {
		t.Errorf("work state dir = %q, want %q", work.StateDir, wantState)
	}
	if work.AccessFile != filepath.Join(wantState, "access.json") {
		t.Errorf("work access file = %q", work.AccessFile)
	}
	if work.TranscriptFile != filepath.Join(wantState, "transcript.jsonl") {
		t.Errorf("work transcript = %q", work.TranscriptFile)
	}
	if work.EnvFile != filepath.Join(dir, ".env") {
		t.Errorf("env file should stay in base dir, got %q", work.EnvFile)
	}
	if work.BotName != "work" {
		t.Errorf("bot name = %q", work.BotName)
	}

	// Default bot: base dir, original token, unchanged layout.
	def, err := Load("")
	if err != nil {
		t.Fatal(err)
	}
	if def.Token != "deftok" {
		t.Errorf("default token = %q, want deftok", def.Token)
	}
	if def.StateDir != dir {
		t.Errorf("default state dir = %q, want %q", def.StateDir, dir)
	}
}

func TestLoadUnknownBotTokenEmpty(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	os.Unsetenv("TELEGRAM_STATE_DIR")
	os.Unsetenv("TELEGRAM_BOT_TOKEN")
	os.Unsetenv("TELEGRAM_BOT_TOKEN_GHOST")
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("TELEGRAM_BOT_TOKEN=deftok\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// A named bot does NOT fall back to the default token — its own key is unset,
	// so it runs token-less (handshake only) rather than hijacking the default.
	c, err := Load("ghost")
	if err != nil {
		t.Fatal(err)
	}
	if c.Token != "" {
		t.Errorf("unknown bot token = %q, want empty", c.Token)
	}
}

func TestLoadInvalidBotName(t *testing.T) {
	for _, bad := range []string{"../evil", "a/b", "has space", "dot.name", "semi;colon"} {
		if _, err := Load(bad); err == nil {
			t.Errorf("bot name %q should be rejected", bad)
		}
	}
	// Underscores and digits are allowed.
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	if _, err := Load("team_1"); err != nil {
		t.Errorf("valid name team_1 rejected: %v", err)
	}
}
