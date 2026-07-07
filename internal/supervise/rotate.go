package supervise

import (
	"os"
	"sync"
)

// RotatingWriter is a size-capped append log: when a write would push the
// file past max bytes, the current file is renamed to path+".1" (replacing
// any previous rotation) and a fresh file is started. Total disk use is thus
// bounded at ~2x max. It exists for the harness pty log — raw TUI output
// that grows without an operator watching it.
type RotatingWriter struct {
	mu   sync.Mutex
	path string
	max  int64
	f    *os.File
	size int64
}

// NewRotatingWriter opens (appending) or creates the log at path with the
// given size cap in bytes.
func NewRotatingWriter(path string, max int64) (*RotatingWriter, error) {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return nil, err
	}
	info, err := f.Stat()
	if err != nil {
		f.Close()
		return nil, err
	}
	return &RotatingWriter{path: path, max: max, f: f, size: info.Size()}, nil
}

func (w *RotatingWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.size > 0 && w.size+int64(len(p)) > w.max {
		if err := w.rotateLocked(); err != nil {
			return 0, err
		}
	}
	n, err := w.f.Write(p)
	w.size += int64(n)
	return n, err
}

func (w *RotatingWriter) rotateLocked() error {
	w.f.Close()
	_ = os.Rename(w.path, w.path+".1") // best-effort; a failed rename just truncates in place
	f, err := os.OpenFile(w.path, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600)
	if err != nil {
		return err
	}
	w.f, w.size = f, 0
	return nil
}

// Close closes the underlying file.
func (w *RotatingWriter) Close() error {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.f.Close()
}
