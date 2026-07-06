package mcpchan

import (
	"os"
	"path/filepath"
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
