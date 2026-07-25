//go:build !linux && !darwin

package supervise

import (
	"io"
	"os"
)

// Attacher on unsupported platforms is inert: it never claims a terminal, so
// TTY() is false and cmd_up falls back to StartOnPTY (which itself returns
// ErrUnsupported here). This keeps `hotline up`'s claude path compiling and its
// behavior on these platforms exactly as before the attached-TUI change.
type Attacher struct{}

func NewAttacher(in, out *os.File, logw io.Writer, cancel func()) (*Attacher, error) {
	return &Attacher{}, nil
}

func (a *Attacher) TTY() bool { return false }

func (a *Attacher) Start(argv []string, dir string, env []string) (Harness, error) {
	return nil, ErrUnsupported
}

func (a *Attacher) Close() error { return nil }
