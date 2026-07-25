//go:build linux || darwin

package supervise

import (
	"io"
	"os"
	"os/exec"
	"syscall"
)

// StartOnPTY launches argv in dir with env on a freshly allocated pty, its
// output streamed to logw. The child gets the pty slave as stdin/stdout/stderr
// and becomes a session leader with the slave as its controlling terminal
// (interactive Claude Code requires a tty). Its own session also means its own
// process group, so Terminate/Kill can take down the harness and its children in
// one signal. Hotline never writes input back to the pty master.
func StartOnPTY(argv []string, dir string, env []string, logw io.Writer) (Harness, error) {
	master, slave, err := openPTY()
	if err != nil {
		return nil, err
	}
	setWinsize(master, 200, 50)

	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Dir = dir
	cmd.Env = env
	cmd.Stdin, cmd.Stdout, cmd.Stderr = slave, slave, slave
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setsid:  true,
		Setctty: true,
		Ctty:    0, // the child's fd 0 — the slave
	}
	setPdeathsig(cmd) // Linux: harness dies with the supervisor instead of orphaning

	if err := cmd.Start(); err != nil {
		master.Close()
		slave.Close()
		return nil, err
	}
	slave.Close() // child holds its own copy now

	// Drain the master until the pty returns EIO (last slave fd closed — the
	// whole process group is gone). The drain goroutine owns closing the master.
	go func() {
		drainPTY(master, logw)
		master.Close()
	}()
	return watchCmd(cmd), nil
}

func drainPTY(master io.Reader, logw io.Writer) {
	buf := make([]byte, 4096)
	for {
		n, readErr := master.Read(buf)
		if n > 0 {
			_, _ = logw.Write(buf[:n])
		}
		if readErr != nil {
			return
		}
	}
}

// StartPiped launches argv in dir with env on plain pipes: stdout and stderr
// stream into logw, stdin reads empty (/dev/null). It is the no-pty sibling
// of StartOnPTY for headless daemons (`opencode serve`) that need no
// terminal. The child still gets its own session — hence its own process
// group, so Terminate/Kill take down the daemon AND its children (the
// hotline MCP server it spawns, tunnels) in one signal — and on Linux dies
// with the supervisor via Pdeathsig, exactly like the pty path.
func StartPiped(argv []string, dir string, env []string, logw io.Writer) (Harness, error) {
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Dir = dir
	cmd.Env = env
	// Stdin nil = /dev/null. Stdout and Stderr are the same writer, so
	// os/exec serializes their copies onto it; RotatingWriter locks anyway.
	cmd.Stdout, cmd.Stderr = logw, logw
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	setPdeathsig(cmd) // Linux: harness dies with the supervisor instead of orphaning

	if err := cmd.Start(); err != nil {
		return nil, err
	}
	return watchCmd(cmd), nil
}

// StartPipedStdinOpen is StartPiped with one difference: the child's stdin is a
// live pipe the supervisor holds open (never written, never EOF'd) for the
// process lifetime, instead of /dev/null. `pi --mode rpc` reads JSONL commands
// from stdin and treats stdin EOF as a shutdown signal; in hotline's topology no
// RPC client ever writes to it (the extension injects turns in-process), so a
// /dev/null stdin would EOF immediately and the session would exit. Holding the
// write end open keeps the headless session alive until the supervisor signals
// it. Output, session/process-group, and Pdeathsig discipline match StartPiped.
func StartPipedStdinOpen(argv []string, dir string, env []string, logw io.Writer) (Harness, error) {
	pr, pw, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Dir = dir
	cmd.Env = env
	cmd.Stdin = pr
	cmd.Stdout, cmd.Stderr = logw, logw
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	setPdeathsig(cmd) // Linux: harness dies with the supervisor instead of orphaning

	if err := cmd.Start(); err != nil {
		pr.Close()
		pw.Close()
		return nil, err
	}
	pr.Close() // the child holds its own copy of the read end now
	return watchCmdWithStdin(cmd, pw), nil
}

// watchCmd wraps a started cmd in the Harness the supervisor watches,
// reaping it in the background.
func watchCmd(cmd *exec.Cmd) *procHarness {
	return watchCmdWithStdin(cmd, nil)
}

// watchCmdWithStdin is watchCmd plus an optional held-open stdin write end (see
// StartPipedStdinOpen). The write end is closed once the process exits so its fd
// never leaks across restarts.
func watchCmdWithStdin(cmd *exec.Cmd, stdin io.Closer) *procHarness {
	h := &procHarness{cmd: cmd, done: make(chan struct{})}
	go func() {
		if err := cmd.Wait(); err != nil {
			h.exitDesc = err.Error()
		} else {
			h.exitDesc = CleanExitDesc
		}
		if stdin != nil {
			stdin.Close()
		}
		close(h.done) // exitDesc write happens-before Done
	}()
	return h
}

// procHarness is one supervised child (pty or piped — the spawn wiring is
// the only difference between the two constructors).
type procHarness struct {
	cmd      *exec.Cmd
	done     chan struct{}
	exitDesc string // written before done closes; read only after
}

func (h *procHarness) Pid() int              { return h.cmd.Process.Pid }
func (h *procHarness) Done() <-chan struct{} { return h.done }
func (h *procHarness) ExitDesc() string      { return h.exitDesc }

// Terminate SIGTERMs the harness's process group (it is a session leader, so
// pgid == pid).
func (h *procHarness) Terminate() { _ = syscall.Kill(-h.cmd.Process.Pid, syscall.SIGTERM) }

// Kill SIGKILLs the process group.
func (h *procHarness) Kill() { _ = syscall.Kill(-h.cmd.Process.Pid, syscall.SIGKILL) }

var _ Harness = (*procHarness)(nil)
