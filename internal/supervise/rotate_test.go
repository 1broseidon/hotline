package supervise

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

func TestRotatingWriterBoundsGrowth(t *testing.T) {
	path := filepath.Join(t.TempDir(), "harness.log")
	w, err := NewRotatingWriter(path, 100)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	chunk := bytes.Repeat([]byte("a"), 60)
	for i := 0; i < 3; i++ {
		if _, err := w.Write(chunk); err != nil {
			t.Fatal(err)
		}
	}
	// Write 3 rotated at writes 2 and 3: active and .1 each hold one chunk.
	if info, err := os.Stat(path); err != nil || info.Size() != 60 {
		t.Errorf("active log size = %v (err %v), want 60", info.Size(), err)
	}
	if info, err := os.Stat(path + ".1"); err != nil || info.Size() != 60 {
		t.Errorf("rotated log size/err = %v/%v, want 60", info, err)
	}
	if _, err := os.Stat(path + ".2"); !os.IsNotExist(err) {
		t.Error("only one rotation generation should be kept")
	}
}

func TestRotatingWriterReopensAppending(t *testing.T) {
	path := filepath.Join(t.TempDir(), "harness.log")
	w, _ := NewRotatingWriter(path, 100)
	w.Write([]byte("first."))
	w.Close()
	w2, err := NewRotatingWriter(path, 100)
	if err != nil {
		t.Fatal(err)
	}
	w2.Write([]byte("second."))
	w2.Close()
	data, _ := os.ReadFile(path)
	if string(data) != "first.second." {
		t.Errorf("append across reopen: %q", data)
	}
}
