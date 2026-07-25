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
	"time"
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

	// StateRoot is the box-owned root where cross-provider stores such as
	// schedules.json and loops.json live. LoadApp defaults it to the shared base
	// for standalone callers; runChannel replaces it with the resolved BoxRoot.
	// Other loaders leave it empty and consumers fall back to StateDir.
	StateRoot string

	// SupervisorDir is the running `hotline up` supervisor's control dir
	// (HOTLINE_SUPERVISOR_DIR), populated by LoadApp from the ambient
	// environment at the CLI/composition boundary. It gates room-rotation
	// restart requests: a non-empty value means a supervisor is present and
	// the connector may ask it to recycle the harness. It is DELIBERATELY read
	// only here, never inside internal/app: reading the ambient env inside
	// NewServer made a bare `go test ./internal/app/` on a supervised operator
	// box file a real restart.request into the live supervisor dir, bouncing
	// the operator's live session. The zero-value (test) Config leaves it empty
	// and inert. Empty for an unsupervised `hotline run`.
	SupervisorDir string

	// Token is the TELEGRAM_BOT_TOKEN after merging the real environment with
	// the .env file. Empty if unset (the channel still runs the MCP handshake,
	// but skips the poller and tools report "no bot token configured").
	Token string

	// Static is true when TELEGRAM_ACCESS_MODE == "static": access.json is
	// snapshotted and pairing writes are disabled.
	Static bool

	// SignalDaemonURL and SignalAccount are set by LoadSignal only: the base
	// URL of the local signal-cli HTTP daemon and the account's E.164 number.
	// Both are empty for telegram/discord configs.
	SignalDaemonURL string
	SignalAccount   string

	// AppBind, AppToken, and AppAllowAny are set by LoadApp only: the TCP
	// bind address for the app-channel WebSocket server, an optional static
	// token that bypasses pairing (token-mode auth), and the opt-in that
	// permits binding a wide-open address (0.0.0.0 / ::). Empty/false for
	// every other provider.
	AppBind     string
	AppToken    string
	AppAllowAny bool

	// AppRelay and AppRelayToken are set by LoadApp only: AppRelay is an
	// optional ws(s)://host/b/<16-lowercase-hex> endpoint that the app channel
	// dials in addition to serving its LAN listener, and AppRelayToken
	// authenticates that outbound dial and fixed-origin blob HTTP requests.
	AppRelay      string
	AppRelayToken string

	// AppPush and AppPushToken are set by LoadApp only: AppPush (HOTLINE_APP_PUSH=1)
	// enables the remote push sender (default off), and AppPushToken
	// (HOTLINE_APP_PUSH_TOKEN) is an optional Expo access token sent as a Bearer
	// header when talking to the Expo push service (enhanced auth). Empty/false
	// for every other provider.
	AppPush      bool
	AppPushToken string

	// AppPushEndpoint is set by LoadApp only (HOTLINE_PUSH_ENDPOINT). When set to
	// a non-empty base URL (e.g. https://push.hotline.dev) it switches the app
	// channel from the legacy Expo push service to the signed hotline push
	// gateway: the box generates/persists a P-256 key, registers device tokens
	// via challenge/complete, and signs every /v1/push. Unset ⇒ the legacy Expo
	// path is used unchanged. Empty for every other provider.
	AppPushEndpoint string

	// APNsKeyFile, APNsKeyID, APNsTeamID, APNsTopic, and APNsEnvironment are set
	// by LoadApp only. Together they enable direct APNs ActivityKit updates,
	// independently of the normal Expo/gateway push path. APNsTopic is the base
	// app bundle id; the liveactivity suffix is added by the sender. Environment
	// defaults to production and also accepts sandbox.
	APNsKeyFile     string
	APNsKeyID       string
	APNsTeamID      string
	APNsTopic       string
	APNsEnvironment string

	// CoreMode / CoreURL are set by LoadApp only (core-v1 SPEC §5). CoreMode
	// (HOTLINE_CORE_MODE=1) routes the box through hotline-core: room registration
	// on start/new-link, device-token forwarding, wake hints instead of
	// Expo-direct, and e=1 envelope QRs. Unset ⇒ byte-for-byte today's behavior
	// (the rollback contract). CoreURL (HOTLINE_CORE_URL) is the control-plane base
	// URL, default https://relay.hotline.dev, overridable for dev/soak.
	CoreMode bool
	CoreURL  string

	// PushPreviewClear (HOTLINE_PUSH_PREVIEW=clear) is a box-owner opt-in: in core
	// mode the wake hint carries the message plaintext (truncated) and the core
	// uses it as the notification body instead of the generic "New message".
	// Unset or any other value ⇒ generic previews, wire-identical to today. The
	// mailbox E2E envelope is untouched; only this preview snippet transits
	// readable, by explicit choice.
	PushPreviewClear bool

	// UnifiedChatOff (HOTLINE_UNIFIED_CHAT=0) disables the multi-device chat_id
	// unification. Default (unset) is ON: an app device_send is stamped
	// chat_id="app" (one conversation across every device) with user/user_id
	// still the originating deviceID. Setting the env to 0 restores the legacy
	// per-device chat_id (chat_id=deviceID) for anyone who scripted against it.
	// The "off" sense is stored so the zero value is the default-on behavior.
	UnifiedChatOff bool

	// ReadSyncOff (HOTLINE_READ_SYNC=0) disables box read-state sync (the `read`
	// frame, max-merge, fan, and post-drain snapshot). Default (unset) is ON.
	// Read-state is inert when unused, but the kill switch exists for soak
	// hygiene. The "off" sense is stored so the zero value is default-on.
	ReadSyncOff bool

	// InboundCoalesce (HOTLINE_APP_COALESCE != 0) enables the app-channel inbound
	// coalescer + typing hold gate: a texting burst lands as ONE SendChannel per
	// harness instead of one-per-bubble (typing-signal design phase 1). Load()
	// sets it ON by default; only an explicit "0" disables. UNLIKE the other app
	// switches this stores the POSITIVE sense, so the zero-value Config (the test
	// idiom) leaves it OFF: coalescing turns synchronous 1:1 delivery into
	// windowed/merged delivery, and the existing synchronous unit tests assert the
	// legacy contract — they keep exercising the (unchanged) accept path, while
	// the coalescer's own tests enable it explicitly.
	InboundCoalesce bool

	// AppCoalesceWindow (HOTLINE_APP_COALESCE_WINDOW) overrides the app-channel
	// inbound coalescer's quiet window AND its complete-looking grace (the two were
	// deliberately unified). Accepts a Go duration ("3s", "1500ms") or a bare
	// integer count of seconds ("3"). Zero (unset or invalid) ⇒ the built-in
	// default. It does NOT touch HOTLINE_APP_COALESCE=0, the kill switch.
	AppCoalesceWindow time.Duration
}

// DefaultCoreURL is the control-plane base URL a box registers/wakes against when
// HOTLINE_CORE_URL is unset (core-v1 SPEC §5).
const DefaultCoreURL = "https://relay.hotline.dev"

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
// $TELEGRAM_STATE_DIR, then ${XDG_CONFIG_HOME:-~/.config}/hotline. On first
// resolve of the default path, state left at the pre-rename
// ~/.claude/channels/tele-go location is copied over (the old dir is left in
// place; see migrateState).
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

// StateRoot returns the shared state base dir (the one holding .env and every
// provider's per-instance state). Callers that need a base-dir file outside
// any one provider's config — like the global HOTLINE.md voice — use this.
func StateRoot() (string, error) {
	return resolveStateDir()
}

// AnthropicEnvKeys is the allowlist of keys carried from the shared .env into
// the Claude Code child process to point it at an alternate Anthropic-compatible
// provider. Only these keys ever cross over — never the whole .env (which holds
// TELEGRAM_BOT_TOKEN and other credentials that claude has no business seeing).
//
// The list is deliberately broader than the `hotline setup` flags: setup writes
// the common four (base URL, the two auth forms, the primary model), but a power
// user can hand-add any of these to the .env — the per-role model overrides
// (Claude Code's ANTHROPIC_SMALL_FAST_MODEL is deprecated in favor of the
// per-role ANTHROPIC_DEFAULT_*_MODEL vars), custom headers, request timeout, and
// the tool-search toggle (a non-Anthropic base URL disables Claude Code's MCP
// tool search; ENABLE_TOOL_SEARCH=true restores it) — and have it injected too.
var AnthropicEnvKeys = []string{
	"ANTHROPIC_BASE_URL",
	"ANTHROPIC_AUTH_TOKEN",
	"ANTHROPIC_API_KEY",
	"ANTHROPIC_MODEL",
	"ANTHROPIC_DEFAULT_OPUS_MODEL",
	"ANTHROPIC_DEFAULT_SONNET_MODEL",
	"ANTHROPIC_DEFAULT_HAIKU_MODEL",
	"ANTHROPIC_SMALL_FAST_MODEL", // deprecated by Claude Code; kept for back-compat passthrough
	"ANTHROPIC_CUSTOM_HEADERS",
	"API_TIMEOUT_MS",
	"ENABLE_TOOL_SEARCH",
}

// AnthropicChildEnv appends the operator's alternate-provider settings from the
// shared .env onto base and returns the augmented environment for the Claude
// Code child process. Only the allowlisted AnthropicEnvKeys are injected, and
// only when the key is NOT already set in the real process environment — the
// real environment wins, matching hotline's convention everywhere else and
// letting an operator override any key for a single run. base is not mutated in
// place beyond append's normal semantics; callers pass a slice they own (e.g.
// os.Environ()).
func AnthropicChildEnv(base []string) ([]string, error) {
	baseDir, err := resolveStateDir()
	if err != nil {
		return nil, err
	}
	envFile := filepath.Join(baseDir, ".env")
	dotEnv, err := loadDotEnv(envFile)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", envFile, err)
	}
	out := base
	for _, k := range AnthropicEnvKeys {
		if _, real := os.LookupEnv(k); real {
			continue // real environment already carries it and wins
		}
		if v, ok := dotEnv[k]; ok && v != "" {
			out = append(out, k+"="+v)
		}
	}
	return out, nil
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
	newDir, err := defaultStateDir()
	if err != nil {
		return "", err
	}
	oldDir, err := legacyStateDir()
	if err != nil {
		return "", err
	}
	if err := migrateState(oldDir, newDir); err != nil {
		return "", fmt.Errorf("migrating state from %s to %s: %w", oldDir, newDir, err)
	}
	return newDir, nil
}

// defaultStateDir is the hotline-owned default:
// ${XDG_CONFIG_HOME:-~/.config}/hotline.
func defaultStateDir() (string, error) {
	if v := os.Getenv("XDG_CONFIG_HOME"); v != "" {
		return filepath.Join(v, "hotline"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolving home directory: %w", err)
	}
	return filepath.Join(home, ".config", "hotline"), nil
}

// legacyStateDir is the pre-rename default (~/.claude/channels/tele-go),
// where state from tele-go and hotline <= v0.1.0 lives.
func legacyStateDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolving home directory: %w", err)
	}
	return filepath.Join(home, ".claude", "channels", "tele-go"), nil
}

// migrateState copies the legacy state dir to the new default, once. It is a
// no-op when the new dir already exists (no clobber, no re-copy) or when the
// old dir does not exist. The copy preserves file and directory permissions
// and is staged next to the destination then renamed, so a failed copy never
// leaves a half-populated new dir. The old dir is deliberately left in place:
// a still-running older binary may be using it.
func migrateState(oldDir, newDir string) error {
	if _, err := os.Stat(newDir); err == nil {
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	oldInfo, err := os.Stat(oldDir)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	if !oldInfo.IsDir() {
		return nil
	}

	if err := os.MkdirAll(filepath.Dir(newDir), 0o755); err != nil {
		return err
	}
	tmp := newDir + ".migrating"
	if err := os.RemoveAll(tmp); err != nil {
		return err
	}
	if err := copyTree(oldDir, tmp); err != nil {
		_ = os.RemoveAll(tmp)
		return err
	}
	if err := os.Rename(tmp, newDir); err != nil {
		_ = os.RemoveAll(tmp)
		return err
	}
	fmt.Fprintf(os.Stderr, "hotline: migrated state from %s to %s\n", tildify(oldDir), tildify(newDir))
	return nil
}

// copyTree recursively copies src to dst, preserving permissions on regular
// files and directories. Non-regular files (sockets, symlinks, fifos — e.g. a
// stale unix socket in the state dir) are skipped.
func copyTree(src, dst string) error {
	return filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(src, path)
		if err != nil {
			return err
		}
		target := filepath.Join(dst, rel)
		info, err := d.Info()
		if err != nil {
			return err
		}
		if d.IsDir() {
			if err := os.MkdirAll(target, info.Mode().Perm()); err != nil {
				return err
			}
			// MkdirAll perms are subject to umask; pin the exact mode.
			return os.Chmod(target, info.Mode().Perm())
		}
		if !info.Mode().IsRegular() {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if err := os.WriteFile(target, data, info.Mode().Perm()); err != nil {
			return err
		}
		// WriteFile perms are subject to umask; pin the exact mode.
		return os.Chmod(target, info.Mode().Perm())
	})
}

// tildify abbreviates the user's home dir prefix to ~ for display.
func tildify(path string) string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return path
	}
	if path == home {
		return "~"
	}
	if strings.HasPrefix(path, home+string(filepath.Separator)) {
		return "~" + path[len(home):]
	}
	return path
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
