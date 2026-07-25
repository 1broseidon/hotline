//go:build linux

package supervise

import (
	"syscall"
	"testing"
	"unsafe"
)

// echoEnabled reads the terminal's ECHO lflag via TCGETS. Raw mode clears it;
// a restored (cooked) terminal has it set. It is a concrete, deterministic proxy
// for "the terminal was put back the way we found it".
func echoEnabled(t *testing.T, fd uintptr) bool {
	t.Helper()
	var tio syscall.Termios
	if err := ioctl(fd, syscall.TCGETS, uintptr(unsafe.Pointer(&tio))); err != nil {
		t.Fatalf("TCGETS: %v", err)
	}
	return tio.Lflag&syscall.ECHO != 0
}

// TestAttacherRestoresTerminalOnClose: NewAttacher puts the terminal into raw
// mode (ECHO off); Close must restore it (ECHO back on) and stay idempotent.
// This is the mechanically-testable half of the terminal-restoration guarantee
// — the signal/panic windows it also closes need a live tty and are covered by
// code ordering (handler installed before MakeRaw) rather than a unit test.
func TestAttacherRestoresTerminalOnClose(t *testing.T) {
	m, s, err := openPTY()
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()
	defer s.Close()

	if !echoEnabled(t, s.Fd()) {
		t.Fatal("fresh pty slave should start with ECHO set")
	}

	a, err := NewAttacher(s, s, &syncBuf{}, func() {})
	if err != nil {
		t.Fatal(err)
	}
	if !a.TTY() {
		t.Fatal("expected an active attacher over a pty slave")
	}
	if echoEnabled(t, s.Fd()) {
		t.Fatal("raw mode should have cleared ECHO")
	}

	if err := a.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !echoEnabled(t, s.Fd()) {
		t.Error("Close did not restore the terminal (ECHO still cleared)")
	}
	if err := a.Close(); err != nil {
		t.Errorf("second Close should be a no-op, got %v", err)
	}
}
