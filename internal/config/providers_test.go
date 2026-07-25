package config

import (
	"path/filepath"
	"testing"
)

func TestProvidersDefault(t *testing.T) {
	t.Setenv("HOTLINE_PROVIDERS", "")
	specs, err := Providers("")
	if err != nil {
		t.Fatal(err)
	}
	if len(specs) != 1 || specs[0].Kind != "telegram" || specs[0].Instance != "" {
		t.Fatalf("default should be the sole telegram provider, got %+v", specs)
	}
	if specs[0].Name() != "telegram" {
		t.Fatalf("default name = %q", specs[0].Name())
	}
}

func TestProvidersBotNameFoldsIntoTelegramInstance(t *testing.T) {
	t.Setenv("HOTLINE_PROVIDERS", "")
	specs, err := Providers("work")
	if err != nil {
		t.Fatal(err)
	}
	if len(specs) != 1 || specs[0].Kind != "telegram" || specs[0].Instance != "work" {
		t.Fatalf("--bot work should select the named telegram instance, got %+v", specs)
	}
	if specs[0].Name() != "telegram:work" {
		t.Fatalf("name = %q, want telegram:work", specs[0].Name())
	}
}

func TestProvidersExplicitList(t *testing.T) {
	t.Setenv("HOTLINE_PROVIDERS", "telegram, telegram:beta")
	specs, err := Providers("")
	if err != nil {
		t.Fatal(err)
	}
	if len(specs) != 2 {
		t.Fatalf("got %+v", specs)
	}
	if specs[0].Name() != "telegram" || specs[1].Name() != "telegram:beta" {
		t.Fatalf("names = %q, %q", specs[0].Name(), specs[1].Name())
	}
}

func TestProvidersBotNameConflict(t *testing.T) {
	t.Setenv("HOTLINE_PROVIDERS", "telegram:beta")
	if _, err := Providers("work"); err == nil {
		t.Fatal("--bot with a differently-instanced telegram entry should be rejected")
	}
	// Same instance is not a conflict.
	if _, err := Providers("beta"); err != nil {
		t.Fatalf("matching instance should be fine: %v", err)
	}
}

func TestProviderStateDirMatchesLoaderLayouts(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", root)
	cases := []struct {
		spec ProviderSpec
		want string
	}{
		{ProviderSpec{Kind: "telegram"}, root},
		{ProviderSpec{Kind: "telegram", Instance: "work"}, filepath.Join(root, "bots", "work")},
		{ProviderSpec{Kind: "discord"}, filepath.Join(root, "discord")},
		{ProviderSpec{Kind: "discord", Instance: "work"}, filepath.Join(root, "discord", "instances", "work")},
		{ProviderSpec{Kind: "signal"}, filepath.Join(root, "signal")},
		{ProviderSpec{Kind: "signal", Instance: "work"}, filepath.Join(root, "signal", "instances", "work")},
		{ProviderSpec{Kind: "app"}, filepath.Join(root, "app")},
		{ProviderSpec{Kind: "app", Instance: "work"}, filepath.Join(root, "app", "instances", "work")},
	}
	for _, tc := range cases {
		got, err := ProviderStateDir(tc.spec)
		if err != nil {
			t.Fatalf("ProviderStateDir(%+v): %v", tc.spec, err)
		}
		if got != tc.want {
			t.Errorf("ProviderStateDir(%+v) = %q, want %q", tc.spec, got, tc.want)
		}
	}
	if _, err := ProviderStateDir(ProviderSpec{Kind: "unknown"}); err == nil {
		t.Fatal("unknown provider state layout should fail")
	}
}

func TestProvidersInvalid(t *testing.T) {
	for _, bad := range []string{"tele/gram", "telegram:../evil", "telegram:a b", "telegram,telegram"} {
		t.Setenv("HOTLINE_PROVIDERS", bad)
		if _, err := Providers(""); err == nil {
			t.Errorf("HOTLINE_PROVIDERS=%q should be rejected", bad)
		}
	}
	// Duplicates created by --bot folding are rejected too.
	t.Setenv("HOTLINE_PROVIDERS", "telegram,telegram:work")
	if _, err := Providers("work"); err == nil {
		t.Error("folding --bot into a list that already has that instance should be rejected")
	}
}
