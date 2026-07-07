package supervise

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestStateRoundtripAndMissing(t *testing.T) {
	dir := t.TempDir()
	if st, err := ReadState(dir); err != nil || st != nil {
		t.Fatalf("missing state = (%v, %v), want (nil, nil)", st, err)
	}
	in := &State{PID: 42, Phase: PhaseRunning, HarnessPID: 43, Restarts: 2, LastExit: "exit status 1 after 3s"}
	if err := WriteState(dir, in); err != nil {
		t.Fatal(err)
	}
	out, err := ReadState(dir)
	if err != nil {
		t.Fatal(err)
	}
	if out.PID != 42 || out.Phase != PhaseRunning || out.HarnessPID != 43 || out.Restarts != 2 {
		t.Errorf("roundtrip mismatch: %+v", out)
	}
}

func TestLockLivenessProbe(t *testing.T) {
	dir := t.TempDir()
	if Running(dir) {
		t.Fatal("empty dir should not report running")
	}
	release, err := AcquireLock(dir)
	if err != nil {
		t.Fatal(err)
	}
	if !Running(dir) {
		t.Error("held lock should report running")
	}
	if _, err := AcquireLock(dir); err == nil {
		t.Error("second AcquireLock should fail while held")
	}
	release()
	if Running(dir) {
		t.Error("released lock should not report running")
	}
}

func TestRestartRequestRoundtrip(t *testing.T) {
	dir := t.TempDir()
	if _, ok := consumeRestart(dir); ok {
		t.Fatal("no request should be pending")
	}
	if err := RequestRestart(dir, "user\nasked\tnicely"); err != nil {
		t.Fatal(err)
	}
	reason, ok := consumeRestart(dir)
	if !ok {
		t.Fatal("request not found")
	}
	if reason != "user asked nicely" {
		t.Errorf("reason = %q, want flattened single line", reason)
	}
	if _, ok := consumeRestart(dir); ok {
		t.Error("request should be consumed exactly once")
	}
	if _, err := os.Stat(filepath.Join(dir, requestName)); !os.IsNotExist(err) {
		t.Error("request file should be removed")
	}
}

func TestRestartReasonBoundedAndDefaulted(t *testing.T) {
	dir := t.TempDir()
	if err := RequestRestart(dir, strings.Repeat("x", 5000)); err != nil {
		t.Fatal(err)
	}
	reason, _ := consumeRestart(dir)
	if len(reason) != 200 {
		t.Errorf("reason length = %d, want capped at 200", len(reason))
	}
	if err := RequestRestart(dir, "   "); err != nil {
		t.Fatal(err)
	}
	if reason, _ := consumeRestart(dir); reason != "(no reason given)" {
		t.Errorf("empty reason = %q", reason)
	}
}
