package lifecycle

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"
)

func TestClaimPollerSlotHeldLockRefuses(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	holder := holdPollerLock(t, pidFile+".lock")
	defer holder()
	if err := os.WriteFile(pidFile, []byte("4242"), 0o644); err != nil {
		t.Fatal(err)
	}
	defer swapUsurpWait(20*time.Millisecond, 5*time.Millisecond)()

	release, err := claimPollerSlot(pidFile)
	if release != nil {
		release()
		t.Fatal("contended claim returned a release function")
	}
	if err == nil || !strings.Contains(err.Error(), "poller slot") {
		t.Fatalf("err = %v, want poller slot refusal", err)
	}
	if raw, _ := os.ReadFile(pidFile); strings.TrimSpace(string(raw)) != "4242" {
		t.Fatalf("contended claim replaced diagnostic pid: %q", raw)
	}
}

func TestClaimPollerSlotReleasedDuringWaitSucceeds(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	lockFile, err := os.OpenFile(pidFile+".lock", os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if err := syscall.Flock(int(lockFile.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		t.Fatal(err)
	}
	go func() {
		time.Sleep(40 * time.Millisecond)
		_ = syscall.Flock(int(lockFile.Fd()), syscall.LOCK_UN)
		_ = lockFile.Close()
	}()
	defer swapUsurpWait(time.Second, 5*time.Millisecond)()

	release, err := claimPollerSlot(pidFile)
	if err != nil {
		t.Fatalf("claim after predecessor drained: %v", err)
	}
	defer release()
	assertOwnPID(t, pidFile)
}

func TestClaimPollerSlotReplacesStaleLiveLookingPIDWithoutLock(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	// Our own PID is necessarily live. Without a held flock it is still stale
	// advisory data and must not block replacement (PID reuse safe).
	if err := os.WriteFile(pidFile, []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
		t.Fatal(err)
	}

	release, err := claimPollerSlot(pidFile)
	if err != nil {
		t.Fatalf("stale live-looking pid blocked claim: %v", err)
	}
	defer release()
	assertOwnPID(t, pidFile)
}

func TestClaimPollerSlotReleaseRemovesOwnPIDAndUnlocks(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	release, err := claimPollerSlot(pidFile)
	if err != nil {
		t.Fatal(err)
	}
	assertOwnPID(t, pidFile)
	release()
	release() // sync.Once safe
	if _, err := os.Stat(pidFile); !os.IsNotExist(err) {
		t.Fatalf("pid file should be removed after release, stat err=%v", err)
	}

	// The closure released the kernel lock, not just the advisory file.
	nextRelease, err := claimPollerSlot(pidFile)
	if err != nil {
		t.Fatalf("claim after release: %v", err)
	}
	nextRelease()
}

func TestClaimPollerSlotReleasePreservesForeignPID(t *testing.T) {
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "bot.pid")
	release, err := claimPollerSlot(pidFile)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(pidFile, []byte("999999"), 0o644); err != nil {
		t.Fatal(err)
	}
	release()
	if raw, err := os.ReadFile(pidFile); err != nil || strings.TrimSpace(string(raw)) != "999999" {
		t.Fatalf("foreign pid advisory changed: raw=%q err=%v", raw, err)
	}
}

func holdPollerLock(t *testing.T, path string) func() {
	t.Helper()
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	return func() {
		_ = syscall.Flock(int(f.Fd()), syscall.LOCK_UN)
		_ = f.Close()
	}
}

func assertOwnPID(t *testing.T, pidFile string) {
	t.Helper()
	raw, err := os.ReadFile(pidFile)
	if err != nil {
		t.Fatalf("reading pid file: %v", err)
	}
	if got := strings.TrimSpace(string(raw)); got != strconv.Itoa(os.Getpid()) {
		t.Fatalf("pid file = %q, want %d", got, os.Getpid())
	}
}

func swapUsurpWait(wait, poll time.Duration) func() {
	oldWait, oldPoll := pollerUsurpWait, pollerUsurpPoll
	pollerUsurpWait, pollerUsurpPoll = wait, poll
	return func() { pollerUsurpWait, pollerUsurpPoll = oldWait, oldPoll }
}
