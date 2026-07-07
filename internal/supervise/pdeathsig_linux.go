//go:build linux

package supervise

import (
	"os/exec"
	"syscall"
)

// setPdeathsig asks the kernel to SIGTERM the harness when the supervisor
// (its parent thread) dies, so a crashed supervisor never leaves an orphan
// harness holding the Telegram poller slot. Linux-only; other platforms use
// the no-op in pdeathsig_other.go (mirrors mcpchan's exposure pattern).
func setPdeathsig(cmd *exec.Cmd) {
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Pdeathsig = syscall.SIGTERM
}
