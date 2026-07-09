package main

import (
	"bufio"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"github.com/1broseidon/hotline/internal/config"
)

// telegramTokenRe matches the shape BotFather hands out: numeric bot id, a
// colon, then a 30+ char secret.
var telegramTokenRe = regexp.MustCompile(`^\d+:[A-Za-z0-9_-]{30,}$`)

// e164Re matches an international phone number: +, then 7-15 digits.
var e164Re = regexp.MustCompile(`^\+[1-9]\d{6,14}$`)

func validTelegramToken(s string) bool { return telegramTokenRe.MatchString(s) }
func validE164(s string) bool          { return e164Re.MatchString(s) }

// telegramTokenKey is the .env key for the selected bot: TELEGRAM_BOT_TOKEN for
// the default bot, TELEGRAM_BOT_TOKEN_<NAME> for a named one.
func telegramTokenKey(botName string) string {
	if botName == "" {
		return "TELEGRAM_BOT_TOKEN"
	}
	return "TELEGRAM_BOT_TOKEN_" + strings.ToUpper(botName)
}

// maskSecret keeps just enough of a credential to recognize it.
func maskSecret(v string) string {
	if v == "" {
		return "(not set)"
	}
	if len(v) <= 8 {
		return strings.Repeat("*", len(v))
	}
	return v[:4] + "…" + v[len(v)-4:]
}

// cmdSetup writes global credentials into <stateDir>/.env. Values come from
// flags; when stdin is a TTY, a missing required value is prompted for instead
// of erroring.
func cmdSetup(botName string, args []string, stdin io.Reader, stdout io.Writer, isTTY bool) error {
	fs := flag.NewFlagSet("setup", flag.ContinueOnError)
	fs.SetOutput(stdout)
	telegramToken := fs.String("telegram-token", "", "Telegram bot token from @BotFather")
	signalAccount := fs.String("signal-account", "", "linked Signal account (E.164, e.g. +15551234567)")
	signalDaemonURL := fs.String("signal-daemon-url", "", "signal-cli HTTP daemon URL")
	discordToken := fs.String("discord-token", "", "Discord bot token")
	anthropicBaseURL := fs.String("anthropic-base-url", "", "alternate Anthropic-compatible API base URL (ANTHROPIC_BASE_URL)")
	anthropicToken := fs.String("anthropic-token", "", "bearer token for the alternate provider (ANTHROPIC_AUTH_TOKEN)")
	anthropicAPIKey := fs.String("anthropic-api-key", "", "x-api-key for the alternate provider (ANTHROPIC_API_KEY)")
	anthropicModel := fs.String("anthropic-model", "", "model to use with the alternate provider (ANTHROPIC_MODEL)")
	show := fs.Bool("show", false, "print the current config, tokens masked")
	if err := fs.Parse(args); err != nil {
		return err
	}

	envFile, err := stateEnvFile()
	if err != nil {
		return err
	}

	if *show {
		return setupShow(botName, envFile, stdout)
	}

	existing, err := readEnvFile(envFile)
	if err != nil {
		return err
	}

	tokenKey := telegramTokenKey(botName)
	updates := map[string]string{}

	if *telegramToken != "" {
		updates[tokenKey] = *telegramToken
	} else if existing[tokenKey] == "" {
		// The telegram token is the one required value: prompt on a TTY, error
		// otherwise.
		if !isTTY {
			return fmt.Errorf("missing --telegram-token (no %s in %s, and stdin is not a terminal)", tokenKey, envFile)
		}
		v, err := promptLine(stdin, stdout, fmt.Sprintf("Telegram bot token (from @BotFather, stored as %s): ", tokenKey))
		if err != nil {
			return err
		}
		if v == "" {
			return errors.New("no telegram token given")
		}
		updates[tokenKey] = v
	}
	if *signalAccount != "" {
		updates["SIGNAL_ACCOUNT"] = *signalAccount
	}
	if *signalDaemonURL != "" {
		updates["SIGNAL_DAEMON_URL"] = *signalDaemonURL
	}
	if *discordToken != "" {
		updates["DISCORD_BOT_TOKEN"] = *discordToken
	}
	if *anthropicBaseURL != "" {
		updates["ANTHROPIC_BASE_URL"] = *anthropicBaseURL
	}
	if *anthropicToken != "" {
		updates["ANTHROPIC_AUTH_TOKEN"] = *anthropicToken
	}
	if *anthropicAPIKey != "" {
		updates["ANTHROPIC_API_KEY"] = *anthropicAPIKey
	}
	if *anthropicModel != "" {
		updates["ANTHROPIC_MODEL"] = *anthropicModel
	}

	if v, ok := updates[tokenKey]; ok && !validTelegramToken(v) {
		return fmt.Errorf("telegram token doesn't look right (expected <digits>:<30+ chars>, like 123456789:AAAA…)")
	}
	if v, ok := updates["SIGNAL_ACCOUNT"]; ok && !validE164(v) {
		return fmt.Errorf("signal account must be E.164, like +15551234567 (got %q)", v)
	}

	if len(updates) == 0 {
		fmt.Fprintln(stdout, "Nothing to write. Pass --telegram-token, --signal-account, --signal-daemon-url, --discord-token, --anthropic-base-url, --anthropic-token, --anthropic-api-key, or --anthropic-model.")
		return nil
	}

	if err := writeEnvFile(envFile, updates); err != nil {
		return err
	}

	fmt.Fprintf(stdout, "Wrote %s:\n", envFile)
	for _, k := range sortedKeys(updates) {
		fmt.Fprintf(stdout, "  %s=%s\n", k, maskSecret(updates[k]))
	}
	fmt.Fprintln(stdout, "Next: run `hotline init` in the repo you want to text with.")
	return nil
}

// setupShow prints the state dir and every known credential key, masked.
func setupShow(botName string, envFile string, stdout io.Writer) error {
	env, err := readEnvFile(envFile)
	if err != nil {
		return err
	}
	root := filepath.Dir(envFile)
	fmt.Fprintf(stdout, "state dir:   %s\n", root)
	fmt.Fprintf(stdout, "env file:    %s\n", envFile)
	tokenKey := telegramTokenKey(botName)
	fmt.Fprintf(stdout, "%s: %s\n", tokenKey, maskSecret(env[tokenKey]))
	for _, k := range []string{"DISCORD_BOT_TOKEN", "SIGNAL_ACCOUNT", "SIGNAL_DAEMON_URL", "HOTLINE_PROVIDERS"} {
		fmt.Fprintf(stdout, "%s: %s\n", k, maskSecret(env[k]))
	}
	// Alternate Anthropic provider: the two credentials are masked like tokens;
	// the base URL and model are shown in the clear (they aren't secrets). Any
	// other allowlisted key an operator hand-added to the .env (the per-role
	// model overrides, custom headers, timeout, tool-search toggle) falls through
	// to the generic loop below.
	fmt.Fprintf(stdout, "ANTHROPIC_BASE_URL: %s\n", orNotSet(env["ANTHROPIC_BASE_URL"]))
	fmt.Fprintf(stdout, "ANTHROPIC_AUTH_TOKEN: %s\n", maskSecret(env["ANTHROPIC_AUTH_TOKEN"]))
	fmt.Fprintf(stdout, "ANTHROPIC_API_KEY: %s\n", maskSecret(env["ANTHROPIC_API_KEY"]))
	fmt.Fprintf(stdout, "ANTHROPIC_MODEL: %s\n", orNotSet(env["ANTHROPIC_MODEL"]))
	// Any other credential-looking keys (named bots, named instances).
	for _, k := range sortedKeys(env) {
		if k == tokenKey {
			continue
		}
		switch k {
		case "DISCORD_BOT_TOKEN", "SIGNAL_ACCOUNT", "SIGNAL_DAEMON_URL", "HOTLINE_PROVIDERS",
			"ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_MODEL":
			continue
		}
		fmt.Fprintf(stdout, "%s: %s\n", k, maskSecret(env[k]))
	}
	return nil
}

// orNotSet renders a non-secret value for `setup --show`, or a placeholder when
// it is unset.
func orNotSet(v string) string {
	if v == "" {
		return "(not set)"
	}
	return v
}

// stateEnvFile resolves the shared .env path, creating the state dir (0700)
// on the way.
func stateEnvFile() (string, error) {
	root, err := config.StateRoot()
	if err != nil {
		return "", err
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return "", fmt.Errorf("creating %s: %w", root, err)
	}
	return filepath.Join(root, ".env"), nil
}

// readEnvFile parses KEY=VALUE lines from an .env file. Missing file returns
// an empty map.
func readEnvFile(path string) (map[string]string, error) {
	out := make(map[string]string)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return out, nil
		}
		return nil, err
	}
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if k, v, ok := strings.Cut(line, "="); ok {
			out[strings.TrimSpace(k)] = strings.Trim(strings.TrimSpace(v), `"'`)
		}
	}
	return out, nil
}

// writeEnvFile merges updates into the .env file, preserving every existing
// line (comments, blanks, keys it isn't setting) in place. Updated keys keep
// their position; new keys append at the end. The file is written 0600.
func writeEnvFile(path string, updates map[string]string) error {
	var lines []string
	if data, err := os.ReadFile(path); err == nil {
		lines = strings.Split(strings.TrimRight(string(data), "\n"), "\n")
		if len(lines) == 1 && lines[0] == "" {
			lines = nil
		}
	} else if !os.IsNotExist(err) {
		return err
	}

	written := map[string]bool{}
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		k, _, ok := strings.Cut(trimmed, "=")
		if !ok {
			continue
		}
		k = strings.TrimSpace(k)
		if v, has := updates[k]; has {
			lines[i] = k + "=" + v
			written[k] = true
		}
	}
	for _, k := range sortedKeys(updates) {
		if !written[k] {
			lines = append(lines, k+"="+updates[k])
		}
	}

	content := strings.Join(lines, "\n") + "\n"
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		return err
	}
	return os.Chmod(path, 0o600)
}

func sortedKeys(m map[string]string) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// promptLine prints a prompt and reads one trimmed line.
func promptLine(stdin io.Reader, stdout io.Writer, prompt string) (string, error) {
	fmt.Fprint(stdout, prompt)
	r := bufio.NewReader(stdin)
	line, err := r.ReadString('\n')
	if err != nil && line == "" {
		return "", fmt.Errorf("reading input: %w", err)
	}
	return strings.TrimSpace(line), nil
}

// stdinIsTTY reports whether stdin is an interactive terminal.
func stdinIsTTY() bool {
	fi, err := os.Stdin.Stat()
	if err != nil {
		return false
	}
	return fi.Mode()&os.ModeCharDevice != 0
}
