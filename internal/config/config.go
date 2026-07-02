// Package config resolves the channel's state directory, loads the .env token
// file (matching the official Telegram channel's convention), and exposes the
// paths the rest of the program uses.
package config

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// Config holds resolved paths and the loaded bot token.
type Config struct {
	// BotName is the selected bot ("" = the default/unnamed bot). Named bots get
	// isolated state under <baseDir>/bots/<name> and a per-name token env key.
	BotName string

	StateDir       string
	EnvFile        string
	AccessFile     string
	InboxDir       string
	ApprovedDir    string
	PidFile        string
	TranscriptFile string

	// Token is the TELEGRAM_BOT_TOKEN after merging the real environment with
	// the .env file. Empty if unset (the channel still runs the MCP handshake,
	// but skips the poller and tools report "no bot token configured").
	Token string

	// Static is true when TELEGRAM_ACCESS_MODE == "static": access.json is
	// snapshotted and pairing writes are disabled.
	Static bool
}

var envLineRe = regexp.MustCompile(`^(\w+)=(.*)$`)

// botNameRe constrains a bot name so it is safe as both a directory segment and
// an env-key suffix (no path separators, no traversal, no env-hostile chars).
var botNameRe = regexp.MustCompile(`^[a-zA-Z0-9_]+$`)

// Load resolves the per-bot state directory, loads the shared .env file (without
// overriding the real process environment), and reads the bot's token + the
// access mode.
//
// botName selects which bot to run. "" is the default/unnamed bot: state lives
// directly in the base dir and the token is TELEGRAM_BOT_TOKEN (the original,
// single-bot layout — unchanged). A named bot isolates its state under
// <baseDir>/bots/<name> and reads its token from TELEGRAM_BOT_TOKEN_<NAME>
// (uppercased), so multiple bots can run side by side from one .env. The .env
// itself always lives in the base dir, holding every bot's token.
//
// State-dir precedence (the base dir): $HOTLINE_STATE_DIR, then
// $TELE_GO_STATE_DIR (legacy, kept for one release), then
// $TELEGRAM_STATE_DIR, then ~/.claude/channels/tele-go. The default path
// deliberately keeps the historical tele-go name so existing pairings,
// allowlists, and transcripts survive the rename unchanged.
func Load(botName string) (*Config, error) {
	if botName != "" && !botNameRe.MatchString(botName) {
		return nil, fmt.Errorf("invalid bot name %q: use letters, digits, and underscores only", botName)
	}

	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}

	stateDir := baseDir
	tokenKey := "TELEGRAM_BOT_TOKEN"
	if botName != "" {
		stateDir = filepath.Join(baseDir, "bots", botName)
		tokenKey = "TELEGRAM_BOT_TOKEN_" + strings.ToUpper(botName)
	}

	c := &Config{
		BotName:        botName,
		StateDir:       stateDir,
		EnvFile:        filepath.Join(baseDir, ".env"),
		AccessFile:     filepath.Join(stateDir, "access.json"),
		InboxDir:       filepath.Join(stateDir, "inbox"),
		ApprovedDir:    filepath.Join(stateDir, "approved"),
		PidFile:        filepath.Join(stateDir, "bot.pid"),
		TranscriptFile: filepath.Join(stateDir, "transcript.jsonl"),
	}

	// Best-effort: lock the credential file to owner-only. Ignore failure (the
	// file may not exist yet, or we may be on a filesystem that rejects chmod).
	_ = os.Chmod(c.EnvFile, 0o600)

	dotEnv, err := loadDotEnv(c.EnvFile)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", c.EnvFile, err)
	}

	c.Token = mergedEnv(tokenKey, dotEnv)
	c.Static = mergedEnv("TELEGRAM_ACCESS_MODE", dotEnv) == "static"

	return c, nil
}

// mergedEnv returns the real process environment value if present, otherwise
// the value parsed from the .env file. The real environment always wins.
func mergedEnv(key string, dotEnv map[string]string) string {
	if v, ok := os.LookupEnv(key); ok {
		return v
	}
	return dotEnv[key]
}

func resolveStateDir() (string, error) {
	if v := os.Getenv("HOTLINE_STATE_DIR"); v != "" {
		return v, nil
	}
	// Legacy fallback (pre-rename name), honored for one release.
	if v := os.Getenv("TELE_GO_STATE_DIR"); v != "" {
		return v, nil
	}
	if v := os.Getenv("TELEGRAM_STATE_DIR"); v != "" {
		return v, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolving home directory: %w", err)
	}
	// Kept as "tele-go" on purpose: this is where pre-rename state
	// (access.json, transcripts, inbox) already lives, and the renamed binary
	// must keep reading it.
	return filepath.Join(home, ".claude", "channels", "tele-go"), nil
}

// EnsureDirs creates the state, inbox, and approved directories with 0700
// permissions. It is idempotent.
func (c *Config) EnsureDirs() error {
	for _, dir := range []string{c.StateDir, c.InboxDir, c.ApprovedDir} {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return fmt.Errorf("creating %s: %w", dir, err)
		}
	}
	return nil
}

// loadDotEnv parses a simple KEY=VALUE .env file. Lines starting with '#' and
// blank lines are ignored; surrounding single or double quotes are stripped. A
// missing file is not an error (returns an empty map).
func loadDotEnv(path string) (map[string]string, error) {
	out := make(map[string]string)
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return out, nil
		}
		return nil, err
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		m := envLineRe.FindStringSubmatch(line)
		if m == nil {
			continue
		}
		out[m[1]] = unquote(strings.TrimSpace(m[2]))
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return out, nil
}

func unquote(s string) string {
	if len(s) >= 2 {
		if (s[0] == '"' && s[len(s)-1] == '"') || (s[0] == '\'' && s[len(s)-1] == '\'') {
			return s[1 : len(s)-1]
		}
	}
	return s
}
