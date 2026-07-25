package config

// Per-box knob resolution (sol review #10).
//
// The model/effort knobs used to resolve against StateRoot()/.env for every
// box: LoadSDK/LoadPiModel read it and UpdateSDKEnv/UpdatePiEnv wrote it. So a
// named box changing its model changed what EVERY other box on the machine got
// on its next respawn — settings the app presents as per-box, and which the
// operator has no reason to think are global. Two boxes on one machine could
// silently fight over one model line.
//
// The fix keeps the shared base .env exactly where it belongs (it holds
// credentials — TELEGRAM_BOT_TOKEN, the Anthropic provider keys — which ARE
// machine-wide) and gives the runtime knobs a per-box home:
//
//	real environment      wins, as everywhere else in hotline
//	<boxRoot>/.env        this box's knobs — what a hot apply writes
//	<StateRoot>/.env      the machine-wide default, still readable
//
// The default (uninstanced) box's root IS StateRoot, so for it the two files
// are the same file and behaviour is byte-identical to before. Only named and
// multi-instance boxes change, and they change in the direction the UI already
// promised.

import (
	"fmt"
	"os"
	"path/filepath"
)

// boxEnv is a resolved two-tier .env view for one box.
type boxEnv struct {
	box  map[string]string
	base map[string]string
}

// loadBoxEnv reads the box's own .env and the shared base .env. When boxRoot is
// empty or equal to the base, both tiers are the same map and resolution is
// identical to the old single-file behaviour.
func loadBoxEnv(boxRoot string) (boxEnv, error) {
	base, err := StateRoot()
	if err != nil {
		return boxEnv{}, err
	}
	baseEnv, err := loadDotEnv(filepath.Join(base, ".env"))
	if err != nil {
		return boxEnv{}, fmt.Errorf("reading %s: %w", filepath.Join(base, ".env"), err)
	}
	if boxRoot == "" || boxRoot == base {
		return boxEnv{box: baseEnv, base: baseEnv}, nil
	}
	boxFile := filepath.Join(boxRoot, ".env")
	scoped, err := loadDotEnv(boxFile)
	if err != nil {
		return boxEnv{}, fmt.Errorf("reading %s: %w", boxFile, err)
	}
	return boxEnv{box: scoped, base: baseEnv}, nil
}

// lookup resolves one key: real environment, then this box's .env, then the
// shared base .env.
func (e boxEnv) lookup(key string) string {
	if v, ok := os.LookupEnv(key); ok {
		return v
	}
	if v, ok := e.box[key]; ok {
		return v
	}
	return e.base[key]
}

// boxEnvFile is where a confirmed knob change for this box is written. The
// default box writes the shared base .env, exactly as before; a named box
// writes its own, so its model can never move another box's.
func boxEnvFile(boxRoot string) (string, error) {
	base, err := StateRoot()
	if err != nil {
		return "", err
	}
	root := boxRoot
	if root == "" {
		root = base
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return "", fmt.Errorf("creating %s: %w", root, err)
	}
	return filepath.Join(root, ".env"), nil
}
