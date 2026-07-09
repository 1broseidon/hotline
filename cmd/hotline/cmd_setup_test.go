package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func setupTestState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	return dir
}

const goodToken = "123456789:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"

func TestSetupWritesEnvWithPerms(t *testing.T) {
	dir := setupTestState(t)
	var out bytes.Buffer
	err := cmdSetup("", []string{"--telegram-token", goodToken}, strings.NewReader(""), &out, false)
	if err != nil {
		t.Fatalf("setup: %v", err)
	}
	envFile := filepath.Join(dir, ".env")
	fi, err := os.Stat(envFile)
	if err != nil {
		t.Fatal(err)
	}
	if fi.Mode().Perm() != 0o600 {
		t.Errorf(".env perms = %v, want 0600", fi.Mode().Perm())
	}
	env, _ := readEnvFile(envFile)
	if env["TELEGRAM_BOT_TOKEN"] != goodToken {
		t.Errorf("token not written: %q", env["TELEGRAM_BOT_TOKEN"])
	}
	if strings.Contains(out.String(), goodToken) {
		t.Error("output leaks the full token")
	}
}

func TestSetupMergePreservesExistingKeys(t *testing.T) {
	dir := setupTestState(t)
	envFile := filepath.Join(dir, ".env")
	orig := "# my comment\nDISCORD_BOT_TOKEN=keepme\nTELEGRAM_BOT_TOKEN=0:old\n"
	if err := os.WriteFile(envFile, []byte(orig), 0o600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdSetup("", []string{"--telegram-token", goodToken, "--signal-account", "+15551234567"}, strings.NewReader(""), &out, false); err != nil {
		t.Fatalf("setup: %v", err)
	}
	data, _ := os.ReadFile(envFile)
	s := string(data)
	if !strings.Contains(s, "# my comment") {
		t.Error("comment dropped")
	}
	if !strings.Contains(s, "DISCORD_BOT_TOKEN=keepme") {
		t.Error("unrelated key clobbered")
	}
	if !strings.Contains(s, "TELEGRAM_BOT_TOKEN="+goodToken) {
		t.Error("token not updated in place")
	}
	if !strings.Contains(s, "SIGNAL_ACCOUNT=+15551234567") {
		t.Error("new key not appended")
	}
}

func TestSetupNamedBotKey(t *testing.T) {
	dir := setupTestState(t)
	var out bytes.Buffer
	if err := cmdSetup("work", []string{"--telegram-token", goodToken}, strings.NewReader(""), &out, false); err != nil {
		t.Fatalf("setup: %v", err)
	}
	env, _ := readEnvFile(filepath.Join(dir, ".env"))
	if env["TELEGRAM_BOT_TOKEN_WORK"] != goodToken {
		t.Errorf("named-bot key missing, env=%v", env)
	}
}

func TestSetupNonTTYMissingTokenErrors(t *testing.T) {
	setupTestState(t)
	var out bytes.Buffer
	err := cmdSetup("", nil, strings.NewReader(""), &out, false)
	if err == nil || !strings.Contains(err.Error(), "--telegram-token") {
		t.Errorf("want clear missing-flag error, got %v", err)
	}
}

func TestSetupPromptsOnTTY(t *testing.T) {
	dir := setupTestState(t)
	var out bytes.Buffer
	err := cmdSetup("", nil, strings.NewReader(goodToken+"\n"), &out, true)
	if err != nil {
		t.Fatalf("setup: %v", err)
	}
	env, _ := readEnvFile(filepath.Join(dir, ".env"))
	if env["TELEGRAM_BOT_TOKEN"] != goodToken {
		t.Error("prompted token not written")
	}
}

func TestSetupRejectsBadValues(t *testing.T) {
	setupTestState(t)
	var out bytes.Buffer
	if err := cmdSetup("", []string{"--telegram-token", "notatoken"}, strings.NewReader(""), &out, false); err == nil {
		t.Error("bad telegram token accepted")
	}
	if err := cmdSetup("", []string{"--telegram-token", goodToken, "--signal-account", "5551234567"}, strings.NewReader(""), &out, false); err == nil {
		t.Error("non-E.164 signal account accepted")
	}
}

func TestSetupShow(t *testing.T) {
	dir := setupTestState(t)
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("TELEGRAM_BOT_TOKEN="+goodToken+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdSetup("", []string{"--show"}, strings.NewReader(""), &out, false); err != nil {
		t.Fatalf("show: %v", err)
	}
	if strings.Contains(out.String(), goodToken) {
		t.Error("show leaks the full token")
	}
	if !strings.Contains(out.String(), "TELEGRAM_BOT_TOKEN") {
		t.Error("show missing token line")
	}
}

// TestSetupWritesAnthropicKeys: the five alternate-provider flags land in the
// shared .env, and the echo of what was written masks the two secrets.
func TestSetupWritesAnthropicKeys(t *testing.T) {
	dir := setupTestState(t)
	var out bytes.Buffer
	args := []string{
		"--telegram-token", goodToken,
		"--anthropic-base-url", "https://alt.example/v1",
		"--anthropic-token", "bearer-supersecret-value",
		"--anthropic-api-key", "apikey-supersecret-value",
		"--anthropic-model", "alt-model",
	}
	if err := cmdSetup("", args, strings.NewReader(""), &out, false); err != nil {
		t.Fatalf("setup: %v", err)
	}
	env, _ := readEnvFile(filepath.Join(dir, ".env"))
	for k, v := range map[string]string{
		"ANTHROPIC_BASE_URL":   "https://alt.example/v1",
		"ANTHROPIC_AUTH_TOKEN": "bearer-supersecret-value",
		"ANTHROPIC_API_KEY":    "apikey-supersecret-value",
		"ANTHROPIC_MODEL":      "alt-model",
	} {
		if env[k] != v {
			t.Errorf("%s = %q, want %q", k, env[k], v)
		}
	}
	if strings.Contains(out.String(), "bearer-supersecret-value") {
		t.Error("setup output leaks ANTHROPIC_AUTH_TOKEN")
	}
	if strings.Contains(out.String(), "apikey-supersecret-value") {
		t.Error("setup output leaks ANTHROPIC_API_KEY")
	}
}

// TestSetupShowMasksAnthropicSecrets: --show masks the two credentials but
// prints the base URL and model names in the clear.
func TestSetupShowMasksAnthropicSecrets(t *testing.T) {
	dir := setupTestState(t)
	content := "TELEGRAM_BOT_TOKEN=" + goodToken + "\n" +
		"ANTHROPIC_BASE_URL=https://alt.example/v1\n" +
		"ANTHROPIC_AUTH_TOKEN=bearer-supersecret-value\n" +
		"ANTHROPIC_API_KEY=apikey-supersecret-value\n" +
		"ANTHROPIC_MODEL=alt-model\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdSetup("", []string{"--show"}, strings.NewReader(""), &out, false); err != nil {
		t.Fatalf("show: %v", err)
	}
	s := out.String()
	if strings.Contains(s, "bearer-supersecret-value") {
		t.Error("show leaks ANTHROPIC_AUTH_TOKEN")
	}
	if strings.Contains(s, "apikey-supersecret-value") {
		t.Error("show leaks ANTHROPIC_API_KEY")
	}
	if !strings.Contains(s, "ANTHROPIC_BASE_URL: https://alt.example/v1") {
		t.Errorf("show should print base URL in the clear:\n%s", s)
	}
	if !strings.Contains(s, "ANTHROPIC_MODEL: alt-model") {
		t.Errorf("show should print model in the clear:\n%s", s)
	}
}

func TestValidators(t *testing.T) {
	if validTelegramToken("123:short") {
		t.Error("short token passed")
	}
	if !validTelegramToken(goodToken) {
		t.Error("good token failed")
	}
	if !validE164("+306912345678") {
		t.Error("good E.164 failed")
	}
	for _, bad := range []string{"+0123456789", "15551234567", "+1", "+1555123456789012345"} {
		if validE164(bad) {
			t.Errorf("bad E.164 %q passed", bad)
		}
	}
}
