//go:build !linux

package supervise

import "os/exec"

// setPdeathsig is a no-op off Linux: Pdeathsig is a Linux-specific
// SysProcAttr field. Elsewhere an orphaned harness is recovered by the
// stale-poller claim on the next `hotline up`.
func setPdeathsig(cmd *exec.Cmd) {}
