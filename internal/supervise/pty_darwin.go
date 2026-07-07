//go:build darwin

package supervise

import (
	"fmt"
	"os"
	"syscall"
	"unsafe"
)

// Darwin pty ioctls (from <sys/ttycom.h>); the syscall package doesn't export
// them.
const (
	tiocptygname = 0x40807453 // _IOR('t', 83, [128]byte) — slave device name
	tiocptygrant = 0x20007454 // _IO('t', 84) — grantpt
	tiocptyunlk  = 0x20007452 // _IO('t', 82) — unlockpt
)

// openPTY allocates a pseudo-terminal pair via /dev/ptmx using the Darwin
// grant/unlock/name ioctls. Pure syscalls, no cgo, no dependencies.
func openPTY() (master, slave *os.File, err error) {
	m, err := os.OpenFile("/dev/ptmx", os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		return nil, nil, fmt.Errorf("opening /dev/ptmx: %w", err)
	}
	if err := ioctl(m.Fd(), tiocptygrant, 0); err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("grantpt: %w", err)
	}
	if err := ioctl(m.Fd(), tiocptyunlk, 0); err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("unlockpt: %w", err)
	}
	var buf [128]byte
	if err := ioctl(m.Fd(), tiocptygname, uintptr(unsafe.Pointer(&buf[0]))); err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("ptsname: %w", err)
	}
	name := buf[:]
	for i, b := range name {
		if b == 0 {
			name = name[:i]
			break
		}
	}
	s, err := os.OpenFile(string(name), os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("opening pty slave: %w", err)
	}
	return m, s, nil
}

// setWinsize gives the pty a sane geometry so the harness TUI doesn't render
// into a 0x0 terminal.
func setWinsize(f *os.File, cols, rows int) {
	w := struct{ rows, cols, x, y uint16 }{uint16(rows), uint16(cols), 0, 0}
	_ = ioctl(f.Fd(), syscall.TIOCSWINSZ, uintptr(unsafe.Pointer(&w)))
}

func ioctl(fd, req, arg uintptr) error {
	if _, _, e := syscall.Syscall(syscall.SYS_IOCTL, fd, req, arg); e != 0 {
		return e
	}
	return nil
}
