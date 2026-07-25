package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/mdp/qrterminal/v3"
	"rsc.io/qr"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

// TestNewLinkWithoutCodeByteIdentical guards that adding the --code flag did not
// perturb the plaintext `hotline relay new-link` output: stdout must be exactly
// the pair URI followed by its QR block (what printLink emits) and nothing else —
// no code line, no extra bytes.
func TestNewLinkWithoutCodeByteIdentical(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	// Pin an explicit name so the room name is not rolled — a rolled name prints
	// a "rolled: ..." banner ahead of the pair link, which would (intentionally)
	// perturb the byte-identical stdout this test guards.
	t.Setenv("HOTLINE_ASSISTANT_NAME", "testbot")

	var out, errb bytes.Buffer
	if err := cmdRelay("", []string{"new-link"}, &out, &errb); err != nil {
		t.Fatalf("new-link: %v", err)
	}

	got := out.String()
	uri := strings.SplitN(got, "\n", 2)[0]
	if !strings.HasPrefix(uri, "hotline://pair?") {
		t.Fatalf("first line is not a pair URI: %q", uri)
	}
	// Re-render exactly what printLink would produce for that URI.
	var want bytes.Buffer
	want.WriteString(uri + "\n")
	qrterminal.GenerateHalfBlock(uri, qr.M, &want)
	if got != want.String() {
		t.Fatalf("new-link output drifted from printLink:\n got %q\nwant %q", got, want.String())
	}
	if strings.Contains(got, "code:") {
		t.Fatalf("plaintext new-link leaked a code line: %q", got)
	}
}

// TestNewLinkCodeRequiresCoreMode verifies the flag errors clearly without core
// mode (design §6.1) rather than silently printing a plaintext URI.
func TestNewLinkCodeRequiresCoreMode(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	// This is specifically the fail-fast non-core contract. Never inherit a
	// developer shell's live core mode and enter the five-minute code flow.
	t.Setenv("HOTLINE_CORE_MODE", "0")

	var out, errb bytes.Buffer
	err := cmdRelay("", []string{"new-link", "--code"}, &out, &errb)
	if err == nil || !strings.Contains(err.Error(), "core relay") {
		t.Fatalf("want core-mode error, got %v", err)
	}
	if out.Len() != 0 {
		t.Fatalf("errored --code still wrote stdout: %q", out.String())
	}
	// SHOULD-FIX 3: the core-mode guard must fail fast BEFORE minting a room, so no
	// orphan open room is left consuming a slot.
	cfg, cerr := config.LoadApp("")
	if cerr != nil {
		t.Fatalf("load app config: %v", cerr)
	}
	store, serr := app.OpenRelayStore(cfg.StateDir)
	if serr != nil {
		t.Fatalf("open relay store: %v", serr)
	}
	if _, ok := store.CurrentRoom(); ok {
		t.Fatal("no-core-mode --code error minted an orphan room")
	}
	if n := len(store.ServedRooms()); n != 0 {
		t.Fatalf("no-core-mode --code error left %d room(s)", n)
	}
}

// TestNewLinkNameFlagWinsOverEnv proves `new-link --name` sets the minted room
// name, beating HOTLINE_ASSISTANT_NAME, and does not print a roll banner (the
// name was explicit, not rolled).
func TestNewLinkNameFlagWinsOverEnv(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	t.Setenv("HOTLINE_ASSISTANT_NAME", "from-env")

	var out, errb bytes.Buffer
	if err := cmdRelay("", []string{"new-link", "--name", "Griffin"}, &out, &errb); err != nil {
		t.Fatalf("new-link --name: %v", err)
	}
	if strings.Contains(out.String(), "rolled:") {
		t.Fatalf("explicit --name should not print a roll banner: %q", out.String())
	}

	cfg, cerr := config.LoadApp("")
	if cerr != nil {
		t.Fatalf("load app config: %v", cerr)
	}
	store, serr := app.OpenRelayStore(cfg.StateDir)
	if serr != nil {
		t.Fatalf("open relay store: %v", serr)
	}
	room, ok := store.CurrentRoom()
	if !ok {
		t.Fatal("--name did not mint a room")
	}
	if room.Name != "Griffin" {
		t.Fatalf("room name = %q, want Griffin (flag must beat env)", room.Name)
	}
}

// TestNewLinkNameFlagDoesNotChangeBoxIdentity proves the FB21 scope: `new-link
// --name` overrides ONLY this link's pre-connect placeholder, never the durable
// box identity (which stays the seeded name).
func TestNewLinkNameFlagDoesNotChangeBoxIdentity(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	t.Setenv("HOTLINE_ASSISTANT_NAME", "from-env")

	var out, errb bytes.Buffer
	if err := cmdRelay("", []string{"new-link", "--name", "Griffin"}, &out, &errb); err != nil {
		t.Fatalf("new-link --name: %v", err)
	}
	cfg, cerr := config.LoadApp("")
	if cerr != nil {
		t.Fatalf("load app config: %v", cerr)
	}
	store, serr := app.OpenRelayStore(cfg.StateDir)
	if serr != nil {
		t.Fatalf("open relay store: %v", serr)
	}
	if name, ok := store.IdentityName(); !ok || name != "from-env" {
		t.Fatalf("box identity = %q (ok=%v), want from-env (unchanged by --name)", name, ok)
	}
}

// TestNewLinkUsesBoxIdentity proves that without --name the minted room carries
// the box identity name (FB21 §2: rooms are no longer independently named).
func TestNewLinkUsesBoxIdentity(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	t.Setenv("HOTLINE_ASSISTANT_NAME", "Wendigo")

	var out, errb bytes.Buffer
	if err := cmdRelay("", []string{"new-link"}, &out, &errb); err != nil {
		t.Fatalf("new-link: %v", err)
	}
	cfg, cerr := config.LoadApp("")
	if cerr != nil {
		t.Fatalf("load app config: %v", cerr)
	}
	store, serr := app.OpenRelayStore(cfg.StateDir)
	if serr != nil {
		t.Fatalf("open relay store: %v", serr)
	}
	room, ok := store.CurrentRoom()
	if !ok || room.Name != "Wendigo" {
		t.Fatalf("room name = %q (ok=%v), want Wendigo (box identity)", room.Name, ok)
	}
}

// TestNewLinkCodeRotateAllMutuallyExclusive guards the guard.
func TestNewLinkCodeRotateAllMutuallyExclusive(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")

	var out, errb bytes.Buffer
	err := cmdRelay("", []string{"new-link", "--code", "--rotate-all"}, &out, &errb)
	if err == nil || !strings.Contains(err.Error(), "mutually exclusive") {
		t.Fatalf("want mutual-exclusion error, got %v", err)
	}
}
