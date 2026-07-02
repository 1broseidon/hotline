// Package lifecycle owns process-level concerns: the single-poller PID guard,
// graceful shutdown cooperating with the SDK's ownership of stdio, the orphan
// watchdog, and the force-exit timer.
package lifecycle

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// claimPollerSlot ensures this process is the only Telegram poller for the
// token. Telegram allows exactly one getUpdates consumer per token; a crashed
// session can leave an orphan holding the slot forever (every new session then
// sees 409 Conflict). If a live stale poller is recorded, it is SIGTERM'd
// before we write our own pid (tmp + rename, 0644).
//
// The whole read-kill-write runs under a flock(LOCK_EX) on a sibling lock file
// (mirroring access.Mutate) so two near-simultaneous starts can't both proceed:
// the loser blocks until the winner has recorded its pid, then reads the
// winner's live pid and SIGTERMs it, leaving exactly one recorded poller.
func claimPollerSlot(pidFile string) error {
	lockPath := pidFile + ".lock"
	lf, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return err
	}
	defer lf.Close()
	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX); err != nil {
		return fmt.Errorf("locking %s: %w", lockPath, err)
	}
	defer syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)

	if raw, err := os.ReadFile(pidFile); err == nil {
		if stale, err := strconv.Atoi(strings.TrimSpace(string(raw))); err == nil {
			if stale > 1 && stale != os.Getpid() && processAlive(stale) {
				fmt.Fprintf(os.Stderr, "hotline: replacing stale poller pid=%d\n", stale)
				_ = syscall.Kill(stale, syscall.SIGTERM)
				// Give it a moment to release the getUpdates slot.
				time.Sleep(500 * time.Millisecond)
			}
		}
	}

	// Per-process unique temp name so concurrent writers (should the lock ever
	// be bypassed) never interleave on the same temp file before rename.
	tmp := fmt.Sprintf("%s.%d.tmp", pidFile, os.Getpid())
	if err := os.WriteFile(tmp, []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
		return err
	}
	if err := os.Rename(tmp, pidFile); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// processAlive reports whether a process with the given pid exists (signal 0).
func processAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	err := syscall.Kill(pid, 0)
	if err == nil {
		return true
	}
	// EPERM means the process exists but we can't signal it.
	return err == syscall.EPERM
}

// releasePollerSlot removes the pid file if it still records this process.
func releasePollerSlot(pidFile string) {
	if raw, err := os.ReadFile(pidFile); err == nil {
		if pid, err := strconv.Atoi(strings.TrimSpace(string(raw))); err == nil && pid == os.Getpid() {
			_ = os.Remove(pidFile)
		}
	}
}
