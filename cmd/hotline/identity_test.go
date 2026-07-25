package main

import (
	"bytes"
	"testing"

	"github.com/1broseidon/hotline/internal/app"
)

// TestEnsureIdentitySeedingPrecedence pins the FB21 seed precedence:
// HOTLINE_ASSISTANT_NAME > bot instance name > one creature roll. Only the roll
// prints flair.
func TestEnsureIdentitySeedingPrecedence(t *testing.T) {
	cases := []struct {
		name    string
		env     string
		botName string
		want    string // "" means "expect a roll from the creature table"
	}{
		{"env beats instance", "from-env", "instname", "from-env"},
		{"instance when no env", "", "instname", "instname"},
		{"roll when nothing set", "", "", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv("HOTLINE_ASSISTANT_NAME", tc.env)
			store, err := app.OpenRelayStore(t.TempDir())
			if err != nil {
				t.Fatalf("open store: %v", err)
			}
			var out bytes.Buffer
			got, err := ensureIdentity(store, tc.botName, &out)
			if err != nil {
				t.Fatalf("ensureIdentity: %v", err)
			}
			if tc.want != "" {
				if got != tc.want {
					t.Fatalf("name = %q, want %q", got, tc.want)
				}
				if out.Len() != 0 {
					t.Fatalf("non-roll seed printed flair: %q", out.String())
				}
				return
			}
			// Rolled: must be a real table entry and must print its flair.
			valid := false
			for _, c := range creatures {
				if c.name == got {
					valid = true
					break
				}
			}
			if !valid {
				t.Fatalf("rolled name %q is not in the creature table", got)
			}
			if out.Len() == 0 {
				t.Fatal("rolled seed printed no flair line")
			}
			// The persisted identity must match what was returned.
			if persisted, ok := store.IdentityName(); !ok || persisted != got {
				t.Fatalf("persisted identity = %q (ok=%v), want %q", persisted, ok, got)
			}
		})
	}
}

// TestEnsureIdentitySeedsOnceNoReroll proves the seed is a one-time event: a
// second boot over the same state dir returns the SAME name, never re-rolls, and
// ignores a now-set env (the durable identity wins).
func TestEnsureIdentitySeedsOnceNoReroll(t *testing.T) {
	dir := t.TempDir()

	// First boot: nothing set → a roll seeds the identity.
	t.Setenv("HOTLINE_ASSISTANT_NAME", "")
	store1, err := app.OpenRelayStore(dir)
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	var out1 bytes.Buffer
	first, err := ensureIdentity(store1, "", &out1)
	if err != nil {
		t.Fatalf("first seed: %v", err)
	}
	if out1.Len() == 0 {
		t.Fatal("first boot did not roll (expected flair)")
	}

	// Second boot: a fresh store over the same dir, env now set to something
	// else. The persisted identity must win — no re-roll, env ignored, no flair.
	t.Setenv("HOTLINE_ASSISTANT_NAME", "different-now")
	store2, err := app.OpenRelayStore(dir)
	if err != nil {
		t.Fatalf("reopen store: %v", err)
	}
	var out2 bytes.Buffer
	second, err := ensureIdentity(store2, "botname", &out2)
	if err != nil {
		t.Fatalf("second seed: %v", err)
	}
	if second != first {
		t.Fatalf("re-seed changed identity: first=%q second=%q", first, second)
	}
	if out2.Len() != 0 {
		t.Fatalf("second boot printed flair (re-rolled?): %q", out2.String())
	}
}
