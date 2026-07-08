package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/supervise"
)

// startHarness / startHarnessPiped are the seams cmd_up goes through to
// spawn one supervised harness; swapped out in tests so no pty, real claude,
// or real opencode is needed.
var (
	startHarness      = supervise.StartOnPTY
	startHarnessPiped = supervise.StartPiped
)

// cmdUp launches the coding-agent harness under hotline's always-on
// supervisor and restarts it on any exit with exponential backoff, until
// `hotline down`. Which harness comes from HOTLINE_HARNESS, exactly like the
// rest of hotline: claude runs on a supervisor-owned pty (interactive Claude
// Code needs a terminal), `opencode serve` runs headless on plain pipes. By
// default the supervisor detaches from the terminal (re-exec as `up
// --foreground` in a new session, output to supervisor.log); --foreground
// keeps it attached, which is the shape a tmux pane or a systemd unit wants.
func cmdUp(botName string, args, passthrough []string, dir string, stdout, stderr io.Writer) error {
	fs := flag.NewFlagSet("up", flag.ContinueOnError)
	fs.SetOutput(stdout)
	providers := fs.String("providers", "", "comma-separated provider list (exported as HOTLINE_PROVIDERS)")
	yolo := fs.Bool("yolo", false, "start claude with --dangerously-skip-permissions (the permission relay never fires)")
	foreground := fs.Bool("foreground", false, "run the supervisor in this terminal instead of detaching (for tmux/systemd)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	// HOTLINE_HARNESS picks what gets supervised, exactly like `hotline run`:
	// claude (the default) on a supervisor-owned pty, or `opencode serve` on
	// plain pipes — a headless daemon that needs no terminal. On the opencode
	// path the supervised process is the serve daemon; opencode then spawns
	// hotline itself via the project's MCP config, unchanged.
	harnessMode, err := config.Harness()
	if err != nil {
		return err
	}
	if harnessMode == "opencode" && *yolo {
		// --yolo is claude's --dangerously-skip-permissions; opencode has no
		// spawn-flag equivalent — its permission policy lives in opencode.json's
		// "permission" block (and the scaffolded hotline agent file). Silently
		// ignoring the flag would lie about what's running.
		return fmt.Errorf("--yolo maps to claude's --dangerously-skip-permissions and has no opencode equivalent; set the \"permission\" block in opencode.json instead")
	}

	if *providers != "" {
		os.Setenv("HOTLINE_PROVIDERS", *providers)
	}
	if botName != "" {
		os.Setenv("HOTLINE_BOT", botName)
	}

	if harnessMode == "opencode" {
		if _, err := exec.LookPath("opencode"); err != nil {
			return fmt.Errorf("opencode not found on PATH. Install OpenCode first: https://opencode.ai")
		}
	} else if _, err := exec.LookPath("claude"); err != nil {
		return fmt.Errorf("claude not found on PATH. Install Claude Code first: https://claude.com/claude-code")
	}

	stateRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	supDir := supervise.Dir(stateRoot)
	if err := os.MkdirAll(supDir, 0o700); err != nil {
		return err
	}

	if !*foreground {
		if supervise.Running(supDir) {
			return alreadyRunningErr(supDir)
		}
		return detachSupervisor(supDir, *yolo, passthrough, dir, stdout)
	}

	// Foreground: this process IS the supervisor. The held flock is the
	// singleton guard and the liveness signal for status/down.
	release, err := supervise.AcquireLock(supDir)
	if err != nil {
		return alreadyRunningErr(supDir)
	}
	defer release()

	warnMissingCreds(botName, stderr)

	harnessLog, err := supervise.NewRotatingWriter(filepath.Join(supDir, supervise.HarnessLogName), 5<<20)
	if err != nil {
		return err
	}
	defer harnessLog.Close()

	// The harness argv is re-resolved on every spawn (same machinery as
	// `hotline start` / the same config source as `hotline run`), so an
	// `hotline init` fix, a binary upgrade, or an OPENCODE_SERVER_URL change
	// applies on the next restart without bouncing the supervisor.
	start := func(ctx context.Context) (supervise.Harness, error) {
		// HOTLINE_SUPERVISOR_DIR enables the restart MCP tool in the hotline
		// session: claude passes its env to stdio MCP children, and opencode
		// serve does too (verified: process env is merged with opencode.json's
		// explicit environment block).
		env := append(os.Environ(), supervise.EnvDir+"="+supDir)

		if harnessMode == "opencode" {
			bin, err := exec.LookPath("opencode")
			if err != nil {
				return nil, err
			}
			argv, err := opencodeServeArgv(bin, passthrough)
			if err != nil {
				return nil, err
			}
			// Headless daemon: plain pipes, no pty.
			return startHarnessPiped(argv, dir, env, harnessLog)
		}

		bin, err := exec.LookPath("claude")
		if err != nil {
			return nil, err
		}
		argv := append([]string{bin}, channelArgs(dir, stderr)...)
		if *yolo {
			argv = append(argv, "--dangerously-skip-permissions")
		}
		argv = append(argv, passthrough...)
		if os.Getenv("TERM") == "" {
			env = append(env, "TERM=xterm-256color") // detached env has no TERM; the TUI needs one
		}
		return startHarness(argv, dir, env, harnessLog)
	}

	sup := supervise.New(supDir, start)
	sup.Log = stderr
	sup.Argv = supervisedArgvLabel(harnessMode, *yolo, passthrough)
	sup.WorkDir = dir
	loopRunner := loop.NewRunner(stateRoot)
	loopRunner.Log = stderr

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	defer signal.Stop(sigCh)
	go func() {
		for sig := range sigCh {
			if sig == syscall.SIGHUP {
				// SIGHUP = bounce the harness, via the same control file the
				// restart tool uses — one signal path, uniformly logged.
				_ = supervise.RequestRestart(supDir, "SIGHUP")
				continue
			}
			cancel()
			return
		}
	}()

	go func() {
		if err := loopRunner.Run(ctx); err != nil {
			fmt.Fprintf(stderr, "hotline: loop runner exited: %v\n", err)
		}
	}()
	return sup.Run(ctx)
}

// detachSupervisor re-execs this binary as `up --foreground` in a new session
// with stdin from /dev/null and both output streams appended to
// supervisor.log, then returns immediately. Selections already applied to the
// environment (HOTLINE_PROVIDERS, HOTLINE_BOT) ride along via os.Environ.
func detachSupervisor(supDir string, yolo bool, passthrough []string, dir string, stdout io.Writer) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	args := []string{"up", "--foreground"}
	if yolo {
		args = append(args, "--yolo")
	}
	if len(passthrough) > 0 {
		args = append(args, "--")
		args = append(args, passthrough...)
	}

	logPath := filepath.Join(supDir, supervise.SupervisorLogName)
	logF, err := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	defer logF.Close()
	devnull, err := os.OpenFile(os.DevNull, os.O_RDONLY, 0)
	if err != nil {
		return err
	}
	defer devnull.Close()

	cmd := exec.Command(exe, args...)
	cmd.Dir = dir
	cmd.Env = os.Environ()
	cmd.Stdin, cmd.Stdout, cmd.Stderr = devnull, logF, logF
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true} // survive this terminal
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("detaching supervisor: %w", err)
	}
	pid := cmd.Process.Pid
	_ = cmd.Process.Release()

	fmt.Fprintf(stdout, "hotline: supervisor starting (pid %d)\n  log:   %s\n  state: %s\nCheck `hotline status`; stop with `hotline down`.\n",
		pid, logPath, filepath.Join(supDir, "state.json"))
	return nil
}

// cmdDown stops a running supervisor: SIGTERM (its shutdown path stops the
// harness gracefully first), then wait for the flock to free.
func cmdDown(stdout io.Writer) error {
	stateRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	supDir := supervise.Dir(stateRoot)
	if !supervise.Running(supDir) {
		fmt.Fprintln(stdout, "hotline: supervisor not running")
		return nil
	}
	st, err := supervise.ReadState(supDir)
	if err != nil || st == nil || st.PID <= 0 {
		return fmt.Errorf("supervisor is running but %s is unreadable — stop it by pid manually (err: %v)", filepath.Join(supDir, "state.json"), err)
	}
	if err := syscall.Kill(st.PID, syscall.SIGTERM); err != nil {
		return fmt.Errorf("signalling supervisor (pid %d): %w", st.PID, err)
	}
	deadline := time.Now().Add(20 * time.Second)
	for supervise.Running(supDir) {
		if time.Now().After(deadline) {
			return fmt.Errorf("supervisor (pid %d) did not stop within 20s — check %s", st.PID, filepath.Join(supDir, supervise.SupervisorLogName))
		}
		time.Sleep(200 * time.Millisecond)
	}
	fmt.Fprintf(stdout, "hotline: supervisor stopped (pid %d)\n", st.PID)
	return nil
}

// printSupervisorStatus appends the always-on block to `hotline status`.
func printSupervisorStatus(stateRoot string) {
	supDir := supervise.Dir(stateRoot)
	if !supervise.Running(supDir) {
		fmt.Printf("supervisor:  not running (`hotline up` for an always-on session)\n")
		return
	}
	st, err := supervise.ReadState(supDir)
	if err != nil || st == nil {
		fmt.Printf("supervisor:  running (state unreadable: %v)\n", err)
		return
	}
	switch st.Phase {
	case supervise.PhaseBackoff:
		fmt.Printf("supervisor:  running (pid %d) — harness DOWN, retrying at %s\n", st.PID, localTime(st.NextRestartAt))
	default:
		fmt.Printf("supervisor:  running (pid %d) — harness pid %d, up since %s\n", st.PID, st.HarnessPID, localTime(st.HarnessStartedAt))
	}
	if st.Restarts > 0 {
		fmt.Printf("  restarts:  %d (last exit: %s)\n", st.Restarts, st.LastExit)
	}
	fmt.Printf("  logs:      %s\n", supDir)
}

func alreadyRunningErr(supDir string) error {
	pid := "?"
	if st, err := supervise.ReadState(supDir); err == nil && st != nil {
		pid = fmt.Sprint(st.PID)
	}
	return fmt.Errorf("supervisor already running (pid %s) — see `hotline status`, stop with `hotline down`", pid)
}

// supervisedArgvLabel is the display form of what the supervisor runs,
// recorded in state.json (the real argv is re-resolved every spawn).
func supervisedArgvLabel(harnessMode string, yolo bool, passthrough []string) []string {
	if harnessMode == "opencode" {
		return append([]string{"opencode", "serve"}, passthrough...)
	}
	argv := []string{"claude"}
	if yolo {
		argv = append(argv, "--dangerously-skip-permissions")
	}
	return append(argv, passthrough...)
}

// opencodeServeArgv builds the `opencode serve` command line from the same
// config source the hotline MCP child will read (OPENCODE_SERVER_URL,
// default http://127.0.0.1:4096), so the supervised daemon binds exactly
// where hotline's SSE link later connects. Args after -- go to opencode
// serve verbatim.
func opencodeServeArgv(bin string, passthrough []string) ([]string, error) {
	ocfg, err := config.LoadOpenCode()
	if err != nil {
		return nil, err
	}
	host, port, err := opencodeServeAddr(ocfg.ServerURL)
	if err != nil {
		return nil, err
	}
	argv := []string{bin, "serve", "--port", port, "--hostname", host}
	return append(argv, passthrough...), nil
}

// opencodeServeAddr derives the bind host and port from the configured
// server URL. A URL without an explicit port gets the scheme default — the
// port the SSE client will actually dial — so serve and client always agree.
func opencodeServeAddr(serverURL string) (host, port string, err error) {
	u, err := url.Parse(serverURL)
	if err != nil {
		return "", "", fmt.Errorf("parsing OPENCODE_SERVER_URL %q: %w", serverURL, err)
	}
	host = u.Hostname()
	if host == "" {
		return "", "", fmt.Errorf("OPENCODE_SERVER_URL %q has no host", serverURL)
	}
	port = u.Port()
	if port == "" {
		if u.Scheme == "https" {
			port = "443"
		} else {
			port = "80"
		}
	}
	return host, port, nil
}
