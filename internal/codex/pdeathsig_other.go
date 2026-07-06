//go:build !linux

package codex

import "os/exec"

func setPdeathsig(cmd *exec.Cmd) {}
