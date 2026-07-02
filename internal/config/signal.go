package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// DefaultSignalDaemonURL is where a locally run
// `signal-cli daemon --http 127.0.0.1:8080` listens.
const DefaultSignalDaemonURL = "http://127.0.0.1:8080"

// LoadSignal resolves the per-instance Signal state directory and daemon
// settings. It mirrors LoadDiscord's layout under a "signal" subtree of the
// shared base dir: the default instance lives at <baseDir>/signal and reads
// SIGNAL_ACCOUNT / SIGNAL_DAEMON_URL; a named instance isolates its state
// under <baseDir>/signal/instances/<name> and reads SIGNAL_ACCOUNT_<NAME> /
// SIGNAL_DAEMON_URL_<NAME> (uppercased). The shared .env in the base dir
// holds every provider's settings.
//
// Unlike telegram/discord there is no bot token: hotline talks to a locally
// running signal-cli HTTP daemon. SIGNAL_ACCOUNT (the E.164 number of the
// linked account) is what makes the provider "configured"; SIGNAL_DAEMON_URL
// defaults to DefaultSignalDaemonURL.
func LoadSignal(instance string) (*Config, error) {
	if instance != "" && !botNameRe.MatchString(instance) {
		return nil, fmt.Errorf("invalid signal instance %q: use letters, digits, and underscores only", instance)
	}

	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}

	stateDir := filepath.Join(baseDir, "signal")
	suffix := ""
	if instance != "" {
		stateDir = filepath.Join(baseDir, "signal", "instances", instance)
		suffix = "_" + strings.ToUpper(instance)
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

	c.SignalAccount = mergedEnv("SIGNAL_ACCOUNT"+suffix, dotEnv)
	c.SignalDaemonURL = mergedEnv("SIGNAL_DAEMON_URL"+suffix, dotEnv)
	if c.SignalDaemonURL == "" {
		c.SignalDaemonURL = DefaultSignalDaemonURL
	}
	c.SignalDaemonURL = strings.TrimRight(c.SignalDaemonURL, "/")
	c.Static = mergedEnv("SIGNAL_ACCESS_MODE"+suffix, dotEnv) == "static"

	return c, nil
}
