//go:build linux || darwin

package supervise

import (
	"bytes"
	"io"
	"os"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
	"unsafe"
)

// activeAttacher builds an Attacher bridging a real pty slave (so TTY() is true
// without the test process's own controlling terminal). The returned master is
// the "keyboard/screen" side: write to it to simulate operator keystrokes, read
// from it to see what the attacher painted. cancelled reports whether the
// supervisor-cancel callback fired. Everything is torn down on cleanup.
func activeAttacher(t *testing.T, logw io.Writer) (a *Attacher, master *os.File, cancelled *int32) {
	t.Helper()
	m, s, err := openPTY()
	if err != nil {
		t.Fatal(err)
	}
	var flag int32
	a, err = NewAttacher(s, s, logw, func() { atomic.StoreInt32(&flag, 1) })
	if err != nil {
		m.Close()
		s.Close()
		t.Fatalf("NewAttacher over a pty slave: %v", err)
	}
	if !a.TTY() {
		t.Fatal("TTY() = false over a pty slave; the active path is untested")
	}
	t.Cleanup(func() {
		_ = a.Close()
		m.Close()
		s.Close()
	})
	return a, m, &flag
}

// TestDrainPTYTees is the fix in miniature: master output must reach BOTH the
// operator's terminal and the log. drainPTY writes to whatever io.Writer it is
// given, so the attacher hands it a MultiWriter; this verifies both halves
// receive every byte (no real tty needed — an os.Pipe stands in for the master).
func TestDrainPTYTees(t *testing.T) {
	pr, pw, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	var term, logw bytes.Buffer
	done := make(chan struct{})
	go func() {
		drainPTY(pr, io.MultiWriter(&term, &logw))
		close(done)
	}()
	if _, err := pw.WriteString("hello-tui"); err != nil {
		t.Fatal(err)
	}
	pw.Close() // EOF ends the drain
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("drain did not finish")
	}
	if term.String() != "hello-tui" {
		t.Errorf("terminal got %q, want the full stream", term.String())
	}
	if logw.String() != "hello-tui" {
		t.Errorf("log got %q, want the full stream (the log is how this bug was diagnosed)", logw.String())
	}
}

// TestAttacherNonTTYFallback: when stdin is not a terminal the attacher is
// passive — TTY() is false, raw mode is never engaged, and Start behaves exactly
// like StartOnPTY (child on a pty, output to the log only, the terminal writer
// untouched). This is the CI/piped path.
func TestAttacherNonTTYFallback(t *testing.T) {
	devnull, err := os.Open(os.DevNull) // not a terminal
	if err != nil {
		t.Fatal(err)
	}
	defer devnull.Close()
	outFile, err := os.CreateTemp(t.TempDir(), "out")
	if err != nil {
		t.Fatal(err)
	}
	defer outFile.Close()

	logw := &syncBuf{}
	a, err := NewAttacher(devnull, outFile, logw, func() {})
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	if a.TTY() {
		t.Fatal("TTY() = true for a non-terminal stdin; must be passive")
	}

	h, err := a.Start([]string{"/bin/sh", "-c", "echo NTOUT"}, t.TempDir(), os.Environ())
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h)

	deadline := time.Now().Add(2 * time.Second)
	for !strings.Contains(logw.String(), "NTOUT") {
		if time.Now().After(deadline) {
			t.Fatalf("log missing child output: %q", logw.String())
		}
		time.Sleep(5 * time.Millisecond)
	}
	if b, _ := os.ReadFile(outFile.Name()); len(b) != 0 {
		t.Errorf("non-tty path wrote %q to the terminal writer; it must stay untouched", b)
	}
}

// TestSetWinsizeRoundTrips exercises the winsize plumbing the attacher relies on
// for its initial sizing and SIGWINCH forwarding: a size set on the master is
// visible on the slave. No real tty required — openPTY gives both ends.
func TestSetWinsizeRoundTrips(t *testing.T) {
	master, slave, err := openPTY()
	if err != nil {
		t.Fatal(err)
	}
	defer master.Close()
	defer slave.Close()

	setWinsize(master, 123, 45) // cols, rows
	cols, rows := getWinsize(t, slave)
	if cols != 123 || rows != 45 {
		t.Errorf("slave sees %dx%d, want 123x45 forwarded from the master", cols, rows)
	}
}

// getWinsize reads the slave's window size via TIOCGWINSZ (the read side of the
// setWinsize ioctl), so the test can confirm what the master forwarded.
func getWinsize(t *testing.T, f *os.File) (cols, rows int) {
	t.Helper()
	var w struct{ rows, cols, x, y uint16 }
	if err := ioctl(f.Fd(), syscall.TIOCGWINSZ, uintptr(unsafe.Pointer(&w))); err != nil {
		t.Fatalf("TIOCGWINSZ: %v", err)
	}
	return int(w.cols), int(w.rows)
}

// TestMixedTTYIsPassive: a tty stdin with a non-tty stdout (the `hotline up
// >file` shape) must NOT attach — entering raw mode there would strand the TUI
// in the file and risk a SIGPIPE past the deferred restore. The attacher stays
// passive (TTY() == false), so the caller keeps the log-only StartOnPTY path.
func TestMixedTTYIsPassive(t *testing.T) {
	m, s, err := openPTY() // s (a pty slave) IS a terminal
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	defer s.Close()
	outFile, err := os.CreateTemp(t.TempDir(), "out") // a file — NOT a terminal
	if err != nil {
		t.Fatal(err)
	}
	defer outFile.Close()

	a, err := NewAttacher(s, outFile, &syncBuf{}, func() {})
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	if a.TTY() {
		t.Fatal("TTY() = true with a tty stdin but a file stdout; must require BOTH ends")
	}
}

// TestETXCancelsWhenNoLiveMaster: during a spawn-failure backoff there is no
// live generation, so raw mode would silently eat ^C and the operator could not
// stop the supervisor from its only terminal. An ETX (0x03) with no live master
// must trigger supervisor cancellation instead.
func TestETXCancelsWhenNoLiveMaster(t *testing.T) {
	a, master, cancelled := activeAttacher(t, &syncBuf{})
	// No Start() has been called: a.cur is nil (the backoff window).
	if _, err := master.Write([]byte{0x03}); err != nil {
		t.Fatal(err)
	}
	waitFlag(t, cancelled, "ETX with no live master must cancel the supervisor")
	_ = a
}

// TestStdinLossCancels: if the operator terminal disappears (pane killed →
// stdin read returns EIO/EOF) the supervisor must stop rather than run blind on
// a dead terminal.
func TestStdinLossCancels(t *testing.T) {
	a, master, cancelled := activeAttacher(t, &syncBuf{})
	master.Close() // the "terminal" goes away; the slave read now errors
	waitFlag(t, cancelled, "stdin EOF/EIO must cancel the supervisor")
	_ = a
}

// TestStartRetiresPreviousGeneration: on respawn the previous generation must be
// retired — its master closed and its output drain awaited — BEFORE the new one
// is exposed, so two drains never paint the terminal at once. White-box: after
// the second Start the first generation's drain has completed and a.cur points
// at the new generation.
func TestStartRetiresPreviousGeneration(t *testing.T) {
	a, _, _ := activeAttacher(t, &syncBuf{})

	h1, err := a.Start([]string{"/bin/sh", "-c", "exit 0"}, t.TempDir(), os.Environ())
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h1)
	a.mu.Lock()
	gen1 := a.cur
	a.mu.Unlock()
	if gen1 == nil {
		t.Fatal("first generation was not exposed")
	}
	if !gen1.live.Load() {
		t.Fatal("first generation should be live (able to paint the terminal) before retirement")
	}

	h2, err := a.Start([]string{"/bin/sh", "-c", "exit 0"}, t.TempDir(), os.Environ())
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h2)

	// The second Start retired the first generation synchronously before exposing
	// itself: gen1's terminal sink is silenced and its drain finished.
	if gen1.live.Load() {
		t.Error("previous generation still live after respawn: two drains could paint the terminal at once")
	}
	select {
	case <-gen1.drainDone:
	default:
		t.Error("previous generation's drain was not awaited before the new one was exposed")
	}
	a.mu.Lock()
	gen2 := a.cur
	a.mu.Unlock()
	if gen2 == nil || gen2 == gen1 {
		t.Fatalf("a.cur = %p, want a new generation distinct from %p", gen2, gen1)
	}
}

// TestTeeWriterIsolatesSinkError: the tee must attempt BOTH sinks and swallow a
// per-sink error, so a failing terminal write never starves harness.log (the log
// is how the original bug was diagnosed).
func TestTeeWriterIsolatesSinkError(t *testing.T) {
	var log bytes.Buffer
	w := teeWriter{term: errWriter{}, log: &log}
	n, err := w.Write([]byte("tui-bytes"))
	if err != nil {
		t.Fatalf("tee reported an error despite a failing sink: %v", err)
	}
	if n != len("tui-bytes") {
		t.Fatalf("tee wrote %d, want %d (drainPTY relies on a full count)", n, len("tui-bytes"))
	}
	if log.String() != "tui-bytes" {
		t.Errorf("log got %q; a terminal-sink error must not starve the log", log.String())
	}
}

// errWriter always fails, standing in for a broken terminal sink.
type errWriter struct{}

func (errWriter) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }

// waitFlag polls an atomically-set cancellation flag with a deadline.
func waitFlag(t *testing.T, flag *int32, msg string) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if atomic.LoadInt32(flag) == 1 {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal(msg)
}
