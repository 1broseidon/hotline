//go:build linux

package mcpchan

import (
	"os/exec"
	"syscall"
)

// setPdeathsig asks the kernel to send SIGTERM to the child when the hotline
// process (its parent thread) dies, so a tunnel subprocess dies with hotline
// instead of orphaning to init. Linux-only; other platforms use the no-op in
// exposure_other.go.
func setPdeathsig(cmd *exec.Cmd) {
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Pdeathsig = syscall.SIGTERM
}
