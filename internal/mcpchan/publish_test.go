package mcpchan

import (
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// TestSafePublishPathAllowsCleanArtifact accepts a plain temp directory holding
// only an artifact file — the intended happy path.
func TestSafePublishPathAllowsCleanArtifact(t *testing.T) {
	// Move cwd somewhere unrelated so the artifact dir is never an ancestor of
	// the working directory.
	t.Chdir(t.TempDir())

	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "index.html"), "<h1>hi</h1>")

	if err := safePublishPath(dir); err != nil {
		t.Fatalf("clean artifact dir should be allowed, got: %v", err)
	}
	// A single file inside a clean dir is also fine (its parent is served).
	if err := safePublishPath(filepath.Join(dir, "index.html")); err != nil {
		t.Fatalf("clean artifact file should be allowed, got: %v", err)
	}
}

func TestSafePublishPathRefusesRoot(t *testing.T) {
	if err := safePublishPath(string(filepath.Separator)); err == nil {
		t.Fatal("expected refusal for filesystem root")
	}
}

func TestSafePublishPathRefusesHome(t *testing.T) {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		t.Skip("no home dir")
	}
	if err := safePublishPath(filepath.Clean(home)); err == nil {
		t.Fatal("expected refusal for home directory")
	}
}

func TestSafePublishPathRefusesCwdAndAncestors(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "a", "b")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	// EvalSymlinks so the cwd we compare against matches (macOS /var etc).
	real, err := filepath.EvalSymlinks(sub)
	if err != nil {
		t.Fatal(err)
	}
	t.Chdir(real)

	// The cwd itself, a parent, and the temp root are all ancestors-or-equal.
	for _, p := range []string{real, filepath.Dir(real), dir} {
		if err := safePublishPath(p); err == nil {
			t.Errorf("expected refusal for cwd/ancestor %q", p)
		}
	}
}

func TestSafePublishPathRefusesSensitiveEntries(t *testing.T) {
	t.Chdir(t.TempDir())

	for _, entry := range []string{".git", ".env", ".env.production", "node_modules", "id_rsa", ".ssh"} {
		dir := t.TempDir()
		target := filepath.Join(dir, entry)
		if strings.HasPrefix(entry, ".env") || entry == "id_rsa" {
			writeFile(t, target, "secret")
		} else {
			if err := os.Mkdir(target, 0o755); err != nil {
				t.Fatal(err)
			}
		}
		if err := safePublishPath(dir); err == nil {
			t.Errorf("expected refusal for dir containing %q", entry)
		}
	}
}

func TestSafePublishPathRefusesMissing(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "nope")
	if err := safePublishPath(missing); err == nil {
		t.Fatal("expected refusal for a path that does not exist")
	}
}

// TestResolveExposureSelection covers the backend selection rule: the empty
// value and "localhostrun" both resolve to the default localhost.run backend,
// the aliases map to their backends, and an unknown value errors.
func TestResolveExposureSelection(t *testing.T) {
	for _, tc := range []struct {
		in       string
		wantName string
	}{
		{"", "localhostrun"}, // default
		{"localhostrun", "localhostrun"},
		{"LocalhostRun", "localhostrun"}, // case-insensitive
		{"cloudflared", "cloudflared"},
		{"local", "local"},
		{"off", "local"}, // alias
	} {
		exp, err := resolveExposure(tc.in)
		if err != nil {
			t.Fatalf("resolveExposure(%q) error: %v", tc.in, err)
		}
		if exp.name() != tc.wantName {
			t.Errorf("resolveExposure(%q).name() = %q, want %q", tc.in, exp.name(), tc.wantName)
		}
	}
	if _, err := resolveExposure("bogus"); err == nil {
		t.Fatal("expected error for unknown exposure value")
	}
}

// TestLocalExposureLoopback verifies the local backend returns a loopback URL
// and a nil cmd (no subprocess to track) and reports itself as non-public.
func TestLocalExposureLoopback(t *testing.T) {
	exp := localExposure{}
	if exp.public() {
		t.Fatal("local exposure must not be public")
	}
	url, cmd, err := exp.expose(context.Background(), 12345)
	if err != nil {
		t.Fatalf("local expose error: %v", err)
	}
	if cmd != nil {
		t.Errorf("local expose cmd = %v, want nil", cmd)
	}
	if want := "http://127.0.0.1:12345"; url != want {
		t.Errorf("local expose url = %q, want %q", url, want)
	}
}

// TestPublishLocalBackendMessage runs publish() end to end with the local
// backend (no network) and asserts the returned message is honest: it points at
// the loopback URL and never claims the link is public.
func TestPublishLocalBackendMessage(t *testing.T) {
	t.Chdir(t.TempDir())

	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "index.html"), "<h1>hi</h1>")

	msg, isErr := publish(context.Background(), PublishInput{Path: dir}, localExposure{})
	if isErr {
		t.Fatalf("publish with local backend errored: %s", msg)
	}
	if !strings.Contains(msg, "http://127.0.0.1:") {
		t.Errorf("message missing loopback URL: %q", msg)
	}
	if !strings.Contains(msg, "LOCAL") {
		t.Errorf("message should describe the url as LOCAL: %q", msg)
	}
	if strings.Contains(msg, "PUBLIC") {
		t.Errorf("local backend message must not claim the link is PUBLIC: %q", msg)
	}
	// Clean up the loopback server this test started.
	publishReg.closeAll()
}

// TestPdeathsigSetOnTunnelCommands asserts the tunnel subprocesses are built
// with SysProcAttr (Pdeathsig) on Linux so a tunnel dies with hotline rather
// than orphaning to init. Non-Linux uses the no-op fallback, so skip there.
func TestPdeathsigSetOnTunnelCommands(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("Pdeathsig is Linux-only")
	}
	if cmd := (localhostRunExposure{}).command(4567); cmd.SysProcAttr == nil {
		t.Error("localhost.run ssh command should have SysProcAttr set (Pdeathsig)")
	}
	// cloudflared's command builder also sets Pdeathsig, but only when the
	// binary is present; skip the assertion when it is not installed.
	if cmd, err := (cloudflaredExposure{}).command(4567); err == nil {
		if cmd.SysProcAttr == nil {
			t.Error("cloudflared command should have SysProcAttr set (Pdeathsig)")
		}
	}
}

func TestParseTunnelURL(t *testing.T) {
	for _, tc := range []struct {
		name string
		line string
		want string
		ok   bool
	}{
		{"lhr.life", "  ** your connection id is abc **  https://a1b2c3.lhr.life", "https://a1b2c3.lhr.life", true},
		{"lhr.life tunneled line", "a1b2c3.lhr.life tunneled with tls termination, https://a1b2c3.lhr.life", "https://a1b2c3.lhr.life", true},
		{"trycloudflare", "2024-01-01 INF |  https://random-words-1234.trycloudflare.com  |", "https://random-words-1234.trycloudflare.com", true},
		{"trailing punct", "URL: https://foo.lhr.life.", "https://foo.lhr.life", true},
		{"http only", "http://insecure.example not matched", "", false},
		{"no url", "Warning: Permanently added 'localhost.run' to known hosts.", "", false},
		// Real localhost.run welcome-banner decoys: none of these are the tunnel.
		{"banner twitter decoy", "Follow your favourite reverse tunnel at [https://twitter.com/localhost_run].", "", false},
		{"banner admin decoy", "To set up and manage custom domains go to https://admin.localhost.run/", "", false},
		{"banner docs decoy", "To explore using localhost.run visit the documentation site: https://localhost.run/docs/", "", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := parseTunnelURL(tc.line)
			if ok != tc.ok || got != tc.want {
				t.Fatalf("parseTunnelURL(%q) = (%q,%v), want (%q,%v)", tc.line, got, ok, tc.want, tc.ok)
			}
		})
	}
}
