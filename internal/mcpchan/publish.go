package mcpchan

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// PublishInput is the decoded argument set for the publish tool.
type PublishInput struct {
	Path string `json:"path"`
}

// publishSchema is the verbatim InputSchema for the publish tool.
const publishSchema = `{"type":"object","properties":{"path":{"type":"string","description":"Absolute path to the artifact to publish — a directory or a single HTML file. It is served over a local static server and exposed via a temporary public tunnel."}},"required":["path"]}`

// publishURLTimeout bounds how long we wait for the tunnel to print its public
// URL before giving up and killing the process.
const publishURLTimeout = 20 * time.Second

// tunnelURLRe matches the public URL a tunnel provider prints for the tunnel
// itself. It is deliberately scoped to the real tunnel hosts — localhost.run's
// free tunnels resolve to *.lhr.life and cloudflared quick tunnels to
// *.trycloudflare.com — because localhost.run's welcome banner is full of decoy
// https links (twitter.com, admin.localhost.run, docs pages) that a generic
// "first https URL" match would grab instead.
var tunnelURLRe = regexp.MustCompile(`https://[a-zA-Z0-9-]+\.(?:lhr\.life|trycloudflare\.com)\b`)

// publishRegistry keeps published servers and tunnels alive for the lifetime of
// the process. v1 is ephemeral with no unpublish/TTL: entries are added and
// never removed, so nothing here is garbage-collected and ports stay up.
type publishRegistry struct {
	mu      sync.Mutex
	entries []*publishEntry
}

// publishEntry holds the live resources backing one published artifact.
type publishEntry struct {
	url      string
	listener net.Listener
	server   *http.Server
	tunnel   *exec.Cmd
}

// publishReg is the process-wide registry. It is package-level so published
// artifacts survive after the tool handler returns.
var publishReg = &publishRegistry{}

func (r *publishRegistry) add(e *publishEntry) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.entries = append(r.entries, e)
}

// safePublishPath refuses to serve paths that would expose the whole system,
// the user's home, or a source tree. abs must already be absolute and
// symlink-resolved. It returns a descriptive error when the path is unsafe or
// missing, and nil when it is a plausible artifact to publish.
//
// The check is conservative by design: better to refuse a borderline path and
// make the caller point at a clean artifact directory than to tunnel a repo or
// a home directory to the public internet.
func safePublishPath(abs string) error {
	if abs == "" {
		return fmt.Errorf("empty path")
	}
	if !filepath.IsAbs(abs) {
		return fmt.Errorf("path must be absolute: %q", abs)
	}

	info, err := os.Stat(abs)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("path does not exist: %q", abs)
		}
		return fmt.Errorf("cannot stat path %q: %v", abs, err)
	}

	// root is the directory that would actually be served: the target itself
	// when it is a directory, or its parent when it is a single file.
	root := abs
	if !info.IsDir() {
		root = filepath.Dir(abs)
	}
	root = filepath.Clean(root)

	// Never expose the filesystem root or the user's home directory.
	if root == string(filepath.Separator) {
		return fmt.Errorf("refusing to publish the filesystem root")
	}
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		if filepath.Clean(home) == root {
			return fmt.Errorf("refusing to publish the home directory %q", root)
		}
	}

	// Never expose the working directory or any ancestor of it — that is how
	// the whole repo (or more) would leak. root is an ancestor-or-equal of cwd
	// exactly when a relative path from root to cwd does not climb (no "..").
	if cwd, err := os.Getwd(); err == nil {
		if cwd, err := filepath.EvalSymlinks(cwd); err == nil {
			if rel, err := filepath.Rel(root, cwd); err == nil {
				if rel == "." || !strings.HasPrefix(rel, "..") {
					return fmt.Errorf("refusing to publish %q: it is the working directory or a parent of it", root)
				}
			}
		}
	}

	// Refuse when the served directory has obviously sensitive entries at its
	// root — a strong signal it is a source tree or a home, not an artifact.
	if entry, bad := sensitiveEntry(root); bad {
		return fmt.Errorf("refusing to publish %q: it contains a sensitive entry %q at its root", root, entry)
	}

	return nil
}

// sensitiveEntry reports whether dir contains a sensitive file or directory at
// its top level, returning the offending name. It scans one level only.
func sensitiveEntry(dir string) (string, bool) {
	names, err := os.ReadDir(dir)
	if err != nil {
		return "", false
	}
	// Exact-name matches that mark a source tree, home, or credential store.
	exact := map[string]struct{}{
		".git":         {},
		"node_modules": {},
		".ssh":         {},
		"id_rsa":       {},
		"id_rsa.pub":   {},
		".aws":         {},
		".gcloud":      {},
		".npmrc":       {},
		"credentials":  {},
	}
	for _, e := range names {
		name := e.Name()
		if _, ok := exact[name]; ok {
			return name, true
		}
		// .env and any .env.* variant (.env.local, .env.production, ...).
		if name == ".env" || strings.HasPrefix(name, ".env.") {
			return name, true
		}
	}
	return "", false
}

// publish resolves and vets path, starts a static server rooted at the
// artifact, opens a public tunnel to it, and returns the public URL with an
// explicit note that the link is public and temporary. On any failure it
// returns a descriptive message and true (isError).
func publish(ctx context.Context, in PublishInput) (string, bool) {
	if strings.TrimSpace(in.Path) == "" {
		return "publish failed: path is required", true
	}

	abs, err := filepath.Abs(in.Path)
	if err != nil {
		return "publish failed: " + err.Error(), true
	}
	// Resolve symlinks so the guard sees the real target, not an alias.
	if resolved, err := filepath.EvalSymlinks(abs); err == nil {
		abs = resolved
	} else if os.IsNotExist(err) {
		return fmt.Sprintf("publish failed: path does not exist: %q", in.Path), true
	}

	if err := safePublishPath(abs); err != nil {
		return "publish refused: " + err.Error(), true
	}

	// Determine the served root and, for a single file, the path to open.
	root := abs
	var target string
	if info, err := os.Stat(abs); err == nil && !info.IsDir() {
		root = filepath.Dir(abs)
		target = filepath.Base(abs)
	}

	entry, err := startStaticServer(root)
	if err != nil {
		return "publish failed: could not start local server: " + err.Error(), true
	}

	port := entry.listener.Addr().(*net.TCPAddr).Port
	url, err := openTunnel(ctx, port)
	if err != nil {
		_ = entry.server.Close()
		return "publish failed: " + err.Error(), true
	}
	entry.url = url
	publishReg.add(entry)

	if target != "" {
		url = strings.TrimRight(url, "/") + "/" + target
	}

	msg := fmt.Sprintf("Published: %s\n\nThis is a PUBLIC, TEMPORARY link — anyone with the URL can open it, and it stays up only while this session runs. Tell the user that plainly when you share it.", url)
	return msg, false
}

// startStaticServer binds an http.FileServer to an ephemeral loopback port and
// begins serving root. The listener and server are returned in a registry entry
// so the caller can keep them alive.
func startStaticServer(root string) (*publishEntry, error) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	srv := &http.Server{Handler: http.FileServer(http.Dir(root))}
	go func() { _ = srv.Serve(ln) }()
	return &publishEntry{listener: ln, server: srv}, nil
}

// openTunnel exposes localhost:port to the public internet and returns the
// first https URL the provider prints. It prefers cloudflared when the binary
// is on PATH (no account needed for a quick tunnel) and otherwise uses
// localhost.run over ssh. The spawned process is kept in the registry so the
// tunnel stays up; on timeout or early exit the process is killed and an error
// returned.
func openTunnel(ctx context.Context, port int) (string, error) {
	if entry, err := cloudflaredTunnel(port); err == nil {
		return entry, nil
	}
	return localhostRunTunnel(ctx, port)
}

// cloudflaredTunnel is the optional preferred path: it only runs when a
// cloudflared binary is present. It returns an error (so the caller falls back)
// when cloudflared is missing or fails to print a URL in time.
func cloudflaredTunnel(port int) (string, error) {
	bin, err := exec.LookPath("cloudflared")
	if err != nil {
		return "", fmt.Errorf("cloudflared not found")
	}
	cmd := exec.Command(bin, "tunnel", "--url", fmt.Sprintf("http://localhost:%d", port))
	url, err := runTunnelCmd(cmd)
	if err != nil {
		return "", err
	}
	publishReg.add(&publishEntry{url: url, tunnel: cmd})
	return url, nil
}

// localhostRunTunnel opens a localhost.run tunnel over ssh, non-interactively,
// and returns the public URL it prints (currently an *.lhr.life address).
func localhostRunTunnel(ctx context.Context, port int) (string, error) {
	cmd := exec.Command("ssh",
		"-o", "StrictHostKeyChecking=accept-new",
		"-o", "UserKnownHostsFile=/dev/null",
		"-o", "ServerAliveInterval=30",
		"-R", fmt.Sprintf("80:localhost:%d", port),
		"nokey@localhost.run",
	)
	url, err := runTunnelCmd(cmd)
	if err != nil {
		return "", fmt.Errorf("localhost.run tunnel failed: %v", err)
	}
	publishReg.add(&publishEntry{url: url, tunnel: cmd})
	return url, nil
}

// runTunnelCmd starts cmd and scans its stdout and stderr concurrently for the
// tunnel's public URL. The two streams must be read independently, not merged:
// localhost.run prints its URL to stderr while stdout stays open and empty, so a
// serialized read of stdout-then-stderr would block forever. It waits up to
// publishURLTimeout; on timeout, or when both streams end without a URL, it
// kills the process and returns an error. On success the process is left
// running and both pipes keep draining so they never block it.
func runTunnelCmd(cmd *exec.Cmd) (string, error) {
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "", err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return "", err
	}
	if err := cmd.Start(); err != nil {
		return "", err
	}

	// Buffered for both scanners so a goroutine never blocks sending after we
	// have already returned.
	found := make(chan string, 2)
	go scanForURL(stdout, found)
	go scanForURL(stderr, found)

	deadline := time.After(publishURLTimeout)
	ended := 0
	for {
		select {
		case url := <-found:
			if url != "" {
				return url, nil
			}
			// A stream ended without a URL; only give up once both have.
			if ended++; ended >= 2 {
				_ = cmd.Process.Kill()
				return "", fmt.Errorf("tunnel exited before printing a public URL")
			}
		case <-deadline:
			_ = cmd.Process.Kill()
			return "", fmt.Errorf("timed out after %s waiting for the tunnel URL", publishURLTimeout)
		}
	}
}

// scanForURL reads lines from r and sends the first tunnel URL it finds on ch.
// If the stream ends first, it sends "" so the waiter can count it as done.
// After a match it keeps draining r so the pipe never blocks the running
// process.
func scanForURL(r io.Reader, ch chan<- string) {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for sc.Scan() {
		if url, ok := parseTunnelURL(sc.Text()); ok {
			ch <- url
			for sc.Scan() {
			}
			return
		}
	}
	ch <- ""
}

// parseTunnelURL extracts the first https URL from a line of tunnel output,
// trimming trailing punctuation that providers sometimes append. It returns
// false when the line has no URL.
func parseTunnelURL(line string) (string, bool) {
	m := tunnelURLRe.FindString(line)
	if m == "" {
		return "", false
	}
	m = strings.TrimRight(m, ".,;:!)\"'")
	return m, true
}
