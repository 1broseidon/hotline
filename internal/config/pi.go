package config

import (
	"strings"
)

// PiModel holds the operator's optional provider/model/thinking selection for
// the Pi harness. Any field may be empty (unset), meaning "leave it to Pi": an
// unset key defers to an explicit `hotline up -- --provider/--model/--thinking`
// passthrough flag, and past that to Pi's own settings.json defaults.
type PiModel struct {
	// Provider is Pi's --provider value (e.g. "anthropic", "openai", "google").
	Provider string
	// Model is Pi's --model value (a pattern or id; supports "provider/id" and
	// an optional ":<thinking>" suffix).
	Model string
	// Thinking is Pi's --thinking level (off, minimal, low, medium, high, xhigh).
	Thinking string
	// Models is Pi's --models value: the comma-separated pattern list that
	// SCOPES the Ctrl+P model cycle (model catalog amendment 2026-07-20). Each
	// entry is a pattern in the same grammar --model takes, plus globs
	// ("anthropic/*", "*sonnet*") and an optional ":<thinking>" suffix. Unset
	// means "no scope": Pi cycles every model that has auth configured.
	//
	// This is the ONE knob behind the app's model row. The box passes it to Pi
	// as --models (so Ctrl+P cycles it) AND re-exports the EFFECTIVE value into
	// the harness env (so the extension resolves the identical list for the
	// catalog it reports to the app) — one list, three surfaces.
	Models string
}

// LoadPiModel resolves the Pi model knob from the shared base-dir .env merged
// with the real environment (which wins), mirroring LoadOpenCode's env-key
// style: HOTLINE_PI_PROVIDER, HOTLINE_PI_MODEL, HOTLINE_PI_THINKING. All three
// are optional; an unset key stays empty so the caller can let a passthrough
// flag or Pi's own defaults take over.
func LoadPiModel() (PiModel, error) { return LoadPiModelForBox("") }

// LoadPiModelForBox resolves the pi knobs for ONE box: real environment, then
// <boxRoot>/.env, then the shared base .env (boxenv.go). The pi counterpart of
// LoadSDKForBox, and per-box for the same reason — these knobs move at runtime
// through the app, so a shared file means one box's model change silently
// retargets every other box on its next respawn.
func LoadPiModelForBox(boxRoot string) (PiModel, error) {
	env, err := loadBoxEnv(boxRoot)
	if err != nil {
		return PiModel{}, err
	}
	return PiModel{
		Provider: strings.TrimSpace(env.lookup("HOTLINE_PI_PROVIDER")),
		Model:    strings.TrimSpace(env.lookup("HOTLINE_PI_MODEL")),
		Thinking: strings.TrimSpace(env.lookup("HOTLINE_PI_THINKING")),
		Models:   NormalizePiModels(env.lookup("HOTLINE_PI_MODELS")),
	}, nil
}

// piModelsMaxPatterns / piModelsMaxPatternLen bound the scope knob so a
// fat-fingered .env can never build a launch line (or a catalog frame) that
// blows past what the wire and Pi's own arg parser handle comfortably. Both are
// far above any real selection — the fleet's lists run 3-8 patterns.
const (
	piModelsMaxPatterns   = 32
	piModelsMaxPatternLen = 64
)

// ParsePiModels splits a HOTLINE_PI_MODELS / --models value into its patterns,
// applying exactly the split Pi's own arg parser applies (comma, then trim) and
// then the bounds above. Blank entries are dropped, so "a,,b" and "a, b" both
// yield ["a","b"]; an over-long pattern or a pattern past the count cap is
// dropped rather than truncated (a truncated glob would silently match the
// WRONG models, which is worse than not matching).
func ParsePiModels(s string) []string {
	var out []string
	for _, part := range strings.Split(s, ",") {
		p := strings.TrimSpace(part)
		if p == "" || len(p) > piModelsMaxPatternLen {
			continue
		}
		out = append(out, p)
		if len(out) >= piModelsMaxPatterns {
			break
		}
	}
	return out
}

// NormalizePiModels re-joins ParsePiModels into the canonical single-line form
// the box puts on the launch line and re-exports to the harness. Normalizing at
// BOTH ends is what makes the three surfaces agree literally: the --models
// argument, the HOTLINE_PI_MODELS the extension reads, and the .env line are
// the same bytes, so a whitespace-sloppy .env can never make the app's catalog
// disagree with what Ctrl+P cycles. Returns "" when nothing survives.
func NormalizePiModels(s string) string { return strings.Join(ParsePiModels(s), ",") }

// piThinkingLevels are Pi's ThinkingLevel literals (@earendil-works/
// pi-agent-core: "off" | "minimal" | "low" | "medium" | "high" | "xhigh" |
// "max"), the accepted HOTLINE_PI_THINKING / --thinking values. Unlike the
// claude-sdk effort knob there is NO raw-integer form — Pi's
// ExtensionAPI.setThinkingLevel takes a level, never a token budget — so
// ValidPiThinking deliberately refuses digits that ValidSDKEffort accepts.
var piThinkingLevels = map[string]bool{
	"off": true, "minimal": true, "low": true, "medium": true,
	"high": true, "xhigh": true, "max": true,
}

// ValidPiThinking reports whether s (already lowercased and trimmed, the way
// the app channel normalizes an effort request) is an acceptable Pi thinking
// level. The pi counterpart of ValidSDKEffort: the app channel's
// set_sdk_config validation for a PI box delegates here so a value the box
// accepts can never be one Pi's setThinkingLevel would reject.
//
// The app's picker offers only the shared five (low|medium|high|xhigh|max);
// "off" and "minimal" are accepted so a level George set in Pi's own TUI round-
// trips through the wire for display instead of being refused as invalid.
func ValidPiThinking(s string) bool { return piThinkingLevels[s] }

// UpdatePiEnv persists the Pi model/thinking knobs into the shared state-dir
// .env — the file LoadPiModel merges under the real environment and the
// supervisor's per-respawn re-resolution reads (cmd_up's pi start() re-runs
// LoadPiModel and rebuilds the --provider/--model/--thinking argv). The pi
// counterpart of UpdateSDKEnv, with the same pointer semantics: nil leaves the
// knob's line untouched, a pointer to "" clears it by REMOVING the line.
//
// Whenever model is non-nil (set OR cleared), HOTLINE_PI_PROVIDER is also
// removed. The model the harness confirms is the CANONICAL "provider/id" form,
// which piModelArgs passes as a bare `--model provider/id` that Pi's
// resolveCliModel infers the provider from; a leftover HOTLINE_PI_PROVIDER
// would be emitted as an explicit --provider that then fights the prefix (an
// explicit provider makes Pi treat "other-provider/id" as a literal pattern
// under that provider and fail to resolve). Removing it keeps the persisted
// selection self-describing — the same reason UpdateSDKEnv drops the
// deprecated HOTLINE_CLAUDE_SDK_MODEL on any model write.
//
// Documented caveat, not code (the pi analogue of UpdateSDKEnv's real-env-wins
// note): piModelArgs is passthrough-wins. A box launched as
// `hotline up -- --model X` ignores this .env edit on its next respawn, so a
// hot apply survives only until the box restarts. Boxes that want a persistent
// remote knob must set HOTLINE_PI_* in the .env and drop the passthrough flags.
func UpdatePiEnv(model, thinking *string) error { return UpdatePiEnvForBox("", model, thinking) }

// UpdatePiEnvForBox persists a confirmed pi change into THIS box's .env.
func UpdatePiEnvForBox(boxRoot string, model, thinking *string) error {
	envFile, err := boxEnvFile(boxRoot)
	if err != nil {
		return err
	}
	updates := map[string]string{}
	var remove []string
	if model != nil {
		if *model == "" {
			remove = append(remove, "HOTLINE_PI_MODEL")
		} else {
			updates["HOTLINE_PI_MODEL"] = *model
		}
		remove = append(remove, "HOTLINE_PI_PROVIDER")
	}
	if thinking != nil {
		if *thinking == "" {
			remove = append(remove, "HOTLINE_PI_THINKING")
		} else {
			updates["HOTLINE_PI_THINKING"] = *thinking
		}
	}
	if len(updates) == 0 && len(remove) == 0 {
		return nil
	}
	return WriteEnvFile(envFile, updates, remove)
}
