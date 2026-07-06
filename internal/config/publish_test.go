package config

import (
	"os"
	"testing"
)

func TestPublishExposureDefaultAndOverride(t *testing.T) {
	withState(t)
	os.Unsetenv("HOTLINE_PUBLISH_EXPOSURE")

	if v, err := PublishExposure(); err != nil || v != "localhostrun" {
		t.Fatalf("default exposure = %q, %v; want localhostrun", v, err)
	}

	for _, tc := range []struct{ in, want string }{
		{"localhostrun", "localhostrun"},
		{"cloudflared", "cloudflared"},
		{"local", "local"},
		{"off", "local"},       // alias
		{"LOCAL", "local"},     // case-insensitive
		{"  local  ", "local"}, // trimmed
	} {
		t.Setenv("HOTLINE_PUBLISH_EXPOSURE", tc.in)
		if v, err := PublishExposure(); err != nil || v != tc.want {
			t.Fatalf("exposure %q = %q, %v; want %q", tc.in, v, err, tc.want)
		}
	}

	t.Setenv("HOTLINE_PUBLISH_EXPOSURE", "bogus")
	if _, err := PublishExposure(); err == nil {
		t.Fatal("expected error for unknown exposure value")
	}
}
