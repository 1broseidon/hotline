package lifecycle

import (
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func TestProcessAlive(t *testing.T) {
	if !processAlive(os.Getpid()) {
		t.Fatal("the test process must report alive")
	}
	if processAlive(0) {
		t.Fatal("pid 0 is not a valid live process")
	}
	if processAlive(-1) {
		t.Fatal("negative pid is not alive")
	}

	// A reaped child pid should be reported dead. Spawn `true` and wait for it.
	cmd := exec.Command("true")
	if err := cmd.Start(); err != nil {
		t.Skipf("cannot spawn helper: %v", err)
	}
	pid := cmd.Process.Pid
	_ = cmd.Wait()
	if processAlive(pid) {
		t.Errorf("reaped pid %d should be dead", pid)
	}
}

func TestClaimAndReleasePollerSlot(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")

	if err := claimPollerSlot(pidFile); err != nil {
		t.Fatalf("claim: %v", err)
	}
	raw, err := os.ReadFile(pidFile)
	if err != nil {
		t.Fatalf("pid file not written: %v", err)
	}
	got, err := strconv.Atoi(strings.TrimSpace(string(raw)))
	if err != nil {
		t.Fatalf("pid file contents %q: %v", raw, err)
	}
	if got != os.Getpid() {
		t.Fatalf("pid file = %d, want %d", got, os.Getpid())
	}

	releasePollerSlot(pidFile)
	if _, err := os.Stat(pidFile); !os.IsNotExist(err) {
		t.Fatalf("pid file should be removed after release, stat err = %v", err)
	}
}

func TestClaimReplacesDeadStalePoller(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")

	// Seed with a definitely-dead pid (a reaped child).
	cmd := exec.Command("true")
	if err := cmd.Start(); err != nil {
		t.Skipf("cannot spawn helper: %v", err)
	}
	dead := cmd.Process.Pid
	_ = cmd.Wait()
	if err := os.WriteFile(pidFile, []byte(strconv.Itoa(dead)), 0o644); err != nil {
		t.Fatal(err)
	}

	// Claiming over a dead stale pid should succeed and record our pid.
	if err := claimPollerSlot(pidFile); err != nil {
		t.Fatalf("claim over dead stale: %v", err)
	}
	raw, _ := os.ReadFile(pidFile)
	if strings.TrimSpace(string(raw)) != strconv.Itoa(os.Getpid()) {
		t.Fatalf("pid file = %q, want our pid %d", raw, os.Getpid())
	}
}

func TestReleaseLeavesForeignPidFile(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	// A pid file owned by another process must not be removed by us.
	if err := os.WriteFile(pidFile, []byte("999999"), 0o644); err != nil {
		t.Fatal(err)
	}
	releasePollerSlot(pidFile)
	if _, err := os.Stat(pidFile); err != nil {
		t.Fatalf("foreign pid file should be left intact, got %v", err)
	}
}
