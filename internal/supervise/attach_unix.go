//go:build linux || darwin

package supervise

import (
	"bytes"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"golang.org/x/term"
)

// retireTimeout bounds how long a respawn (or Close) waits for the previous
// generation's output drain to finish before exposing the new terminal. A dead
// pty whose descendants still hold the slave must never hang the supervisor.
const retireTimeout = 2 * time.Second

// Attacher bridges the operator's real terminal to the harness pty across the
// supervisor's respawn loop. It exists because the plain StartOnPTY drains the
// pty master into the log ONLY — so on the claude path an attached `hotline up`
// showed supervisor log lines while the interactive TUI rendered invisibly into
// harness.log and Claude's consent prompts sat unanswerable. Attacher makes the
// attached claude path deliver the real TUI: the operator's stdin drives the
// harness, the harness paints their stdout, and every byte is still teed to the
// log (the log is how this bug was diagnosed).
//
// One Attacher spans the whole supervised session; the supervisor calls Start
// once per spawn. Each spawn is a *generation* (ptyGen): the master, a
// cancellable per-generation stdin writer, and a drain-completion signal. On
// respawn the previous generation is RETIRED — its master closed and its output
// drain awaited (bounded) — before the new one is exposed, so two generations
// never paint the terminal at once and a blocked write to a dead pty can never
// stall the single stdin pump.
//
// Raw mode, SIGWINCH, and job-control (SIGTSTP/SIGCONT) handling are engaged
// once in NewAttacher and undone once in Close; the caller MUST defer Close so
// the terminal is restored on every exit path (harness quit, `hotline down`,
// panic in an Attacher goroutine).
//
// When stdin AND stdout are not BOTH the operator's terminal (CI, a pipe, or a
// redirected `hotline up >file`), tty is false: NewAttacher engages nothing,
// Start falls back to exactly StartOnPTY's log-only behavior, and the caller
// keeps routing through its own StartOnPTY seam.
type Attacher struct {
	in     *os.File  // the operator's terminal (os.Stdin)
	out    *os.File  // the operator's terminal (os.Stdout)
	logw   io.Writer // harness.log — always teed, tty or not
	cancel func()    // stops the supervisor (ETX with no live master / stdin loss)

	tty      bool
	fd       int // stdin fd
	outFd    int // stdout fd
	oldState *term.State

	mu     sync.Mutex
	cur    *ptyGen // the live generation, nil between spawns / after Close
	closed bool

	winch chan os.Signal
	jobc  chan os.Signal
}

// ptyGen is one spawned pty and the goroutines bridging it. Its master is
// closed exactly once (closeOnce), by whichever of retirement or the drain
// goroutine gets there first. live gates its terminal sink: retirement flips it
// false so a still-draining old generation can never paint the terminal after
// the next one is exposed — closing a pty master does NOT reliably unblock a
// drain whose slave a descendant still holds (Go keeps ttys in blocking mode),
// so disconnecting the sink, not the read, is what guarantees no interleave.
type ptyGen struct {
	master    *os.File
	in        chan []byte   // stdin bytes routed to this generation's writer
	drainDone chan struct{} // closed when the output drain has finished
	stop      chan struct{} // closed to retire this generation's writer
	live      atomic.Bool   // true while this generation may paint the terminal
	closeOnce sync.Once
}

func (g *ptyGen) close() { g.closeOnce.Do(func() { _ = g.master.Close() }) }

// NewAttacher builds the terminal bridge. It attaches only when BOTH stdin and
// stdout are terminals (a redirected `hotline up >file` keeps a tty stdin but a
// file stdout — entering raw mode there would strand the TUI in the file and
// risk a SIGPIPE that skips the deferred restore). When it attaches it installs
// the SIGWINCH and job-control handlers BEFORE putting the terminal into raw
// mode — so a signal in the startup window can never leave the terminal raw —
// and starts the persistent stdin pump. cancel is invoked when the operator
// asks to stop from the only terminal (ETX during a spawn-failure backoff) or
// when that terminal disappears (stdin EOF/EIO). Otherwise it returns a passive
// Attacher (TTY() == false) that engages nothing. The caller must defer Close.
func NewAttacher(in, out *os.File, logw io.Writer, cancel func()) (*Attacher, error) {
	a := &Attacher{in: in, out: out, logw: logw, cancel: cancel, fd: int(in.Fd()), outFd: int(out.Fd())}
	// Finding 1: require stdin AND stdout to be the operator's terminal.
	if !term.IsTerminal(a.fd) || !term.IsTerminal(a.outFd) {
		return a, nil // passive: caller falls back to StartOnPTY
	}
	// Finding 2: signal coordination is installed BEFORE MakeRaw, so a SIGWINCH
	// or job-control signal that lands in the startup window is captured (and the
	// terminal restored) rather than taking the default disposition against a raw
	// terminal. Fatal-signal coordination (INT/TERM/QUIT/HUP) is the caller's, and
	// is likewise installed before this constructor runs.
	a.winch = make(chan os.Signal, 1)
	signal.Notify(a.winch, syscall.SIGWINCH)
	a.jobc = make(chan os.Signal, 1)
	signal.Notify(a.jobc, syscall.SIGTSTP, syscall.SIGCONT)
	st, err := term.MakeRaw(a.fd)
	if err != nil {
		signal.Stop(a.winch)
		signal.Stop(a.jobc)
		return nil, err
	}
	a.tty = true
	a.oldState = st
	go a.pumpStdin()
	go a.watchWinch()
	go a.watchJobControl()
	return a, nil
}

// TTY reports whether the Attacher is bridging a real terminal. When false the
// caller must not use Start; it falls back to StartOnPTY (the passive Attacher
// has engaged nothing).
func (a *Attacher) TTY() bool { return a.tty }

// Start launches argv on a fresh pty and bridges it to the operator's terminal.
// On the tty path it retires the previous generation (closing its master and
// awaiting its drain, bounded) before exposing the new one, sizes the pty from
// the real terminal, registers the master with the stdin pump and SIGWINCH
// handler, and tees master output to BOTH the terminal and the log. On the
// non-tty (passive) path it is identical to StartOnPTY: fixed 200x50, output to
// the log only, terminal untouched.
func (a *Attacher) Start(argv []string, dir string, env []string) (Harness, error) {
	master, slave, err := openPTY()
	if err != nil {
		return nil, err
	}
	// Size before the child boots so its TUI comes up at the right geometry.
	if a.tty {
		if cols, rows, err := term.GetSize(a.fd); err == nil {
			setWinsize(master, cols, rows)
		} else {
			setWinsize(master, 200, 50)
		}
	} else {
		setWinsize(master, 200, 50)
	}

	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Dir = dir
	cmd.Env = env
	cmd.Stdin, cmd.Stdout, cmd.Stderr = slave, slave, slave
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setsid:  true,
		Setctty: true,
		Ctty:    0, // the child's fd 0 — the slave
	}
	setPdeathsig(cmd)

	if err := cmd.Start(); err != nil {
		master.Close()
		slave.Close()
		return nil, err
	}
	slave.Close() // child holds its own copy now

	if !a.tty {
		// Passive path: identical to StartOnPTY (log-only). The drain goroutine
		// owns closing the master.
		go func() {
			drainPTY(master, a.logw)
			master.Close()
		}()
		return watchCmd(cmd), nil
	}

	// tty path — per-generation retirement (finding 3).
	gen := &ptyGen{
		master:    master,
		in:        make(chan []byte, 8),
		drainDone: make(chan struct{}),
		stop:      make(chan struct{}),
	}
	a.mu.Lock()
	old := a.cur
	a.cur = nil // no routing target while we swap generations
	a.mu.Unlock()
	if old != nil {
		a.retire(old) // close old master + await its drain, bounded
	}

	// Tee to BOTH the terminal and the log, attempting both sinks and swallowing
	// per-sink errors so a stdout fault never starves harness.log (finding 6). The
	// terminal half is gated on gen.live so retirement can silence it instantly.
	sink := teeWriter{term: a.out, log: a.logw, live: &gen.live}

	a.mu.Lock()
	if a.closed {
		// Close won the race while we retired the old generation. Do not expose a
		// new terminal-painting drain; let the supervisor tear the child down.
		a.mu.Unlock()
		gen.close()
		return watchCmd(cmd), nil
	}
	gen.live.Store(true)
	a.cur = gen
	// Re-affirm size under mu so a SIGWINCH that raced the swap is not lost and
	// the resize is serialized against retirement.
	if cols, rows, err := term.GetSize(a.fd); err == nil {
		setWinsize(gen.master, cols, rows)
	}
	a.mu.Unlock()

	go a.writeGen(gen)
	go func() {
		drainPTY(gen.master, sink)
		gen.close()
		close(gen.drainDone)
	}()
	return watchCmd(cmd), nil
}

// retire closes a generation's master and awaits its output drain, bounded.
// Closing the master ends the drain (EIO) and unblocks its writer goroutine
// (the runtime poller wakes a blocked Write with ErrClosed), so a dead pty
// cannot keep painting the terminal or wedge input after the swap.
func (a *Attacher) retire(g *ptyGen) {
	g.live.Store(false) // silence its terminal sink NOW — no interleave past here
	close(g.stop)       // ask its writer to stop
	g.close()           // close master: ends the drain when its slave is gone
	select {
	case <-g.drainDone: // common case: the harness already exited, drain ends fast
	case <-time.After(retireTimeout): // a descendant still holds the slave; proceed
	}
}

// writeGen is one generation's stdin writer. It is cancellable: retirement
// closes stop (and the master), so a write blocked on a dead pty ends this
// goroutine alone and never stalls the single pump.
func (a *Attacher) writeGen(g *ptyGen) {
	for {
		select {
		case b := <-g.in:
			if _, err := g.master.Write(b); err != nil {
				return
			}
		case <-g.stop:
			return
		}
	}
}

// Close retires the live generation, restores the terminal, and tears down the
// pump/winch/job-control handling. It is idempotent and safe to defer. On the
// passive (non-tty) path it is a no-op. It does NOT mark itself closed until
// term.Restore has succeeded, so a failed restore is surfaced to the caller and
// can be retried rather than silently swallowed. The stdin pump goroutine may
// remain blocked in a terminal Read after Close; `hotline up` returns to process
// exit immediately after, which reaps it.
func (a *Attacher) Close() error {
	if !a.tty {
		return nil
	}
	a.mu.Lock()
	if a.closed {
		a.mu.Unlock()
		return nil
	}
	g := a.cur
	a.cur = nil
	a.mu.Unlock()
	if g != nil {
		a.retire(g)
	}
	// Stop delivering signals before restoring, so a racing SIGCONT can't re-raw
	// the terminal after we have cooked it.
	signal.Stop(a.winch)
	signal.Stop(a.jobc)
	if err := term.Restore(a.fd, a.oldState); err != nil {
		return err // not marked closed: the restore can be retried
	}
	a.mu.Lock()
	a.closed = true
	a.mu.Unlock()
	close(a.winch)
	close(a.jobc)
	return nil
}

// pumpStdin is the single long-lived stdin reader. It routes every keystroke to
// the live generation's writer; between respawns (no live generation) an ETX
// (^C) is the operator's only way to stop the supervisor from its one terminal,
// so it triggers cancellation, and any other byte is dropped (there is no TUI to
// receive it during backoff). A stdin read error (EOF/EIO — the pane was killed,
// the terminal is gone) also cancels: a blind supervisor on a dead terminal must
// not keep running.
func (a *Attacher) pumpStdin() {
	defer a.restoreOnPanic()
	buf := make([]byte, 4096)
	for {
		n, err := a.in.Read(buf)
		if n > 0 {
			a.mu.Lock()
			g := a.cur
			a.mu.Unlock()
			if g != nil {
				b := append([]byte(nil), buf[:n]...) // buf is reused; copy for the writer
				select {
				case g.in <- b:
				default: // this generation's pty is wedged; drop rather than stall
				}
			} else if bytes.IndexByte(buf[:n], 0x03) >= 0 {
				a.triggerCancel() // finding 4: ^C with no live master stops the supervisor
			}
		}
		if err != nil {
			a.triggerCancel() // finding 4: terminal loss stops the supervisor
			return
		}
	}
}

// watchWinch forwards terminal resizes to the live master so the TUI reflows.
// The lookup and ioctl are done under mu, serializing resizes against generation
// retirement (a resize can never land on a master being torn down).
func (a *Attacher) watchWinch() {
	defer a.restoreOnPanic()
	for range a.winch {
		cols, rows, err := term.GetSize(a.fd)
		if err != nil {
			continue
		}
		a.mu.Lock()
		if a.cur != nil {
			setWinsize(a.cur.master, cols, rows)
		}
		a.mu.Unlock()
	}
}

// watchJobControl restores the terminal before the process suspends and re-raws
// it on resume, so ^Z (or an external SIGTSTP) never leaves the operator's shell
// with a raw terminal. SIGTSTP is netted here (default-suspend is suppressed once
// notified), so the actual stop is an explicit uncatchable SIGSTOP.
func (a *Attacher) watchJobControl() {
	defer a.restoreOnPanic()
	for sig := range a.jobc {
		switch sig {
		case syscall.SIGTSTP:
			_ = term.Restore(a.fd, a.oldState)
			_ = syscall.Kill(os.Getpid(), syscall.SIGSTOP)
		case syscall.SIGCONT:
			_, _ = term.MakeRaw(a.fd) // oldState (the cooked state) is unchanged
		}
	}
}

// restoreOnPanic restores the terminal if one of the Attacher's own goroutines
// panics, then re-panics. It does NOT cover panics in unrelated goroutines —
// those still terminate the process without running any defer (see the package
// notes / review residual: a parent terminal-guard process would be required).
func (a *Attacher) restoreOnPanic() {
	if r := recover(); r != nil {
		_ = term.Restore(a.fd, a.oldState)
		panic(r)
	}
}

func (a *Attacher) triggerCancel() {
	if a.cancel != nil {
		a.cancel()
	}
}

// teeWriter writes to both the terminal and the log, attempting both sinks even
// when one fails and swallowing per-sink errors, so a stdout fault never starves
// harness.log (and vice versa). It always reports a full write so drainPTY keeps
// draining. It is synchronous: a stalled filesystem still backpressures the TUI
// through the log write — acceptable for a local rotating file, and the bounded
// residual documented in the review.
type teeWriter struct {
	term io.Writer
	log  io.Writer
	live *atomic.Bool // when false (retired), terminal writes are suppressed
}

func (w teeWriter) Write(p []byte) (int, error) {
	if w.live == nil || w.live.Load() {
		_, _ = w.term.Write(p)
	}
	_, _ = w.log.Write(p)
	return len(p), nil
}
