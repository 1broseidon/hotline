package mcpchan

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"os/exec"
	"regexp"
	"strings"
	"time"
)

// exposure is a pluggable backend that makes the loopback static server publicly
// (or, for the local backend, locally) reachable. hotline follows the "operator
// configures, agent consumes" pattern: the in-process http server is the stable
// core, and how it becomes reachable is an operator-selected backend.
type exposure interface {
	// expose makes the loopback server at port reachable and returns the URL to
	// share. cmd is the subprocess backing the tunnel, tracked so it can be torn
	// down; it is nil for backends that spawn no process (the local backend).
	expose(ctx context.Context, port int) (url string, cmd *exec.Cmd, err error)
	// name is the canonical backend name (matches HOTLINE_PUBLISH_EXPOSURE).
	name() string
	// public reports whether expose returns a URL reachable from the public
	// internet. It drives the honesty of the message the tool returns.
	public() bool
}

// newExposure maps a canonical, already-validated backend name (see
// config.PublishExposure) to its implementation. Unrecognized names fall back to
// the safe default so a construction-time surprise can never leave publish with
// a nil backend; validation and loud errors happen earlier in config.
func newExposure(name string) exposure {
	switch name {
	case "cloudflared":
		return cloudflaredExposure{}
	case "local":
		return localExposure{}
	case "localhostrun":
		return localhostRunExposure{}
	default:
		return localhostRunExposure{}
	}
}

// resolveExposure validates a raw backend name and returns its implementation.
// It is the single source of truth for the valid set and is used by tests; the
// running server resolves the name in config and constructs via newExposure.
func resolveExposure(name string) (exposure, error) {
	switch strings.ToLower(strings.TrimSpace(name)) {
	case "", "localhostrun":
		return localhostRunExposure{}, nil
	case "cloudflared":
		return cloudflaredExposure{}, nil
	case "local", "off":
		return localExposure{}, nil
	default:
		return nil, fmt.Errorf("unknown exposure %q (supported: localhostrun, cloudflared, local/off)", name)
	}
}

// localExposure is the no-tunnel backend: it returns the loopback URL and a nil
// cmd. For operators who expose the server themselves (a reverse proxy, an SSH
// port-forward, or a LAN address). The returned URL is NOT public.
type localExposure struct{}

func (localExposure) name() string { return "local" }
func (localExposure) public() bool { return false }
func (localExposure) expose(_ context.Context, port int) (string, *exec.Cmd, error) {
	return fmt.Sprintf("http://127.0.0.1:%d", port), nil, nil
}

// cloudflaredExposure opens a cloudflared quick tunnel. It is only usable when
// the cloudflared binary is on PATH; when it is missing expose errors (an
// explicit operator choice never silently falls back to another backend).
type cloudflaredExposure struct{}

func (cloudflaredExposure) name() string { return "cloudflared" }
func (cloudflaredExposure) public() bool { return true }
func (cloudflaredExposure) expose(_ context.Context, port int) (string, *exec.Cmd, error) {
	cmd, err := (cloudflaredExposure{}).command(port)
	if err != nil {
		return "", nil, err
	}
	url, err := runTunnelCmd(cmd)
	if err != nil {
		return "", nil, err
	}
	return url, cmd, nil
}

// command builds (but does not start) the cloudflared tunnel process, with
// Pdeathsig set so the tunnel dies with hotline instead of orphaning to init.
func (cloudflaredExposure) command(port int) (*exec.Cmd, error) {
	bin, err := exec.LookPath("cloudflared")
	if err != nil {
		return nil, fmt.Errorf("cloudflared not found on PATH")
	}
	cmd := exec.Command(bin, "tunnel", "--url", fmt.Sprintf("http://localhost:%d", port))
	setPdeathsig(cmd)
	return cmd, nil
}

// localhostRunExposure opens a localhost.run tunnel over ssh, non-interactively,
// and returns the public URL it prints (currently an *.lhr.life address). It is
// the batteries-included default: no account or binary install needed.
type localhostRunExposure struct{}

func (localhostRunExposure) name() string { return "localhostrun" }
func (localhostRunExposure) public() bool { return true }
func (localhostRunExposure) expose(_ context.Context, port int) (string, *exec.Cmd, error) {
	cmd := (localhostRunExposure{}).command(port)
	url, err := runTunnelCmd(cmd)
	if err != nil {
		return "", nil, fmt.Errorf("localhost.run tunnel failed: %v", err)
	}
	return url, cmd, nil
}

// command builds (but does not start) the localhost.run ssh process, with
// Pdeathsig set so the tunnel dies with hotline instead of orphaning to init.
func (localhostRunExposure) command(port int) *exec.Cmd {
	cmd := exec.Command("ssh",
		"-o", "StrictHostKeyChecking=accept-new",
		"-o", "UserKnownHostsFile=/dev/null",
		"-o", "ServerAliveInterval=30",
		"-R", fmt.Sprintf("80:localhost:%d", port),
		"nokey@localhost.run",
	)
	setPdeathsig(cmd)
	return cmd
}

// tunnelURLRe matches the public URL a tunnel provider prints for the tunnel
// itself. It is deliberately scoped to the real tunnel hosts — localhost.run's
// free tunnels resolve to *.lhr.life and cloudflared quick tunnels to
// *.trycloudflare.com — because localhost.run's welcome banner is full of decoy
// https links (twitter.com, admin.localhost.run, docs pages) that a generic
// "first https URL" match would grab instead.
var tunnelURLRe = regexp.MustCompile(`https://[a-zA-Z0-9-]+\.(?:lhr\.life|trycloudflare\.com)\b`)

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
