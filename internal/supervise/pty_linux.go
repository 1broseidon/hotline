//go:build linux

package supervise

import (
	"fmt"
	"os"
	"syscall"
	"unsafe"
)

// openPTY allocates a pseudo-terminal pair via /dev/ptmx: unlock the slave
// (TIOCSPTLCK), resolve its number (TIOCGPTN), open /dev/pts/N. Pure
// syscalls, no cgo, no dependencies; grantpt is a no-op with devpts.
func openPTY() (master, slave *os.File, err error) {
	m, err := os.OpenFile("/dev/ptmx", os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		return nil, nil, fmt.Errorf("opening /dev/ptmx: %w", err)
	}
	var unlock int32
	if err := ioctl(m.Fd(), syscall.TIOCSPTLCK, uintptr(unsafe.Pointer(&unlock))); err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("unlockpt: %w", err)
	}
	var n uint32
	if err := ioctl(m.Fd(), syscall.TIOCGPTN, uintptr(unsafe.Pointer(&n))); err != nil {
		m.Close()
		return nil, nil, fmt.Errorf("ptsname: %w", err)
	}
	s, err := os.OpenFile(fmt.Sprintf("/dev/pts/%d", n), os.O_RDWR|syscall.O_NOCTTY, 0)
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
