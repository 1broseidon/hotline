package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// DefaultMCIndexBudget is the byte cap for the injected mission-control index
// render when HOTLINE_MC_INDEX_BUDGET is unset.
const DefaultMCIndexBudget = 4096

// MCConfig is the resolved Mission Control configuration for one session.
type MCConfig struct {
	// Enabled reports whether Mission Control mounts for this harness: the
	// mission tool is registered, the state dir is seeded, and the index is
	// injected. See MissionControl for the env semantics.
	Enabled bool
	// Dir is the Mission Control directory (<box-root>/mc by default).
	Dir string
	// IndexBudget is the byte cap for the injected index render.
	IndexBudget int
	// ContextCap is the requested soft token cap the pi extension enforces (P1);
	// 0 = unset (the harness's own compaction threshold governs). Pi must retain
	// keepRecentTokens (20k by default), so the extension warns and raises an
	// unsafe cap to keepRecentTokens + a 4096-token compactable safety margin.
	// Parsed here so the whole config surface resolves in one place.
	ContextCap int
}

// MissionControl is the compatibility resolver for the box selected by
// HOTLINE_BOT (legacy: TELE_GO_BOT). New callers that already know the box
// selection should use MissionControlForBox or MissionControlForRoot.
func MissionControl(harness string) (MCConfig, error) {
	botName := os.Getenv("HOTLINE_BOT")
	if botName == "" {
		botName = os.Getenv("TELE_GO_BOT")
	}
	return MissionControlForBox(harness, botName)
}

// MissionControlForBox resolves Mission Control for the given harness and box.
func MissionControlForBox(harness, botName string) (MCConfig, error) {
	return resolveMissionControl(harness, func() (string, error) {
		boxRoot, err := BoxRoot(botName)
		if err != nil {
			return "", err
		}
		return filepath.Join(boxRoot, "mc"), nil
	})
}

// MissionControlForRoot resolves Mission Control using boxRoot for the default
// directory. It is for callers such as runChannel that already resolved the
// provider set and box once. Configuration still comes from the shared .env,
// with the real environment winning.
//
// HOTLINE_MISSION_CONTROL semantics (complete): unset ⇒ on for every harness,
// claude included (resolved call #3: full MC on CC — the mission tool mounts and
// claude gets the pointer line, its native hooks deliver the index); "0" ⇒ off
// everywhere; "1" ⇒ on everywhere. The other three envs — HOTLINE_MC_DIR,
// HOTLINE_MC_INDEX_BUDGET, HOTLINE_MC_CONTEXT_CAP — keep their existing
// precedence and parsing behavior.
func MissionControlForRoot(harness, boxRoot string) (MCConfig, error) {
	return resolveMissionControl(harness, func() (string, error) {
		return filepath.Join(boxRoot, "mc"), nil
	})
}

func resolveMissionControl(_ string, defaultDir func() (string, error)) (MCConfig, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return MCConfig{}, err
	}
	dotEnv, err := loadDotEnv(filepath.Join(baseDir, ".env"))
	if err != nil {
		return MCConfig{}, fmt.Errorf("reading %s: %w", filepath.Join(baseDir, ".env"), err)
	}

	c := MCConfig{IndexBudget: DefaultMCIndexBudget}

	switch strings.TrimSpace(mergedEnv("HOTLINE_MISSION_CONTROL", dotEnv)) {
	case "0":
		c.Enabled = false
	case "1":
		c.Enabled = true
	default:
		// FULL MC on every harness by default (resolved call #3): unset ⇒ ON,
		// including claude. Claude opts out with =0 like everyone else. The
		// harness only changes the injection VEHICLE (claude gets a pointer line
		// + the mission tool; its native hooks deliver the index), not whether MC
		// mounts.
		c.Enabled = true
	}

	if v := strings.TrimSpace(mergedEnv("HOTLINE_MC_DIR", dotEnv)); v != "" {
		c.Dir = v
	} else if c.Dir, err = defaultDir(); err != nil {
		return MCConfig{}, err
	}

	if v := strings.TrimSpace(mergedEnv("HOTLINE_MC_INDEX_BUDGET", dotEnv)); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			c.IndexBudget = n
		} else {
			fmt.Fprintf(os.Stderr, "hotline: invalid HOTLINE_MC_INDEX_BUDGET %q; using %d\n", v, DefaultMCIndexBudget)
		}
	}

	if v := strings.TrimSpace(mergedEnv("HOTLINE_MC_CONTEXT_CAP", dotEnv)); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			c.ContextCap = n
		} else {
			fmt.Fprintf(os.Stderr, "hotline: invalid HOTLINE_MC_CONTEXT_CAP %q; ignoring\n", v)
		}
	}

	return c, nil
}
