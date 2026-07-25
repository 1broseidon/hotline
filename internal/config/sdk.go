package config

import (
	"fmt"
	"os"
	"strconv"
	"strings"
)

// SDKConfig is the per-box knob set for the claude-sdk harness (harness/
// claude-sdk): the Agent SDK session's model, thinking effort, and turn cap.
// Zero values mean "let the SDK default win".
type SDKConfig struct {
	// Model is Agent SDK Options.model — a model id/alias string
	// (e.g. "claude-opus-4-8"); empty = SDK default.
	Model string
	// Effort is the thinking-depth knob: one of low|medium|high|xhigh|max, or
	// a positive integer (raw maxThinkingTokens). The TS harness maps names to
	// token budgets (effortToMaxThinkingTokens); empty = SDK default.
	Effort string
	// MaxTurns is Agent SDK Options.maxTurns; 0 = unlimited. A safety valve,
	// not a rate limiter (hitting it ends the query stream and the supervisor
	// respawn-resumes).
	MaxTurns int
	// SettingSources is the normalized (lowercased, comma-joined, de-duped)
	// HOTLINE_SDK_SETTING_SOURCES value — the Agent SDK Options.settingSources
	// filesystem tiers to load. Empty = [] (hermetic; no user/project/local
	// settings bleed — the Umibozu-safe default). The TS harness re-parses this
	// same string (parseSettingSources, harness/claude-sdk/src/options.ts).
	SettingSources string
}

// sdkSettingSources are the accepted HOTLINE_SDK_SETTING_SOURCES tokens — the
// Agent SDK's SettingSource literal union ('user' | 'project' | 'local', per
// @anthropic-ai/claude-agent-sdk sdk.d.ts). Kept in lockstep with the TS
// harness accept list (SDK_SETTING_SOURCES, harness/claude-sdk/src/options.ts);
// change both together.
var sdkSettingSources = map[string]bool{"user": true, "project": true, "local": true}

// parseSDKSettingSources validates and normalizes a comma-separated
// HOTLINE_SDK_SETTING_SOURCES value: tokens trimmed and lowercased, duplicates
// collapsed, order preserved. An unknown token is a loud error (typo-fails-
// loudly). Empty/blank input yields "" (the hermetic [] default).
func parseSDKSettingSources(raw string) (string, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return "", nil
	}
	var out []string
	seen := map[string]bool{}
	for _, part := range strings.Split(raw, ",") {
		tok := strings.ToLower(strings.TrimSpace(part))
		if tok == "" {
			continue
		}
		if !sdkSettingSources[tok] {
			return "", fmt.Errorf("invalid HOTLINE_SDK_SETTING_SOURCES token %q (want a comma-separated list of user|project|local; unset/empty = none)", strings.TrimSpace(part))
		}
		if !seen[tok] {
			seen[tok] = true
			out = append(out, tok)
		}
	}
	return strings.Join(out, ","), nil
}

// sdkEffortNames are the accepted symbolic HOTLINE_SDK_EFFORT values; the
// token mapping lives in the TS harness (one exported const,
// harness/claude-sdk/src/options.ts) so calibration is tuned in one place.
var sdkEffortNames = map[string]bool{"low": true, "medium": true, "high": true, "xhigh": true, "max": true}

// maxSafeInteger is JavaScript's Number.MAX_SAFE_INTEGER. The TS harness is
// what actually applies these values, and it refuses anything above this
// (Number.isSafeInteger), so accepting more here would only produce a knob the
// box swore was valid and the harness then silently ignored.
const maxSafeInteger = 9007199254740991

// validIntegerKnob is the SHARED integer rule for every numeric knob, matching
// the TS harness byte for byte (sol review #12): DIGITS ONLY, positive, and no
// larger than JavaScript can represent exactly.
//
// strconv.Atoi alone was wider than the harness in two directions that both
// ended in a silent no-op. It accepts a leading sign, so "+5" passed Go's
// validation and then failed the harness's /^[0-9]+$/ — effort fell back to the
// SDK default with nothing said. And it accepts values far above 2^53, where
// Number.isSafeInteger refuses and maxTurns becomes UNLIMITED — the opposite of
// what an operator setting a huge cap intends. A knob the box accepts must be
// one the harness will honour.
func validIntegerKnob(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	n, err := strconv.ParseInt(s, 10, 64)
	return err == nil && n > 0 && n <= maxSafeInteger
}

// ValidSDKEffort reports whether s (already lowercased and trimmed, the way
// LoadSDK normalizes it) is an acceptable HOTLINE_SDK_EFFORT value: one of
// the symbolic names, or a positive integer (raw maxThinkingTokens). This is
// byte-for-byte LoadSDK's acceptance rule, factored out so the app channel's
// set_sdk_config validation (internal/app/sdkconfig.go) can never drift from
// what a subsequent LoadSDK will accept.
func ValidSDKEffort(s string) bool {
	if sdkEffortNames[s] {
		return true
	}
	return validIntegerKnob(s)
}

// LoadSDK resolves the claude-sdk harness knobs from the real environment
// (which wins per key) merged with the shared base-dir .env, mirroring
// LoadPiModel/LoadOpenCode: HOTLINE_SDK_MODEL, HOTLINE_SDK_EFFORT,
// HOTLINE_SDK_MAX_TURNS.
//
// HOTLINE_CLAUDE_SDK_MODEL (the prototype's model env) is honored as a
// deprecated fallback when HOTLINE_SDK_MODEL is unset, with a stderr
// deprecation line — the fallback lives here, in one place; the TS harness
// reads only the canonical HOTLINE_SDK_* names that `up` re-exports.
//
// Validation is loud (matching Harness()'s typo-fails-loudly stance): a bad
// effort or turn cap returns an error naming the env var instead of being
// silently ignored.
func LoadSDK() (SDKConfig, error) { return LoadSDKForBox("") }

// LoadSDKForBox resolves the knobs for ONE box: real environment, then
// <boxRoot>/.env, then the shared base .env (boxenv.go). An empty boxRoot — or
// the default box, whose root IS the base — resolves exactly as LoadSDK always
// did. Named boxes get their own knobs instead of sharing one machine-wide set.
func LoadSDKForBox(boxRoot string) (SDKConfig, error) {
	env, err := loadBoxEnv(boxRoot)
	if err != nil {
		return SDKConfig{}, err
	}

	cfg := SDKConfig{
		Model:  strings.TrimSpace(env.lookup("HOTLINE_SDK_MODEL")),
		Effort: strings.ToLower(strings.TrimSpace(env.lookup("HOTLINE_SDK_EFFORT"))),
	}
	if cfg.Model == "" {
		if legacy := strings.TrimSpace(env.lookup("HOTLINE_CLAUDE_SDK_MODEL")); legacy != "" {
			cfg.Model = legacy
			fmt.Fprintln(os.Stderr, "hotline: HOTLINE_CLAUDE_SDK_MODEL is deprecated; set HOTLINE_SDK_MODEL instead (honoring the old name for now)")
		}
	}
	if cfg.Effort != "" && !ValidSDKEffort(cfg.Effort) {
		return SDKConfig{}, fmt.Errorf("invalid HOTLINE_SDK_EFFORT %q (want low|medium|high|xhigh|max, or a positive integer of maxThinkingTokens)", cfg.Effort)
	}
	if raw := strings.TrimSpace(env.lookup("HOTLINE_SDK_MAX_TURNS")); raw != "" {
		// Same shared rule as the effort knob: digits only, positive, within
		// JavaScript's exact-integer range. The harness's parsePositiveInt
		// silently returns "unlimited" for anything outside it, so a value the
		// box accepts here has to be one the harness will actually apply.
		if !validIntegerKnob(raw) {
			return SDKConfig{}, fmt.Errorf("invalid HOTLINE_SDK_MAX_TURNS %q (want a positive integer of digits only, at most %d; unset = unlimited)", raw, maxSafeInteger)
		}
		n, _ := strconv.ParseInt(raw, 10, 64)
		cfg.MaxTurns = int(n)
	}
	sources, err := parseSDKSettingSources(env.lookup("HOTLINE_SDK_SETTING_SOURCES"))
	if err != nil {
		return SDKConfig{}, err
	}
	cfg.SettingSources = sources
	return cfg, nil
}

// UpdateSDKEnv persists the claude-sdk model/effort knobs into the shared
// state-dir .env — the exact file LoadSDK merges under the real environment
// and the supervisor's per-respawn re-resolution reads (cmd_up's claude-sdk
// start() re-runs LoadSDK and re-exports into the child env). A nil pointer
// leaves that knob's line untouched; a pointer to "" clears the knob back to
// the SDK default by REMOVING its line (an empty `KEY=` line would read as
// set-but-empty and still be a real value to some consumers). Comments and
// unrelated keys are preserved in place; the file stays 0600.
//
// Whenever model is non-nil (set OR cleared), the deprecated
// HOTLINE_CLAUDE_SDK_MODEL line is also removed: LoadSDK honors it as a
// fallback when HOTLINE_SDK_MODEL is unset, so leaving it behind would
// silently resurrect an old model on clear.
//
// Documented caveat, not code: mergedEnv is real-env-wins. If the shell that
// launched `hotline up` exported HOTLINE_SDK_MODEL/_EFFORT, this .env edit is
// shadowed and the post-restart identity still shows the old value — clients
// surface that as an apply mismatch (SPEC.md, SDK-settings amendment).
func UpdateSDKEnv(model, effort *string) error { return UpdateSDKEnvForBox("", model, effort) }

// UpdateSDKEnvForBox persists a confirmed change into THIS box's .env
// (<boxRoot>/.env; the base .env for the default box, so its behaviour is
// unchanged). Writing the shared file for every box is what let one box's model
// change move every other box's on its next respawn.
func UpdateSDKEnvForBox(boxRoot string, model, effort *string) error {
	envFile, err := boxEnvFile(boxRoot)
	if err != nil {
		return err
	}
	updates := map[string]string{}
	var remove []string
	if model != nil {
		if *model == "" {
			remove = append(remove, "HOTLINE_SDK_MODEL")
		} else {
			updates["HOTLINE_SDK_MODEL"] = *model
		}
		remove = append(remove, "HOTLINE_CLAUDE_SDK_MODEL")
	}
	if effort != nil {
		if *effort == "" {
			remove = append(remove, "HOTLINE_SDK_EFFORT")
		} else {
			updates["HOTLINE_SDK_EFFORT"] = *effort
		}
	}
	if len(updates) == 0 && len(remove) == 0 {
		return nil
	}
	return WriteEnvFile(envFile, updates, remove)
}
