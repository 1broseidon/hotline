package config

import (
	"path/filepath"
	"testing"
)

func TestLoadDiscordDefaultInstance(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("DISCORD_BOT_TOKEN", "tok-abc")

	c, err := LoadDiscord("")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "discord") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.Token != "tok-abc" {
		t.Fatalf("token %q", c.Token)
	}
}

func TestLoadDiscordNamedInstance(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("DISCORD_BOT_TOKEN", "default-tok")
	t.Setenv("DISCORD_BOT_TOKEN_WORK", "work-tok")

	c, err := LoadDiscord("work")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "discord", "instances", "work") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.Token != "work-tok" {
		t.Fatalf("token %q", c.Token)
	}
}

func TestLoadDiscordRejectsBadInstance(t *testing.T) {
	if _, err := LoadDiscord("../evil"); err == nil {
		t.Fatal("bad instance accepted")
	}
}
