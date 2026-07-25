package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func boxTestState(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", root)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	return root
}

func TestBoxRootDefaultPreservesExactLegacyRootAndMC(t *testing.T) {
	root := boxTestState(t)
	marker := filepath.Join(root, "mc", "INDEX.md")
	if err := os.MkdirAll(filepath.Dir(marker), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(marker, []byte("existing root mission control"), 0o600); err != nil {
		t.Fatal(err)
	}

	box, err := ResolveBox("")
	if err != nil {
		t.Fatal(err)
	}
	got := box.Root
	if got != root {
		t.Fatalf("default BoxRoot = %q, want exact legacy root %q", got, root)
	}
	if box.Key != "default" {
		t.Fatalf("default Box.Key = %q, want default", box.Key)
	}
	// Several uninstanced providers are still the legacy/default box.
	t.Setenv("HOTLINE_PROVIDERS", "telegram,discord,app")
	if got, err := BoxRoot(""); err != nil || got != root {
		t.Fatalf("uninstanced provider box = %q, err=%v; want exact legacy root %q", got, err, root)
	}
	mcCfg, err := MissionControlForBox("pi", "")
	if err != nil {
		t.Fatal(err)
	}
	if mcCfg.Dir != filepath.Join(root, "mc") {
		t.Fatalf("default MC dir = %q, want existing root mc %q", mcCfg.Dir, filepath.Join(root, "mc"))
	}
	if body, err := os.ReadFile(marker); err != nil || string(body) != "existing root mission control" {
		t.Fatalf("existing root MC state changed: body=%q err=%v", body, err)
	}
}

func TestBoxRootNamedBotUsesExistingBotDirectoryWithoutAdoptingRootState(t *testing.T) {
	root := boxTestState(t)
	legacySchedule := filepath.Join(root, "schedules.json")
	if err := os.WriteFile(legacySchedule, []byte(`{"schedules":["legacy"]}`), 0o600); err != nil {
		t.Fatal(err)
	}

	box, err := ResolveBox("Ada")
	if err != nil {
		t.Fatal(err)
	}
	got := box.Root
	want := filepath.Join(root, "bots", "Ada")
	if got != want {
		t.Fatalf("named BoxRoot = %q, want %q", got, want)
	}
	if box.Key != "Ada" {
		t.Fatalf("named Box.Key = %q, want Ada", box.Key)
	}
	if _, err := os.Stat(filepath.Join(got, "schedules.json")); !os.IsNotExist(err) {
		t.Fatalf("named box must not adopt root schedules (err=%v)", err)
	}
}

func TestBoxRootInferredFromProvidersWithoutBot(t *testing.T) {
	root := boxTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Ada")

	got, err := BoxRoot("")
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(root, "bots", "Ada")
	if got != want {
		t.Fatalf("provider-inferred BoxRoot = %q, want %q", got, want)
	}
	mcCfg, err := MissionControlForBox("claude", "")
	if err != nil {
		t.Fatal(err)
	}
	if mcCfg.Dir != filepath.Join(want, "mc") {
		t.Fatalf("provider-inferred MC dir = %q, want %q", mcCfg.Dir, filepath.Join(want, "mc"))
	}
}

func TestBoxRootCommonIdentityAcrossProviders(t *testing.T) {
	root := boxTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "app,telegram:Ada,discord:Ada")

	got, err := BoxRoot("")
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(root, "bots", "Ada"); got != want {
		t.Fatalf("shared-instance BoxRoot = %q, want %q", got, want)
	}
}

func TestBoxRootMultipleIdentitiesIsDeterministicAndIsolated(t *testing.T) {
	root := boxTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Ada,discord:Bob,app")
	firstBox, err := ResolveBox("")
	if err != nil {
		t.Fatal(err)
	}
	first := firstBox.Root
	if !strings.HasPrefix(first, filepath.Join(root, "boxes")+string(os.PathSeparator)) {
		t.Fatalf("ambiguous BoxRoot = %q, want isolated path under %q", first, filepath.Join(root, "boxes"))
	}
	if firstBox.Key != filepath.Base(first) {
		t.Fatalf("multi-instance Box.Key = %q, want path hash %q", firstBox.Key, filepath.Base(first))
	}

	t.Setenv("HOTLINE_PROVIDERS", "app,discord:Bob,telegram:Ada")
	reordered, err := BoxRoot("")
	if err != nil {
		t.Fatal(err)
	}
	if reordered != first {
		t.Fatalf("provider order changed box identity: %q != %q", reordered, first)
	}

	t.Setenv("HOTLINE_PROVIDERS", "app,discord:Ada,telegram:Bob")
	different, err := BoxRoot("")
	if err != nil {
		t.Fatal(err)
	}
	if different == first {
		t.Fatalf("different provider assignment reused box path %q", first)
	}
}
