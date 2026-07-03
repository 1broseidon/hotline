package config

import (
	"fmt"
	"path/filepath"
	"strings"
)

// DefaultOpenCodeServerURL is where a locally run `opencode serve` listens.
const DefaultOpenCodeServerURL = "http://127.0.0.1:4096"

// OpenCodeConfig holds the settings for driving an OpenCode harness over its
// HTTP+SSE control plane. It is populated only in HOTLINE_HARNESS=opencode
// mode; the messaging-provider config (telegram/signal/discord) is orthogonal
// and unchanged.
type OpenCodeConfig struct {
	// ServerURL is the `opencode serve` root, e.g. "http://127.0.0.1:4096"
	// (no trailing slash). Defaults to DefaultOpenCodeServerURL.
	ServerURL string
	// Password is the optional basic-auth secret (OPENCODE_SERVER_PASSWORD).
	// Empty means no auth.
	Password string
	// Session pins the target session id (OPENCODE_SESSION). Empty lets the
	// adapter resolve the most-recently-active session from GET /session.
	Session string
}

// LoadOpenCode resolves the OpenCode harness settings from the real environment
// (which wins) merged with the shared base-dir .env, mirroring LoadSignal's
// env-key style: OPENCODE_SERVER_URL, OPENCODE_SERVER_PASSWORD, OPENCODE_SESSION.
func LoadOpenCode() (*OpenCodeConfig, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", envFile, err)
	}

	c := &OpenCodeConfig{
		ServerURL: mergedEnv("OPENCODE_SERVER_URL", dotEnv),
		Password:  mergedEnv("OPENCODE_SERVER_PASSWORD", dotEnv),
		Session:   mergedEnv("OPENCODE_SESSION", dotEnv),
	}
	if c.ServerURL == "" {
		c.ServerURL = DefaultOpenCodeServerURL
	}
	c.ServerURL = strings.TrimRight(c.ServerURL, "/")
	return c, nil
}

// Harness resolves which coding-agent harness hotline drives, from
// HOTLINE_HARNESS (real env wins over .env), defaulting to "claude". The only
// other supported value is "opencode", which selects the OpenCode HTTP+SSE
// control plane. Unknown values are rejected so a typo fails loudly instead of
// silently falling back to Claude Code.
func Harness() (string, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return "", err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return "", fmt.Errorf("reading %s: %w", envFile, err)
	}
	h := strings.ToLower(strings.TrimSpace(mergedEnv("HOTLINE_HARNESS", dotEnv)))
	switch h {
	case "", "claude":
		return "claude", nil
	case "opencode":
		return "opencode", nil
	default:
		return "", fmt.Errorf("unknown HOTLINE_HARNESS %q (supported: claude, opencode)", h)
	}
}
