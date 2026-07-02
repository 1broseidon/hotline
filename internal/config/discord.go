package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// LoadDiscord resolves the per-instance Discord state directory and token. It
// mirrors Load's telegram layout under a "discord" subtree of the shared base
// dir: the default instance lives at <baseDir>/discord and reads
// DISCORD_BOT_TOKEN; a named instance isolates its state under
// <baseDir>/discord/instances/<name> and reads DISCORD_BOT_TOKEN_<NAME>
// (uppercased). The shared .env in the base dir holds every provider's token.
func LoadDiscord(instance string) (*Config, error) {
	if instance != "" && !botNameRe.MatchString(instance) {
		return nil, fmt.Errorf("invalid discord instance %q: use letters, digits, and underscores only", instance)
	}

	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}

	stateDir := filepath.Join(baseDir, "discord")
	tokenKey := "DISCORD_BOT_TOKEN"
	if instance != "" {
		stateDir = filepath.Join(baseDir, "discord", "instances", instance)
		tokenKey = "DISCORD_BOT_TOKEN_" + strings.ToUpper(instance)
	}

	c := &Config{
		BotName:        instance,
		StateDir:       stateDir,
		EnvFile:        filepath.Join(baseDir, ".env"),
		AccessFile:     filepath.Join(stateDir, "access.json"),
		InboxDir:       filepath.Join(stateDir, "inbox"),
		ApprovedDir:    filepath.Join(stateDir, "approved"),
		PidFile:        filepath.Join(stateDir, "bot.pid"),
		TranscriptFile: filepath.Join(stateDir, "transcript.jsonl"),
	}

	_ = os.Chmod(c.EnvFile, 0o600)

	dotEnv, err := loadDotEnv(c.EnvFile)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", c.EnvFile, err)
	}

	c.Token = mergedEnv(tokenKey, dotEnv)
	c.Static = mergedEnv("DISCORD_ACCESS_MODE", dotEnv) == "static"

	return c, nil
}
