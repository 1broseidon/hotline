package config

import (
	"path/filepath"
	"testing"
	"time"
)

func TestLoadAppDefaultInstance(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("HOTLINE_APP_RELAY", "")
	t.Setenv("HOTLINE_APP_RELAY_TOKEN", "")
	t.Setenv("HOTLINE_APNS_KEY_FILE", "")
	t.Setenv("HOTLINE_APNS_KEY_ID", "")
	t.Setenv("HOTLINE_APNS_TEAM_ID", "")
	t.Setenv("HOTLINE_APNS_TOPIC", "")
	t.Setenv("HOTLINE_APNS_ENVIRONMENT", "")

	c, err := LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "app") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.AppBind != DefaultAppBind {
		t.Fatalf("bind %q, want default %q", c.AppBind, DefaultAppBind)
	}
	if c.AppToken != "" || c.AppAllowAny {
		t.Fatalf("unexpected auth defaults: token=%q allowAny=%v", c.AppToken, c.AppAllowAny)
	}
	if c.AppRelay != "" || c.AppRelayToken != "" {
		t.Fatalf("unexpected relay defaults: url=%q token=%q", c.AppRelay, c.AppRelayToken)
	}
	if c.APNsKeyFile != "" || c.APNsKeyID != "" || c.APNsTeamID != "" || c.APNsTopic != "" {
		t.Fatalf("unexpected APNs credential defaults: %+v", c)
	}
	if c.APNsEnvironment != DefaultAPNsEnvironment {
		t.Fatalf("APNs environment %q, want %q", c.APNsEnvironment, DefaultAPNsEnvironment)
	}
}

func TestLoadAppDefaultRelayEnv(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("HOTLINE_APP_RELAY", "wss://relay.example/b/0123456789abcdef")
	t.Setenv("HOTLINE_APP_RELAY_TOKEN", "relay-default-token")

	c, err := LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if c.AppRelay != "wss://relay.example/b/0123456789abcdef" {
		t.Fatalf("relay %q", c.AppRelay)
	}
	if c.AppRelayToken != "relay-default-token" {
		t.Fatalf("relay token %q", c.AppRelayToken)
	}
}

func TestLoadAppNamedInstanceAndEnv(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("HOTLINE_APP_BIND_WORK", "172.16.30.90:8990")
	t.Setenv("HOTLINE_APP_TOKEN_WORK", "sekret")
	t.Setenv("HOTLINE_APP_ALLOW_ANY_WORK", "1")
	t.Setenv("HOTLINE_APP_RELAY_WORK", "wss://relay.example/b/fedcba9876543210")
	t.Setenv("HOTLINE_APP_RELAY_TOKEN_WORK", "relay-sekret")
	t.Setenv("HOTLINE_APNS_KEY_FILE_WORK", "/run/secrets/AuthKey.p8")
	t.Setenv("HOTLINE_APNS_KEY_ID_WORK", "KEYID12345")
	t.Setenv("HOTLINE_APNS_TEAM_ID_WORK", "TEAMID1234")
	t.Setenv("HOTLINE_APNS_TOPIC_WORK", "dev.hotline.app")
	t.Setenv("HOTLINE_APNS_ENVIRONMENT_WORK", "sandbox")

	c, err := LoadApp("work")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "app", "instances", "work") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.AppBind != "172.16.30.90:8990" {
		t.Fatalf("bind %q", c.AppBind)
	}
	if c.AppToken != "sekret" {
		t.Fatalf("token %q", c.AppToken)
	}
	if !c.AppAllowAny {
		t.Fatal("allowAny should be true")
	}
	if c.AppRelay != "wss://relay.example/b/fedcba9876543210" {
		t.Fatalf("relay %q", c.AppRelay)
	}
	if c.AppRelayToken != "relay-sekret" {
		t.Fatalf("relay token %q", c.AppRelayToken)
	}
	if c.APNsKeyFile != "/run/secrets/AuthKey.p8" || c.APNsKeyID != "KEYID12345" ||
		c.APNsTeamID != "TEAMID1234" || c.APNsTopic != "dev.hotline.app" || c.APNsEnvironment != "sandbox" {
		t.Fatalf("named APNs config not loaded: %+v", c)
	}
}

func TestLoadAppRejectsBadInstance(t *testing.T) {
	if _, err := LoadApp("../evil"); err == nil {
		t.Fatal("bad instance accepted")
	}
}

func TestParseDurationOrSeconds(t *testing.T) {
	tests := []struct {
		in   string
		want time.Duration
		ok   bool
	}{
		{"3s", 3 * time.Second, true},
		{"1500ms", 1500 * time.Millisecond, true},
		{"3", 3 * time.Second, true},     // bare integer ⇒ seconds
		{"0", 0, false},                  // non-positive rejected
		{"0s", 0, false},                 // non-positive rejected
		{"-2s", -2 * time.Second, false}, // negative rejected
		{"abc", 0, false},                // garbage rejected
		{"", 0, false},                   // empty rejected
	}
	for _, tc := range tests {
		got, ok := parseDurationOrSeconds(tc.in)
		if ok != tc.ok || (ok && got != tc.want) {
			t.Fatalf("parseDurationOrSeconds(%q) = (%v, %v), want (%v, %v)", tc.in, got, ok, tc.want, tc.ok)
		}
	}
}

func TestLoadAppCoalesceWindow(t *testing.T) {
	t.Run("unset defaults to zero", func(t *testing.T) {
		t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
		c, err := LoadApp("")
		if err != nil {
			t.Fatal(err)
		}
		if c.AppCoalesceWindow != 0 {
			t.Fatalf("want zero (built-in default), got %v", c.AppCoalesceWindow)
		}
	})
	t.Run("duration string parses", func(t *testing.T) {
		t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
		t.Setenv("HOTLINE_APP_COALESCE_WINDOW", "5s")
		c, err := LoadApp("")
		if err != nil {
			t.Fatal(err)
		}
		if c.AppCoalesceWindow != 5*time.Second {
			t.Fatalf("window %v, want 5s", c.AppCoalesceWindow)
		}
	})
	t.Run("bare seconds parses", func(t *testing.T) {
		t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
		t.Setenv("HOTLINE_APP_COALESCE_WINDOW", "2")
		c, err := LoadApp("")
		if err != nil {
			t.Fatal(err)
		}
		if c.AppCoalesceWindow != 2*time.Second {
			t.Fatalf("window %v, want 2s", c.AppCoalesceWindow)
		}
	})
	t.Run("invalid falls back to default", func(t *testing.T) {
		t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
		t.Setenv("HOTLINE_APP_COALESCE_WINDOW", "nonsense")
		c, err := LoadApp("")
		if err != nil {
			t.Fatal(err)
		}
		if c.AppCoalesceWindow != 0 {
			t.Fatalf("invalid value should leave default zero, got %v", c.AppCoalesceWindow)
		}
	})
}
