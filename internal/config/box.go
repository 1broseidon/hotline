package config

import (
	"crypto/sha256"
	"encoding/hex"
	"path/filepath"
	"sort"
	"strings"
)

// Box is one resolved harness identity: Providers is the configured provider
// set and Root is where that box's mutable cross-provider stores live.
type Box struct {
	// Key is the stable box identity used in ownership metadata: "default" for
	// the uninstanced box, the sole instance name, or the same deterministic
	// hash used by the boxes/ layout when several instances are combined.
	Key       string
	Root      string
	Providers []ProviderSpec
}

// ResolveBox resolves the configured provider set and its box-owned state root.
// The default/uninstanced box keeps the exact shared base root for compatibility.
// A single provider instance (or several providers sharing one instance) uses
// the existing bots/<instance> directory. Provider sets with several distinct
// instances get a deterministic isolated directory under boxes/.
func ResolveBox(botName string) (Box, error) {
	specs, err := Providers(botName)
	if err != nil {
		return Box{}, err
	}
	base, err := StateRoot()
	if err != nil {
		return Box{}, err
	}
	root, key := boxIdentityForProviders(base, specs)
	return Box{Key: key, Root: root, Providers: specs}, nil
}

// BoxRoot returns the root for the box selected by botName and
// HOTLINE_PROVIDERS. StateRoot remains the shared base containing .env,
// HOTLINE.md, and provider-specific state.
func BoxRoot(botName string) (string, error) {
	box, err := ResolveBox(botName)
	if err != nil {
		return "", err
	}
	return box.Root, nil
}

func boxRootForProviders(base string, specs []ProviderSpec) string {
	root, _ := boxIdentityForProviders(base, specs)
	return root
}

func boxIdentityForProviders(base string, specs []ProviderSpec) (root, key string) {
	instances := make(map[string]struct{})
	for _, spec := range specs {
		if spec.Instance != "" {
			instances[spec.Instance] = struct{}{}
		}
	}

	switch len(instances) {
	case 0:
		return base, "default"
	case 1:
		for instance := range instances {
			return filepath.Join(base, "bots", instance), instance
		}
	}

	// At this point an instance alone cannot identify the box. Hash the sorted
	// canonical provider names so provider order is immaterial while different
	// provider-to-instance assignments remain isolated.
	names := make([]string, 0, len(specs))
	for _, spec := range specs {
		names = append(names, spec.Name())
	}
	sort.Strings(names)
	sum := sha256.Sum256([]byte(strings.Join(names, "\x00")))
	key = hex.EncodeToString(sum[:])
	return filepath.Join(base, "boxes", key), key
}
