//go:build !linux && !darwin

package supervise

import "io"

// StartOnPTY is unavailable without a pty implementation; `hotline up` fails
// loudly on these platforms instead of silently crash-looping a harness that
// can't get a terminal.
func StartOnPTY(argv []string, dir string, env []string, logw io.Writer) (Harness, error) {
	return nil, ErrUnsupported
}

// StartPiped shares StartOnPTY's unix process-group/Pdeathsig discipline
// (Setsid, group signalling), so it is gated to the same platforms.
func StartPiped(argv []string, dir string, env []string, logw io.Writer) (Harness, error) {
	return nil, ErrUnsupported
}
