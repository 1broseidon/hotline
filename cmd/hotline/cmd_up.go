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
	"sort"
	"strconv"
	"strings"
	"syscall"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/supervise"
)

// Process seams swapped out in tests so no pty, detached process, real claude,
// or real opencode is needed.
var (
	startHarness               = supervise.StartOnPTY
	startHarnessPiped          = supervise.StartPiped
	startHarnessPipedStdinOpen = supervise.StartPipedStdinOpen
	detachUpSupervisor         = detachSupervisor
	// newAttacher is the terminal-bridge seam. Tests override it to force the
	// passive path so a live test-process tty never takes the real pty branch
	// and bypasses the startHarness seam (finding 7).
	newAttacher = func(in, out *os.File, logw io.Writer, cancel func()) (terminalAttacher, error) {
		return supervise.NewAttacher(in, out, logw, cancel)
	}
)

// terminalAttacher is the slice of *supervise.Attacher cmdUp depends on, exposed
// as an interface so the newAttacher seam can inject a passive fake in tests.
type terminalAttacher interface {
	TTY() bool
	Start(argv []string, dir string, env []string) (supervise.Harness, error)
	Close() error
}

// cmdUp launches the coding-agent harness under hotline's always-on
// supervisor and restarts it on any exit with exponential backoff, until
// `hotline down`. Which harness comes from HOTLINE_HARNESS, exactly like the
// rest of hotline: claude runs on a supervisor-owned pty (interactive Claude
// Code needs a terminal); `opencode serve`, `pi --mode rpc`, and the
// claude-sdk node harness run headless on plain pipes. The supervisor stays
// attached by default. The headless pi/opencode/claude-sdk harnesses can use
// --background/-d to re-exec the supervisor in a new session with output in
// supervisor.log; Claude stays attended and belongs in an operator-owned tmux
// session. The old --foreground flag remains a deprecated compatibility no-op.
func cmdUp(botName string, botExplicit bool, args, passthrough []string, dir string, stdout, stderr io.Writer) error {
	fs := flag.NewFlagSet("up", flag.ContinueOnError)
	fs.SetOutput(stdout)
	providers := fs.String("providers", "", "comma-separated provider list (exported as HOTLINE_PROVIDERS)")
	harness := fs.String("harness", "", "coding-agent harness to supervise: "+strings.Join(config.HarnessValues, "|")+" (exported as HOTLINE_HARNESS; precedence flag > env > state .env)")
	yolo := fs.Bool("yolo", false, "start claude with --dangerously-skip-permissions (the permission relay never fires); claude-sdk sessions already run bypassPermissions, so there the flag only exports HOTLINE_YOLO=1")
	background := fs.Bool("background", false, "detach a headless pi/opencode/claude-sdk supervisor (Claude: use tmux)")
	detach := fs.Bool("d", false, "alias for --background")
	fs.Bool("foreground", false, "deprecated no-op; up stays attached by default")
	if err := fs.Parse(args); err != nil {
		return err
	}
	harnessFlagSet := false
	fs.Visit(func(f *flag.Flag) {
		if f.Name == "foreground" {
			fmt.Fprintln(stderr, "hotline: --foreground is deprecated and no longer needed; `hotline up` stays attached by default")
		}
		if f.Name == "harness" {
			harnessFlagSet = true
		}
	})
	runInBackground := *background || *detach

	if *providers != "" {
		if err := os.Setenv("HOTLINE_PROVIDERS", *providers); err != nil {
			return fmt.Errorf("setting HOTLINE_PROVIDERS: %w", err)
		}
	}
	if botExplicit && botName == "" {
		if err := os.Unsetenv("HOTLINE_BOT"); err != nil {
			return fmt.Errorf("clearing HOTLINE_BOT: %w", err)
		}
		if err := os.Unsetenv("TELE_GO_BOT"); err != nil {
			return fmt.Errorf("clearing TELE_GO_BOT: %w", err)
		}
	} else if botName != "" {
		if err := os.Setenv("HOTLINE_BOT", botName); err != nil {
			return fmt.Errorf("setting HOTLINE_BOT: %w", err)
		}
	}
	// The MCP child receives the project's entry env/args after the harness
	// starts, so that identity is the final authority for every harness. Adopt it
	// before resolving harness mode or any box path. A conflicting explicit bot
	// refuses instead of reserving one box while the child launches another.
	identity, err := adoptProjectMCPServerEnv(dir, stderr)
	if err != nil {
		return err
	}
	if identity.Found {
		if botExplicit && botName != identity.BotName {
			return fmt.Errorf("--bot %q conflicts with the project MCP hotline entry's effective bot %q; make them match before launching", botName, identity.BotName)
		}
		botName = identity.BotName
	}

	// --harness mirrors --providers: exporting HOTLINE_HARNESS into this
	// process's env is the whole implementation — config.Harness(), the
	// supervised child, and the respawn path all inherit it with zero new
	// plumbing, and the flag takes precedence over the real shell env and the
	// state .env. Exported AFTER MCP-identity adoption so an explicit flag wins
	// over a .mcp.json HOTLINE_HARNESS too, and validated up front (sharing
	// config.NormalizeHarness's four-value switch) so a typo fails loudly here
	// instead of inside config.Harness() below.
	if harnessFlagSet {
		norm, err := config.NormalizeHarness(*harness)
		if err != nil {
			return fmt.Errorf("--harness: %w", err)
		}
		if err := os.Setenv("HOTLINE_HARNESS", norm); err != nil {
			return fmt.Errorf("setting HOTLINE_HARNESS: %w", err)
		}
	}

	// HOTLINE_HARNESS picks what gets supervised, exactly like `hotline run`:
	// claude (the default) on a supervisor-owned pty, or `opencode serve` on
	// plain pipes — a headless daemon that needs no terminal. On the opencode
	// path the supervised process is the serve daemon; opencode then spawns
	// hotline itself via the project's MCP config, unchanged. Resolve with its
	// source so `up` can announce what it just launched and why.
	harnessMode, harnessSource, err := config.HarnessResolved()
	if err != nil {
		return err
	}
	if harnessFlagSet {
		harnessSource = config.HarnessSourceFlag
	}
	if harnessMode == "opencode" && *yolo {
		// --yolo is claude's --dangerously-skip-permissions; opencode has no
		// spawn-flag equivalent — its permission policy lives in opencode.json's
		// "permission" block (and the scaffolded hotline agent file). Silently
		// ignoring the flag would lie about what's running.
		return fmt.Errorf("--yolo maps to claude's --dangerously-skip-permissions and has no opencode equivalent; set the \"permission\" block in opencode.json instead")
	}
	if harnessMode == "pi" && *yolo {
		// Pi has no permission prompts by design (README: "No permission
		// popups"), so harness=pi already runs every tool unguarded — there is
		// nothing for --yolo to skip. Erroring keeps the flag honest rather than
		// implying it changed anything.
		return fmt.Errorf("--yolo maps to claude's --dangerously-skip-permissions; pi has no permission prompts (tools already run unguarded, always), so there is nothing to skip")
	}
	if runInBackground && harnessMode == "claude" {
		return fmt.Errorf("Claude's development-channel consent cannot be answered detached; `hotline up --background` is unavailable for the claude harness. Run it under tmux instead:\n  tmux new -s hotline -- hotline up\nDetach with Ctrl-b d; reattach with `tmux attach -t hotline`.")
	}

	switch harnessMode {
	case "opencode":
		if _, err := exec.LookPath("opencode"); err != nil {
			return fmt.Errorf("opencode not found on PATH. Install OpenCode first: https://opencode.ai")
		}
	case "pi":
		if _, err := exec.LookPath("pi"); err != nil {
			return fmt.Errorf("pi not found on PATH. Install Pi first: https://pi.dev")
		}
	case "claude-sdk":
		if _, err := exec.LookPath("node"); err != nil {
			return fmt.Errorf("node not found on PATH. harness=claude-sdk needs node >= 20")
		}
		if _, err := claudeSDKEntry(); err != nil {
			return err
		}
		// The claude-sdk child runs the injected-harness server branch
		// (uncapped instruction path), so warnVoiceOverflow's budget warning
		// does not apply.
		//
		// Validate the SDK knobs NOW so a typo'd HOTLINE_SDK_EFFORT /
		// HOTLINE_SDK_MAX_TURNS fails `up` loudly at launch instead of inside
		// the supervisor's spawn loop (the spawn branch re-resolves per
		// respawn to pick up .env edits). Resolved against THIS box's root, or
		// the pre-flight check would validate a different file than the one the
		// spawn branch reads (sol review #10).
		preflightRoot, err := config.BoxRoot(botName)
		if err != nil {
			return err
		}
		if _, err := config.LoadSDKForBox(preflightRoot); err != nil {
			return err
		}
	default:
		if _, err := exec.LookPath("claude"); err != nil {
			return fmt.Errorf("claude not found on PATH. Install Claude Code first: https://claude.com/claude-code")
		}
		// Claude is the only harness on the budget-capped instruction path;
		// warn at launch if the resolved HOTLINE.md voice won't fit.
		warnVoiceOverflow(botName, stderr)
	}

	// Stable session id for the supervised `pi --mode rpc` (same role as
	// OPENCODE_SESSION): resolved once and reused on every restart so Pi resumes
	// the same session — `pi --session-id <id>` "uses the exact project session
	// id, creating it if missing". Deterministic from the bot name, so it needs
	// no persistence; HOTLINE_PI_SESSION overrides it.
	piSession := piSessionID(botName)

	box, err := config.ResolveBox(botName)
	if err != nil {
		return err
	}
	boxRoot := box.Root
	supDir := supervise.Dir(boxRoot)
	if err := os.MkdirAll(supDir, 0o700); err != nil {
		return err
	}

	if runInBackground {
		if supervise.Running(supDir) {
			return alreadyRunningErr(supDir)
		}
		return detachUpSupervisor(supDir, *yolo, passthrough, dir, stdout)
	}

	// Attached: this process IS the supervisor. The held flock is the
	// singleton guard and the liveness signal for status/down.
	release, err := supervise.AcquireLock(supDir)
	if err != nil {
		return alreadyRunningErr(supDir)
	}
	defer release()

	mcCfg, err := config.MissionControlForRoot(harnessMode, boxRoot)
	if err != nil {
		return err
	}
	ownerSpec, err := boxOwnerSpec(box, harnessMode, dir, mcCfg)
	if err != nil {
		return err
	}
	ownerReservation, err := lifecycle.ReserveBox(ownerSpec)
	if err != nil {
		return err
	}
	defer ownerReservation.Release()

	warnMissingCreds(botName, stderr)

	// Announce the resolved harness and why, alongside the supervisor's own
	// start line: the whole pain today is "what did it just launch, and why".
	// The trailing clause states the deliberate posture — claude runs as an
	// attended TUI so its trust/dev-channel consent stays human-answered (never
	// auto-bypassed), which is also why --background is refused for it; the
	// headless harnesses can detach with -d.
	posture := "headless-capable (-d to detach)"
	if harnessMode == "claude" {
		posture = "attached TUI, consent stays interactive (park it in tmux)"
	}
	fmt.Fprintf(stderr, "hotline: harness %s (%s) — %s\n", harnessMode, harnessSource, posture)

	if harnessMode == "pi" {
		// Pi has no permission prompts by design, so a supervised pi session
		// runs every tool unguarded — the same posture as claude's
		// --dangerously-skip-permissions. Say so loudly, like the yolo path does.
		fmt.Fprintln(stderr, "hotline: harness=pi. Pi has no permission prompts; every tool runs unguarded, like claude --dangerously-skip-permissions (see SECURITY.md).")
	}
	if harnessMode == "claude-sdk" {
		fmt.Fprintln(stderr, "hotline: harness=claude-sdk. The SDK session runs with bypassPermissions; every tool runs unguarded, like claude --dangerously-skip-permissions (see SECURITY.md).")
	}

	harnessLog, err := supervise.NewRotatingWriter(filepath.Join(supDir, supervise.HarnessLogName), 5<<20)
	if err != nil {
		return err
	}
	defer harnessLog.Close()

	// Signal handling is installed BEFORE the attacher puts the terminal into raw
	// mode (finding 2): an external INT/TERM/QUIT/HUP in that startup window then
	// drives a graceful shutdown — Run returns, and the deferred attacher.Close
	// restores the terminal — instead of a default-disposition exit that would
	// leave it raw. ctx is created here too so a cancel that lands before Run even
	// starts is remembered. The receive loop keeps ranging after the first signal
	// so a follow-up isn't swallowed mid-shutdown.
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGQUIT, syscall.SIGHUP)
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
		}
	}()

	// Attached claude runs a real interactive TUI: the attacher bridges the
	// operator's terminal (raw mode, bidirectional copy, SIGWINCH forwarding) to
	// the harness pty and tees output to harnessLog. When stdin/stdout are not both
	// the terminal (CI, a pipe, `hotline up >file`) the attacher is passive
	// (TTY() == false) and the claude branch falls back to the plain StartOnPTY
	// seam — exactly today's log-only behavior. Only the claude path attaches;
	// pi/opencode/claude-sdk stay headless on pipes. Constructed after the
	// announcements so their lines aren't printed into a raw terminal, passed the
	// supervisor's cancel so a same-terminal ^C during a spawn-failure backoff (or
	// a lost terminal) can stop it, and Close is deferred so the terminal is
	// restored on every exit path — its error surfaced, never silently swallowed.
	var attacher terminalAttacher
	if harnessMode == "claude" {
		attacher, err = newAttacher(os.Stdin, os.Stdout, harnessLog, cancel)
		if err != nil {
			return fmt.Errorf("attaching terminal: %w", err)
		}
		defer func() {
			if cerr := attacher.Close(); cerr != nil {
				fmt.Fprintf(stderr, "hotline: restoring the terminal failed: %v (run `reset` if your shell is garbled)\n", cerr)
			}
		}()
	}

	// While the attached claude TUI owns the terminal, supervisor diagnostics
	// (respawn notices, backoff, the "harness running" announce) must NOT race the
	// full-screen TUI on stderr — route them to supervisor/supervisor.log for the
	// duration. Only on the attached path (attacher.TTY()); the passive/non-tty
	// claude path and every other harness keep logging to stderr unchanged, and the
	// pre-raw launch announcements above already went to stderr. Gated on the same
	// attacher decision as the raw-mode engage, so it stays deterministic under the
	// newAttacher test seam. A one-line transition tells the operator where the
	// diagnostics went; it lands in their normal terminal buffer before the TUI
	// paints (\r\n because the terminal is already raw here).
	supLog := stderr
	if attacher != nil && attacher.TTY() {
		supLogPath := filepath.Join(supDir, supervise.SupervisorLogName)
		supLogF, err := os.OpenFile(supLogPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
		if err != nil {
			return fmt.Errorf("opening %s: %w", supLogPath, err)
		}
		defer supLogF.Close()
		supLog = supLogF
		fmt.Fprintf(stderr, "hotline: parking the TUI on your terminal — supervisor diagnostics now stream to %s\r\n", supLogPath)
	}

	// The harness argv is re-resolved on every spawn (the same config source as
	// `hotline run`), so an
	// `hotline init` fix, a binary upgrade, or an OPENCODE_SERVER_URL change
	// applies on the next restart without bouncing the supervisor.
	start := func(ctx context.Context) (supervise.Harness, error) {
		// HOTLINE_SUPERVISOR_DIR enables the restart MCP tool in the hotline
		// session: claude passes its env to stdio MCP children, and opencode
		// serve does too (verified: process env is merged with opencode.json's
		// explicit environment block).
		// hotline (review F6): strip HOTLINE_SUBAGENT_DEPTH from the supervised
		// env. It is an INTERNAL marker the pi subagent tool sets on the children
		// it spawns; a value leaked into the operator's shell that launches
		// `hotline up` would make the top-level pi extension think it is a subagent
		// worker and disable the channel bridge (bot goes dark). The top-level bot
		// is always depth 0, so we scrub the var here before any harness starts.
		env := scrubEnv(append(os.Environ(), supervise.EnvDir+"="+supDir), "HOTLINE_SUBAGENT_DEPTH", lifecycle.OwnerLeaseEnv)
		env = append(env, lifecycle.OwnerLeaseEnv+"="+ownerReservation.ID)

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

		if harnessMode == "pi" {
			bin, err := exec.LookPath("pi")
			if err != nil {
				return nil, err
			}
			// Headless `pi --mode rpc` on plain pipes, but with stdin held open:
			// the hotline-pi extension (loaded inside pi) spawns the hotline run
			// child itself and injects turns in-process, so no RPC client ever
			// writes to pi's stdin — a /dev/null stdin would EOF and end the
			// session. --session-id keeps memory across restarts.
			//
			// The operator model knob (HOTLINE_PI_{PROVIDER,MODEL,THINKING} from
			// the shared .env) is injected BEFORE the passthrough args, and any
			// knob flag the passthrough already carries is dropped — so an explicit
			// `-- --provider/--model/--thinking` always wins over the .env knob
			// without relying on Pi's last-flag-wins.
			// Per-box (sol review #10): the same <boxRoot>/.env the app
			// channel's hot apply writes, so a confirmed change survives the
			// next respawn of THIS box and no other.
			knob, err := config.LoadPiModelForBox(boxRoot)
			if err != nil {
				return nil, err
			}
			argv := []string{bin, "--mode", "rpc", "--session-id", piSession}
			argv = append(argv, piModelArgs(knob, passthrough)...)
			argv = append(argv, passthrough...)
			// Model catalog amendment 2026-07-20: re-export the EFFECTIVE
			// --models scope (after passthrough-wins) into pi's env. The
			// extension cannot see pi's resolved cycling list — neither
			// ExtensionContext nor ExtensionAPI exposes the AgentSession that
			// owns scopedModels — so this env line is how the harness learns
			// which scope the launch line actually chose, and resolves the
			// identical list through pi's own resolveModelScope for the
			// catalog the app renders. Scrubbed first so a stale inherited
			// value can never outrank the resolved one, and only exported when
			// a scope exists (absent = pi cycles everything with auth, which
			// the extension reports as such).
			env = scrubEnv(env, "HOTLINE_PI_MODELS")
			if models := effectivePiModels(knob, passthrough); models != "" {
				env = append(env, "HOTLINE_PI_MODELS="+models)
			}
			return startHarnessPipedStdinOpen(argv, dir, env, harnessLog)
		}

		if harnessMode == "claude-sdk" {
			// Agent-SDK managed edition: a headless node process (spec shape =
			// the pi harness) that spawns the `hotline run` child itself and
			// owns the two-way inject loop. Plain pipes with stdin held open —
			// nothing ever writes to its stdin; an EOF would end the process.
			entry, err := claudeSDKEntry()
			if err != nil {
				return nil, err
			}
			nodeBin, err := exec.LookPath("node")
			if err != nil {
				return nil, err
			}
			// Pin the child to this exact build so the harness never picks a
			// stale `hotline` from PATH.
			hotlineBin, err := os.Executable()
			if err != nil {
				return nil, err
			}
			env = scrubEnv(env, "HOTLINE_BIN", "HOTLINE_YOLO")
			env = append(env, "HOTLINE_BIN="+hotlineBin)
			if *yolo {
				// The session is already bypassPermissions; --yolo only rides
				// along as the conventional env marker.
				env = append(env, "HOTLINE_YOLO=1")
			}
			// Alternate Anthropic provider / pinned keys from the shared .env;
			// real environment wins per key (pluggable auth seam).
			env, err = config.AnthropicChildEnv(env)
			if err != nil {
				return nil, err
			}
			// Per-box SDK knobs (model / effort / max turns): resolve real
			// env + shared .env here and re-export the RESOLVED values, so
			// .env-only knobs reach the node harness (it reads real env only)
			// and, through its env spread, the run child — which seeds the
			// wire metadata. Sitting inside start(), this re-resolves on
			// every respawn: edit .env + SIGHUP picks up a new model without
			// bouncing the supervisor.
			// Per-box (sol review #10): reads the same <boxRoot>/.env the app
			// channel's hot apply writes.
			sdkCfg, err := config.LoadSDKForBox(boxRoot)
			if err != nil {
				return nil, err
			}
			env = scrubEnv(env, "HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_SDK_SETTING_SOURCES")
			if sdkCfg.Model != "" {
				env = append(env, "HOTLINE_SDK_MODEL="+sdkCfg.Model)
			}
			if sdkCfg.Effort != "" {
				env = append(env, "HOTLINE_SDK_EFFORT="+sdkCfg.Effort)
			}
			if sdkCfg.MaxTurns > 0 {
				env = append(env, "HOTLINE_SDK_MAX_TURNS="+strconv.Itoa(sdkCfg.MaxTurns))
			}
			if sdkCfg.SettingSources != "" {
				env = append(env, "HOTLINE_SDK_SETTING_SOURCES="+sdkCfg.SettingSources)
			}
			argv := append([]string{nodeBin, entry}, passthrough...)
			return startHarnessPipedStdinOpen(argv, dir, env, harnessLog)
		}

		bin, err := exec.LookPath("claude")
		if err != nil {
			return nil, err
		}
		argv := append([]string{bin}, channelArgs(dir, stderr)...)
		if *yolo {
			argv = append(argv, "--dangerously-skip-permissions")
			env = append(env, "HOTLINE_YOLO=1")
		}
		argv = append(argv, passthrough...)
		if os.Getenv("TERM") == "" {
			env = append(env, "TERM=xterm-256color") // the pty-hosted TUI needs one
		}
		// Alternate Anthropic provider from the shared .env (claude path only;
		// the opencode branch above does providers via opencode.json). Real
		// environment wins per key.
		env, err = config.AnthropicChildEnv(env)
		if err != nil {
			return nil, err
		}
		// Real terminal → bridge the interactive TUI to it. Non-tty stdin →
		// fall through to the StartOnPTY seam (log-only, and the test seam).
		if attacher != nil && attacher.TTY() {
			return attacher.Start(argv, dir, env)
		}
		return startHarness(argv, dir, env, harnessLog)
	}

	sup := supervise.New(supDir, start)
	sup.Log = supLog
	sup.Argv = supervisedArgvLabel(harnessMode, *yolo, piSession, passthrough)
	sup.WorkDir = dir
	// Attached claude TUI: the operator quitting claude (clean exit) is the stop
	// signal — don't respawn a session they just closed. Only when actually
	// bridging a terminal; a non-tty claude keeps the always-on restart-on-exit
	// posture, and the headless harnesses are never affected.
	sup.StopOnCleanExit = attacher != nil && attacher.TTY()

	return sup.Run(ctx)
}

// detachSupervisor re-execs this binary as `up` (attached is the default) in a
// new session with stdin from /dev/null and both output streams appended to
// supervisor.log, then returns immediately. Selections already applied to the
// environment (HOTLINE_PROVIDERS, HOTLINE_BOT) ride along via os.Environ.
func detachSupervisor(supDir string, yolo bool, passthrough []string, dir string, stdout io.Writer) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	args := []string{"up"}
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

// printSupervisorStatus appends the always-on block to `hotline status`.
func printSupervisorStatus(boxRoot string, stdout io.Writer) {
	supDir := supervise.Dir(boxRoot)
	if !supervise.Running(supDir) {
		fmt.Fprintln(stdout, "supervisor:  not running (`hotline up` for an always-on session)")
		return
	}
	st, err := supervise.ReadState(supDir)
	if err != nil || st == nil {
		fmt.Fprintf(stdout, "supervisor:  running (state unreadable: %v)\n", err)
		return
	}
	switch st.Phase {
	case supervise.PhaseBackoff:
		fmt.Fprintf(stdout, "supervisor:  running (pid %d) — harness DOWN, retrying at %s\n", st.PID, localTime(st.NextRestartAt))
	default:
		fmt.Fprintf(stdout, "supervisor:  running (pid %d) — harness pid %d, up since %s\n", st.PID, st.HarnessPID, localTime(st.HarnessStartedAt))
	}
	if st.Restarts > 0 {
		fmt.Fprintf(stdout, "  restarts:  %d (last exit: %s)\n", st.Restarts, st.LastExit)
	}
	fmt.Fprintf(stdout, "  logs:      %s\n", supDir)
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
func supervisedArgvLabel(harnessMode string, yolo bool, piSession string, passthrough []string) []string {
	switch harnessMode {
	case "opencode":
		return append([]string{"opencode", "serve"}, passthrough...)
	case "pi":
		return append([]string{"pi", "--mode", "rpc", "--session-id", piSession}, passthrough...)
	case "claude-sdk":
		return append([]string{"node", "claude-sdk-harness"}, passthrough...)
	}
	argv := []string{"claude"}
	if yolo {
		argv = append(argv, "--dangerously-skip-permissions")
	}
	return append(argv, passthrough...)
}

// piSessionID resolves the stable session id for the supervised `pi --mode rpc`
// (the same role OPENCODE_SESSION plays for the opencode branch). It is
// deterministic — HOTLINE_PI_SESSION when set, else "hotline" keyed by bot name
// — so every restart passes the same id and Pi resumes the same session without
// any on-disk persistence.
func piSessionID(botName string) string {
	if s := strings.TrimSpace(os.Getenv("HOTLINE_PI_SESSION")); s != "" {
		return s
	}
	if botName != "" {
		return "hotline-" + botName
	}
	return "hotline"
}

// piModelArgs turns the operator model knob into pi flags, in the fixed order
// --provider, --model, --thinking, --models. A knob field is skipped when it is
// empty or when the passthrough already carries that flag, so an explicit
// passthrough selection wins over the .env knob (passthrough-wins, without
// depending on pi's last-flag-wins). Returns nil when the knob is fully unset.
//
// --models is appended LAST rather than slotted next to --model so the existing
// three keep their frozen order (and every frozen expectation built on it); the
// order carries no meaning to pi's arg parser.
func piModelArgs(knob config.PiModel, passthrough []string) []string {
	var out []string
	add := func(flag, val string) {
		if val == "" || passthroughHasFlag(passthrough, flag) {
			return
		}
		out = append(out, flag, val)
	}
	add("--provider", knob.Provider)
	add("--model", knob.Model)
	add("--thinking", knob.Thinking)
	add("--models", knob.Models)
	return out
}

// effectivePiModels reports the --models scope the pi process will ACTUALLY run
// with, after passthrough-wins has been settled (model catalog amendment
// 2026-07-20). That is the value the box re-exports as HOTLINE_PI_MODELS so the
// extension resolves the same list the launch line scoped.
//
// Why it can't just be the knob: piModelArgs DROPS the knob's --models when the
// passthrough already carries one, so on a box launched as
// `hotline up -- --models anthropic/*` the knob is inert and re-exporting it
// would make the app's model rows advertise a list Ctrl+P does not cycle. This
// reads the winning value from whichever side won. Returns "" when neither side
// sets a scope (pi then cycles everything with auth configured, and the
// extension reports exactly that).
func effectivePiModels(knob config.PiModel, passthrough []string) string {
	if v, ok := passthroughFlagValue(passthrough, "--models"); ok {
		return config.NormalizePiModels(v)
	}
	return knob.Models
}

// passthroughFlagValue reads the value a passthrough carries for flag, in both
// spellings pi's arg parser accepts: `--flag value` and `--flag=value`. The
// LAST occurrence wins, matching pi's own last-flag-wins parse, so what we
// re-export is what pi resolved. ok=false means the flag is absent; ok=true with
// an empty value means it was present but valueless (`--models` at the end of
// the line), which scopes nothing — the same as absent for pi, and normalizing
// it to "" here keeps the export honest.
func passthroughFlagValue(args []string, flag string) (string, bool) {
	value, found := "", false
	for i, a := range args {
		switch {
		case a == flag:
			found = true
			if i+1 < len(args) {
				value = args[i+1]
			} else {
				value = ""
			}
		case strings.HasPrefix(a, flag+"="):
			found = true
			value = strings.TrimPrefix(a, flag+"=")
		}
	}
	return value, found
}

type mcpIdentity struct {
	Found   bool
	BotName string // effective child selection after MCP args/env precedence
}

// adoptProjectMCPServerEnv finds the nearest .mcp.json at or above dir, then
// adopts its hotline identity. Walking parents keeps commands run from a
// project subdirectory on the same box as commands run from its root.
func adoptProjectMCPServerEnv(dir string, stderr io.Writer) (mcpIdentity, error) {
	dir, err := filepath.Abs(filepath.Clean(dir))
	if err != nil {
		return mcpIdentity{}, fmt.Errorf("resolving project directory %q: %w", dir, err)
	}
	for {
		path := filepath.Join(dir, ".mcp.json")
		if _, err := os.Stat(path); err == nil {
			return adoptMCPServerEnv(path, stderr)
		} else if !os.IsNotExist(err) {
			return mcpIdentity{}, fmt.Errorf("checking %s: %w", path, err)
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return mcpIdentity{}, nil
		}
		dir = parent
	}
}

// adoptMCPServerEnv aligns operator commands with the box identity recorded in
// one raw .mcp.json hotline entry. The MCP child receives that env block and
// any explicit --bot argument, so project-scoped verbs must resolve the same
// HOTLINE_*/TELE_GO_* values before touching state. Internal lifecycle markers
// are never adopted.
func adoptMCPServerEnv(mcpPath string, stderr io.Writer) (mcpIdentity, error) {
	_, env, entryBot, entryBotSet, found, err := mcpServerEntry(mcpPath)
	if err != nil {
		return mcpIdentity{}, err
	}
	if !found {
		return mcpIdentity{}, nil
	}
	keys := make([]string, 0, len(env))
	for k := range env {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		if !strings.HasPrefix(k, "HOTLINE_") && !strings.HasPrefix(k, "TELE_GO_") {
			continue
		}
		if k == lifecycle.OwnerLeaseEnv || k == supervise.EnvDir {
			fmt.Fprintf(stderr, "hotline: warning: ignoring internal %s from %s — remove it; it would break the supervisor/child ownership handshake\n", k, mcpPath)
			continue
		}
		v := env[k]
		if cur, ok := os.LookupEnv(k); ok && cur != v {
			fmt.Fprintf(stderr, "hotline: %s=%q from %s overrides %q for this project box (the MCP child receives the .mcp.json value)\n", k, v, mcpPath, cur)
		}
		if err := os.Setenv(k, v); err != nil {
			return mcpIdentity{}, fmt.Errorf("adopting %s from %s: %w", k, mcpPath, err)
		}
	}
	effectiveBot := entryBot
	if !entryBotSet {
		effectiveBot = os.Getenv("HOTLINE_BOT")
		if effectiveBot == "" {
			effectiveBot = os.Getenv("TELE_GO_BOT")
		}
	}
	return mcpIdentity{Found: true, BotName: effectiveBot}, nil
}

// claudeSDKEntry resolves the built claude-sdk harness entry point from
// HOTLINE_CLAUDE_SDK_ENTRY (an absolute path to dist/index.js). There is no
// PATH-style fallback: the harness is a repo-local package, so a
// missing/naked value fails loudly with build instructions.
//
// It also enforces the Go↔TS lockstep rule (PARITY.md): the claude-sdk build
// stamps dist/.hotline-harness with the child identity it spawns. A dist
// built before the first-class harness change would spawn the run child as
// HOTLINE_HARNESS=pi, whose ClaimBox would then mismatch this supervisor's
// claude-sdk reservation — a confusing ownership refusal at claim time. The
// marker check converts that version skew into a one-line fix at launch.
func claudeSDKEntry() (string, error) {
	entry := strings.TrimSpace(os.Getenv("HOTLINE_CLAUDE_SDK_ENTRY"))
	if entry == "" {
		return "", fmt.Errorf("harness=claude-sdk needs HOTLINE_CLAUDE_SDK_ENTRY pointing at the built harness. Build it first:\n  cd harness/claude-sdk && npm install && npm run build\nthen export HOTLINE_CLAUDE_SDK_ENTRY=<repo>/harness/claude-sdk/dist/index.js")
	}
	if _, err := os.Stat(entry); err != nil {
		return "", fmt.Errorf("HOTLINE_CLAUDE_SDK_ENTRY=%q is not readable (%v). Build the harness (cd harness/claude-sdk && npm install && npm run build) and point at dist/index.js", entry, err)
	}
	marker := filepath.Join(filepath.Dir(entry), ".hotline-harness")
	raw, err := os.ReadFile(marker)
	if err != nil {
		return "", fmt.Errorf("stale claude-sdk build at %q: no %s marker (%v) — run `npm run build` in harness/claude-sdk so the harness and this binary agree on the child identity", entry, filepath.Base(marker), err)
	}
	for _, line := range strings.Split(string(raw), "\n") {
		if k, v, ok := strings.Cut(strings.TrimSpace(line), "="); ok && k == "child-harness" {
			if v != "claude-sdk" {
				return "", fmt.Errorf("stale claude-sdk build at %q: dist spawns the run child as harness=%s but this binary reserves the box as claude-sdk — run `npm run build` in harness/claude-sdk", entry, v)
			}
			return entry, nil
		}
	}
	return "", fmt.Errorf("stale claude-sdk build at %q: %s carries no child-harness line — run `npm run build` in harness/claude-sdk", entry, filepath.Base(marker))
}

// scrubEnv returns env with every `KEY=...` entry for the named keys removed.
// Used to drop internal markers (e.g. HOTLINE_SUBAGENT_DEPTH) that must never be
// inherited by the top-level supervised harness.
func scrubEnv(env []string, keys ...string) []string {
	if len(keys) == 0 {
		return env
	}
	drop := make(map[string]bool, len(keys))
	for _, k := range keys {
		drop[k] = true
	}
	out := env[:0:0]
	for _, e := range env {
		name := e
		if i := strings.IndexByte(e, '='); i >= 0 {
			name = e[:i]
		}
		if drop[name] {
			continue
		}
		out = append(out, e)
	}
	return out
}

// passthroughHasFlag reports whether args carries the given long flag, in
// either `--flag value` or `--flag=value` form. Pi has no short-flag aliases for
// --provider/--model/--thinking (verified against `pi --help`), so long-form
// matching here fully covers the knob-vs-passthrough dedupe (review F11).
func passthroughHasFlag(args []string, flag string) bool {
	for _, a := range args {
		if a == flag || strings.HasPrefix(a, flag+"=") {
			return true
		}
	}
	return false
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
