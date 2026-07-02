package config

import (
	"fmt"
	"os"
	"strings"
)

// ProviderSpec identifies one configured chat provider. Kind selects the
// adapter ("telegram" is the only one today); Instance optionally selects a
// named instance of it with isolated state and a per-name token. For telegram,
// Instance is exactly the old --bot named-bot semantics: state under
// <baseDir>/bots/<instance> and token TELEGRAM_BOT_TOKEN_<INSTANCE>.
type ProviderSpec struct {
	Kind     string
	Instance string
}

// Name is the provider's source tag: "telegram" for the default instance,
// "telegram:work" for a named one. It keys outbound tool routing and is
// stamped into inbound channel meta as source.
func (s ProviderSpec) Name() string {
	if s.Instance == "" {
		return s.Kind
	}
	return s.Kind + ":" + s.Instance
}

// providerKindRe constrains a provider kind to a simple lowercase word.
var providerKindRe = botNameRe // same alphabet: letters, digits, underscores

// Providers resolves the configured provider list from $HOTLINE_PROVIDERS — a
// comma-separated list of kind[:instance] entries — defaulting to the single
// entry "telegram" when unset, which reproduces the pre-provider behavior
// exactly.
//
// botName folds the legacy --bot / $HOTLINE_BOT selector into provider config:
// a non-empty botName rewrites the bare "telegram" entry to
// "telegram:<botName>", so `hotline --bot work` is shorthand for
// HOTLINE_PROVIDERS=telegram:work. Combining --bot with an explicitly
// instanced telegram entry is ambiguous and rejected.
func Providers(botName string) ([]ProviderSpec, error) {
	raw := os.Getenv("HOTLINE_PROVIDERS")
	if strings.TrimSpace(raw) == "" {
		raw = "telegram"
	}

	var specs []ProviderSpec
	seen := make(map[string]bool)
	for _, entry := range strings.Split(raw, ",") {
		entry = strings.TrimSpace(entry)
		if entry == "" {
			continue
		}
		kind, instance, _ := strings.Cut(entry, ":")
		if !providerKindRe.MatchString(kind) {
			return nil, fmt.Errorf("invalid provider %q in HOTLINE_PROVIDERS", entry)
		}
		if instance != "" && !botNameRe.MatchString(instance) {
			return nil, fmt.Errorf("invalid provider instance %q in HOTLINE_PROVIDERS: use letters, digits, and underscores only", instance)
		}

		spec := ProviderSpec{Kind: kind, Instance: instance}
		if botName != "" && kind == "telegram" {
			if instance != "" && instance != botName {
				return nil, fmt.Errorf("--bot %s conflicts with HOTLINE_PROVIDERS entry %q — use one or the other", botName, entry)
			}
			spec.Instance = botName
		}

		if seen[spec.Name()] {
			return nil, fmt.Errorf("duplicate provider %q in HOTLINE_PROVIDERS", spec.Name())
		}
		seen[spec.Name()] = true
		specs = append(specs, spec)
	}
	if len(specs) == 0 {
		return nil, fmt.Errorf("HOTLINE_PROVIDERS is set but empty")
	}
	return specs, nil
}
