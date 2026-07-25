package config

import (
	"fmt"
	"os"
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
	// Agent pins every inbound turn to a named opencode agent
	// (HOTLINE_OPENCODE_AGENT). `hotline init --harness opencode` sets it to
	// "hotline" (the scaffolded agent). Empty omits the agent field on
	// prompt_async, preserving the pre-agent default-agent behavior.
	Agent string
}

// LoadOpenCode resolves the OpenCode harness settings from the real environment
// (which wins) merged with the shared base-dir .env, mirroring LoadSignal's
// env-key style: OPENCODE_SERVER_URL, OPENCODE_SERVER_PASSWORD, OPENCODE_SESSION,
// HOTLINE_OPENCODE_AGENT.
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
		Agent:     mergedEnv("HOTLINE_OPENCODE_AGENT", dotEnv),
	}
	if c.ServerURL == "" {
		c.ServerURL = DefaultOpenCodeServerURL
	}
	c.ServerURL = strings.TrimRight(c.ServerURL, "/")
	return c, nil
}

// HarnessValues lists the supported coding-agent harness identifiers, in the
// order the docs use. It is the single source of truth for the `--harness` flag
// help on `up` and `init` and the NormalizeHarness switch below.
var HarnessValues = []string{"claude", "claude-sdk", "opencode", "pi"}

// NormalizeHarness canonicalizes a raw harness identifier (case-insensitive,
// trimmed). An empty value defaults to "claude". Unknown values are rejected so
// a typo fails loudly instead of silently falling back to Claude Code. This is
// the one switch shared by Harness (env resolution) and the `--harness` flag on
// `up`/`init`, so the accepted set never drifts between them.
func NormalizeHarness(raw string) (string, error) {
	h := strings.ToLower(strings.TrimSpace(raw))
	switch h {
	case "", "claude":
		return "claude", nil
	case "opencode":
		return "opencode", nil
	case "pi":
		return "pi", nil
	case "claude-sdk":
		return "claude-sdk", nil
	default:
		return "", fmt.Errorf("unknown harness %q (supported: %s)", h, strings.Join(HarnessValues, ", "))
	}
}

// HarnessSource names where a resolved harness value came from, for the
// launch-time attribution line `hotline up` prints. Its string form is the
// parenthetical the operator sees, e.g. `hotline: harness claude (default)`.
type HarnessSource string

const (
	HarnessSourceDefault HarnessSource = "default"
	HarnessSourceFlag    HarnessSource = "from --harness"
	HarnessSourceEnv     HarnessSource = "from HOTLINE_HARNESS"
	HarnessSourceDotEnv  HarnessSource = "from state .env"
)

// Harness resolves which coding-agent harness hotline drives, from
// HOTLINE_HARNESS (real env wins over .env), defaulting to "claude". Other
// supported values are "opencode" (the OpenCode HTTP+SSE control plane),
// "pi" (a supervised `pi --mode rpc` session driven by the hotline-pi
// extension over stdio), and "claude-sdk" (the Agent-SDK managed claude
// edition: a supervised node harness that spawns `hotline run` itself).
// Unknown values are rejected so a typo fails loudly instead of silently
// falling back to Claude Code.
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
	return NormalizeHarness(mergedEnv("HOTLINE_HARNESS", dotEnv))
}

// HarnessResolved returns the same value as Harness plus where it came from, so
// `hotline up` can announce the resolved harness AND why. The `--harness` flag
// is not visible here — `up` exports it into HOTLINE_HARNESS first, so a
// flag-set value surfaces as HarnessSourceEnv and the caller relabels it
// HarnessSourceFlag. An empty HOTLINE_HARNESS is treated as unset for the source
// label (it resolves to the claude default anyway).
func HarnessResolved() (string, HarnessSource, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return "", "", err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return "", "", fmt.Errorf("reading %s: %w", envFile, err)
	}
	h, err := NormalizeHarness(mergedEnv("HOTLINE_HARNESS", dotEnv))
	if err != nil {
		return "", "", err
	}
	// Attribution tracks the same mergedEnv precedence that produced the value:
	// a present real env var wins (even empty — it shadows the .env, and an empty
	// value resolves to the claude default), then the state .env, else the
	// built-in default.
	source := HarnessSourceDefault
	if v, ok := os.LookupEnv("HOTLINE_HARNESS"); ok {
		if strings.TrimSpace(v) != "" {
			source = HarnessSourceEnv
		}
	} else if strings.TrimSpace(dotEnv["HOTLINE_HARNESS"]) != "" {
		source = HarnessSourceDotEnv
	}
	return h, source, nil
}
