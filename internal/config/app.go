package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/supervise"
)

const (
	// DefaultAppBind is where the app-channel WebSocket server listens when
	// HOTLINE_APP_BIND is unset. Loopback-only by default: a network-reachable
	// bind must be chosen deliberately (a specific LAN/tailnet IP), and a wide-open
	// 0.0.0.0/:: bind additionally requires HOTLINE_APP_ALLOW_ANY=1.
	DefaultAppBind = "127.0.0.1:8990"

	// DefaultAPNsEnvironment sends ActivityKit updates to production APNs unless
	// an app instance explicitly selects the sandbox endpoint.
	DefaultAPNsEnvironment = "production"
)

// LoadApp resolves the per-instance state directory and bind settings for the
// app channel. It mirrors LoadSignal's layout under an "app" subtree of the
// shared base dir: the default instance lives at <baseDir>/app and reads
// HOTLINE_APP_BIND / HOTLINE_APP_TOKEN; a named instance isolates its state
// under <baseDir>/app/instances/<name> and reads app settings with a _<NAME>
// suffix (uppercased). The shared .env in the base dir holds every provider's
// settings.
//
// Auth: with no HOTLINE_APP_TOKEN the channel uses the same access model as
// every other provider (device secret → device id → pairing/allowlist). Setting
// HOTLINE_APP_TOKEN switches to token mode: any client presenting that exact
// token is allowed, bypassing pairing (a fallback for when the pairing UX isn't
// wired end to end). HOTLINE_APP_ALLOW_ANY=1 is the opt-in for a wide-open bind.
func LoadApp(instance string) (*Config, error) {
	if instance != "" && !botNameRe.MatchString(instance) {
		return nil, fmt.Errorf("invalid app instance %q: use letters, digits, and underscores only", instance)
	}

	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}

	stateDir := filepath.Join(baseDir, "app")
	suffix := ""
	if instance != "" {
		stateDir = filepath.Join(baseDir, "app", "instances", instance)
		suffix = "_" + strings.ToUpper(instance)
	}

	c := &Config{
		BotName:        instance,
		StateDir:       stateDir,
		StateRoot:      baseDir,
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

	c.AppBind = mergedEnv("HOTLINE_APP_BIND"+suffix, dotEnv)
	if c.AppBind == "" {
		c.AppBind = DefaultAppBind
	}
	c.AppToken = mergedEnv("HOTLINE_APP_TOKEN"+suffix, dotEnv)
	c.AppAllowAny = mergedEnv("HOTLINE_APP_ALLOW_ANY"+suffix, dotEnv) == "1"
	c.AppRelay = mergedEnv("HOTLINE_APP_RELAY"+suffix, dotEnv)
	c.AppRelayToken = mergedEnv("HOTLINE_APP_RELAY_TOKEN"+suffix, dotEnv)
	c.AppPush = mergedEnv("HOTLINE_APP_PUSH"+suffix, dotEnv) == "1"
	c.AppPushToken = mergedEnv("HOTLINE_APP_PUSH_TOKEN"+suffix, dotEnv)
	c.AppPushEndpoint = strings.TrimSpace(mergedEnv("HOTLINE_PUSH_ENDPOINT"+suffix, dotEnv))
	c.APNsKeyFile = strings.TrimSpace(mergedEnv("HOTLINE_APNS_KEY_FILE"+suffix, dotEnv))
	c.APNsKeyID = strings.TrimSpace(mergedEnv("HOTLINE_APNS_KEY_ID"+suffix, dotEnv))
	c.APNsTeamID = strings.TrimSpace(mergedEnv("HOTLINE_APNS_TEAM_ID"+suffix, dotEnv))
	c.APNsTopic = strings.TrimSpace(mergedEnv("HOTLINE_APNS_TOPIC"+suffix, dotEnv))
	c.APNsEnvironment = strings.TrimSpace(mergedEnv("HOTLINE_APNS_ENVIRONMENT"+suffix, dotEnv))
	if c.APNsEnvironment == "" {
		c.APNsEnvironment = DefaultAPNsEnvironment
	}
	c.CoreMode = mergedEnv("HOTLINE_CORE_MODE"+suffix, dotEnv) == "1"
	c.CoreURL = strings.TrimSpace(mergedEnv("HOTLINE_CORE_URL"+suffix, dotEnv))
	if c.CoreURL == "" {
		c.CoreURL = DefaultCoreURL
	}
	// HOTLINE_PUSH_PREVIEW=clear opts the box owner into readable push previews:
	// the wake hint carries the message plaintext and the core uses it as the
	// notification body. Any other value (or unset) ⇒ today's generic behavior.
	c.PushPreviewClear = mergedEnv("HOTLINE_PUSH_PREVIEW"+suffix, dotEnv) == "clear"
	// Multi-device sync kill switches (design-multidevice-sync §1.1, §10). Both
	// default ON; only an explicit "0" disables. Stored in the "off" sense so the
	// zero-value Config keeps the default-on behavior.
	c.UnifiedChatOff = mergedEnv("HOTLINE_UNIFIED_CHAT"+suffix, dotEnv) == "0"
	c.ReadSyncOff = mergedEnv("HOTLINE_READ_SYNC"+suffix, dotEnv) == "0"
	// App inbound coalescer + typing hold gate (typing-signal design phase 1).
	// Default ON; only an explicit "0" disables. Positive sense (see the Config
	// field doc): a loaded production config coalesces, the zero-value test Config
	// does not.
	c.InboundCoalesce = mergedEnv("HOTLINE_APP_COALESCE"+suffix, dotEnv) != "0"
	// HOTLINE_APP_COALESCE_WINDOW overrides the coalescer window+grace (unified).
	// Invalid input logs and leaves the built-in default (zero ⇒ default).
	if raw := strings.TrimSpace(mergedEnv("HOTLINE_APP_COALESCE_WINDOW"+suffix, dotEnv)); raw != "" {
		if d, ok := parseDurationOrSeconds(raw); ok {
			c.AppCoalesceWindow = d
		} else {
			fmt.Fprintf(os.Stderr, "hotline: invalid HOTLINE_APP_COALESCE_WINDOW %q; using default\n", raw)
		}
	}
	c.Static = mergedEnv("APP_ACCESS_MODE"+suffix, dotEnv) == "static"

	// HOTLINE_SUPERVISOR_DIR is exported by the `hotline up` supervisor onto its
	// harness (and inherited by this MCP grandchild) — it is a live ambient env
	// var, NOT a .env setting, so it is read from the real environment only and
	// is intentionally not suffixed per instance. Reading it HERE (the CLI
	// composition boundary) instead of inside internal/app.NewServer is the
	// whole fix: a bare `go test ./internal/app/` no longer inherits a path to
	// the live supervisor and can no longer file a real restart.request that
	// bounces the operator's session.
	c.SupervisorDir = os.Getenv(supervise.EnvDir)

	return c, nil
}

// parseDurationOrSeconds accepts a Go duration string ("3s", "1500ms") or a bare
// positive integer count of seconds ("3"). It reports ok=false for anything that
// parses as neither or resolves to a non-positive duration, so the caller keeps
// its default.
func parseDurationOrSeconds(raw string) (time.Duration, bool) {
	if d, err := time.ParseDuration(raw); err == nil {
		return d, d > 0
	}
	if n, err := strconv.Atoi(raw); err == nil {
		d := time.Duration(n) * time.Second
		return d, d > 0
	}
	return 0, false
}
