package config

import (
	"fmt"
	"path/filepath"
	"strings"
)

// PublishExposure resolves which exposure backend the publish tool uses to make
// a locally served artifact reachable, from HOTLINE_PUBLISH_EXPOSURE (real env
// wins over .env), defaulting to "localhostrun".
//
// Supported values:
//   - "localhostrun" (default) — an ssh tunnel to localhost.run (*.lhr.life).
//   - "cloudflared"            — a cloudflared quick tunnel (binary must be on PATH).
//   - "local" / "off"          — no tunnel; serve on loopback only, for operators
//     who expose via their own proxy, SSH port-forward, or LAN.
//
// The default is deliberately NOT auto-switched to cloudflared even when the
// binary is present: an explicit choice is the only thing that changes the
// backend, so behavior stays predictable. Unknown values are rejected so a typo
// fails loudly instead of silently falling back.
func PublishExposure() (string, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return "", err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return "", fmt.Errorf("reading %s: %w", envFile, err)
	}
	v := strings.ToLower(strings.TrimSpace(mergedEnv("HOTLINE_PUBLISH_EXPOSURE", dotEnv)))
	switch v {
	case "", "localhostrun":
		return "localhostrun", nil
	case "cloudflared":
		return "cloudflared", nil
	case "local", "off":
		return "local", nil
	default:
		return "", fmt.Errorf("unknown HOTLINE_PUBLISH_EXPOSURE %q (supported: localhostrun, cloudflared, local/off)", v)
	}
}
