package mcpchan

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
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

// publishRegistry keeps published servers and tunnels alive for the lifetime of
// the process. v1 is ephemeral with no unpublish/TTL: entries are added and
// never removed during normal operation, so nothing is garbage-collected and
// ports stay up. closeAll (invoked at shutdown) tears everything down at once.
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

// closeAll shuts down every published static server and kills every tracked
// tunnel subprocess. It is idempotent (entries are cleared) and safe to call on
// a registry that was never used. Pdeathsig already reaps tunnels when hotline
// exits on Linux; closeAll is the explicit, cross-platform teardown for a
// graceful shutdown and the belt-and-suspenders kill for the http listeners.
func (r *publishRegistry) closeAll() {
	r.mu.Lock()
	entries := r.entries
	r.entries = nil
	r.mu.Unlock()

	for _, e := range entries {
		if e.server != nil {
			_ = e.server.Close()
		}
		if e.tunnel != nil && e.tunnel.Process != nil {
			_ = e.tunnel.Process.Kill()
		}
	}
}

// CloseAllPublished tears down every artifact published this session: it stops
// the loopback servers and kills the tunnel subprocesses. Wire it into the
// graceful-shutdown path (see cmd/hotline: the lifecycle cleanup hook).
func CloseAllPublished() { publishReg.closeAll() }

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
// artifact, and exposes it via the operator-selected backend, returning the URL
// to share. The returned message tells the truth about reachability: the tunnel
// backends say the link is public and temporary; the local backend says it is
// reachable only on this machine (or via the operator's own exposure). On any
// failure it returns a descriptive message and true (isError).
func publish(ctx context.Context, in PublishInput, exp exposure) (string, bool) {
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
	url, cmd, err := exp.expose(ctx, port)
	if err != nil {
		_ = entry.server.Close()
		return "publish failed: " + err.Error(), true
	}
	// One entry now backs the whole publication: the loopback server plus the
	// tunnel subprocess (nil for the local backend). closeAll tears both down.
	entry.url = url
	entry.tunnel = cmd
	publishReg.add(entry)

	if target != "" {
		url = strings.TrimRight(url, "/") + "/" + target
	}

	var note string
	if exp.public() {
		note = "This is a PUBLIC, TEMPORARY link — anyone with the URL can open it, and it stays up only while this session runs. Tell the user that plainly when you share it."
	} else {
		note = "This is a LOCAL url, reachable only on this machine (or via whatever exposure the operator has set up — a proxy, SSH port-forward, or LAN). It is NOT public; don't imply anyone with the link can open it. It stays up only while this session runs."
	}
	return fmt.Sprintf("Published: %s\n\n%s", url, note), false
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
