//go:build !linux

package mcpchan

import "os/exec"

// setPdeathsig is a no-op on non-linux platforms: Pdeathsig is a Linux-specific
// SysProcAttr field. macOS teardown relies on the registry's closeAll instead.
func setPdeathsig(cmd *exec.Cmd) {}
