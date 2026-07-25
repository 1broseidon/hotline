// Package lifecycle owns process-level concerns: lifetime filesystem guards,
// graceful shutdown cooperating with the SDK's ownership of stdio, the orphan
// watchdog, and the force-exit timer.
package lifecycle

import (
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// pollerUsurpWait bounds how long claimPollerSlot waits for a predecessor's
// lifetime flock to drain before deciding the slot is genuinely contended. It
// is a package var (not a const) so tests can shrink it.
var pollerUsurpWait = 3 * time.Second

// pollerUsurpPoll is the nonblocking flock retry interval during that wait.
var pollerUsurpPoll = 50 * time.Millisecond

// claimPollerSlot takes and returns a lifetime-held flock for one provider
// consumer. bot.pid remains a diagnostic advisory only: PID liveness is never
// checked, so a stale or PID-reused value cannot block a safe replacement.
func claimPollerSlot(pidFile string) (func(), error) {
	lockPath := pidFile + ".lock"
	lf, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}

	deadline := time.Now().Add(pollerUsurpWait)
	for {
		err = syscall.Flock(int(lf.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			break
		}
		if !errors.Is(err, syscall.EWOULDBLOCK) && !errors.Is(err, syscall.EAGAIN) {
			_ = lf.Close()
			return nil, fmt.Errorf("locking %s: %w", lockPath, err)
		}
		if pollerUsurpWait <= 0 || !time.Now().Before(deadline) {
			detail := pollerPIDDetail(pidFile)
			_ = lf.Close()
			return nil, fmt.Errorf("poller slot already held by another hotline provider%s; refusing to start a second consumer", detail)
		}
		poll := pollerUsurpPoll
		if poll <= 0 {
			poll = 50 * time.Millisecond
		}
		remaining := time.Until(deadline)
		if poll > remaining {
			poll = remaining
		}
		if poll > 0 {
			time.Sleep(poll)
		}
	}

	if err := writePollerPID(pidFile); err != nil {
		_ = syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
		_ = lf.Close()
		return nil, err
	}

	var once sync.Once
	release := func() {
		once.Do(func() {
			// Remove our advisory while the lifetime lock is still held. If another
			// writer replaced it, releasePollerSlot deliberately preserves it.
			releasePollerSlot(pidFile)
			_ = syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
			_ = lf.Close()
		})
	}
	return release, nil
}

func writePollerPID(pidFile string) error {
	// Per-process unique temp name so a bypassed/legacy writer cannot interleave
	// bytes with us. The lifetime flock, not this rename, is liveness truth.
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

func pollerPIDDetail(pidFile string) string {
	raw, err := os.ReadFile(pidFile)
	if err != nil {
		return ""
	}
	pid := strings.TrimSpace(string(raw))
	if pid == "" {
		return ""
	}
	return fmt.Sprintf(" (bot.pid reports pid %s)", pid)
}

// releasePollerSlot removes the pid advisory if it still records this process.
// It does not release the lifetime flock; the closure returned by
// ClaimPollerSlot does that, and process exit is the final kernel backstop.
func releasePollerSlot(pidFile string) {
	if raw, err := os.ReadFile(pidFile); err == nil {
		if pid, err := strconv.Atoi(strings.TrimSpace(string(raw))); err == nil && pid == os.Getpid() {
			_ = os.Remove(pidFile)
		}
	}
}
