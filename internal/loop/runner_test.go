package loop

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func testRunner(t *testing.T) *Runner {
	t.Helper()
	root := t.TempDir()
	r := NewRunner(root)
	r.Path = Path(root)
	r.Log = &bytes.Buffer{}
	r.now = time.Now
	r.logMax = 1 << 20
	return r
}

func TestRunnerSkipsOverlap(t *testing.T) {
	r := testRunner(t)
	l, err := Add(r.Path, Loop{Label: "slow", Every: "10s", Cmd: "sleep 0.2; echo done"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}

	done := make(chan Result, 1)
	go func() {
		res, err := r.RunOnce(context.Background(), l)
		if err != nil {
			t.Errorf("first run: %v", err)
		}
		done <- res
	}()
	deadline := time.Now().Add(2 * time.Second)
	for {
		r.mu.Lock()
		inflight := r.inflight[l.Label]
		r.mu.Unlock()
		if inflight {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("run did not become inflight")
		}
		time.Sleep(5 * time.Millisecond)
	}
	res, err := r.RunOnce(context.Background(), l)
	if err != nil {
		t.Fatal(err)
	}
	if !res.Skipped {
		t.Fatal("second run should skip while first is still running")
	}
	first := <-done
	if first.ExitCode != 0 || !strings.Contains(first.Stdout, "done") {
		t.Errorf("first run = %+v", first)
	}
}

func TestRunnerTimeoutAndStateDirExport(t *testing.T) {
	r := testRunner(t)
	l, err := Add(r.Path, Loop{
		Label:   "env",
		Every:   "10s",
		Timeout: "100ms",
		Cmd:     "printf '%s|%s' \"$HOTLINE_LOOP_LABEL\" \"$HOTLINE_LOOP_STATE_DIR\"; sleep 5",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	res, err := r.RunOnce(context.Background(), l)
	if err != nil {
		t.Fatal(err)
	}
	if res.ExitCode != 124 {
		t.Fatalf("ExitCode = %d, want timeout 124", res.ExitCode)
	}
	wantState := StateDir(r.StateRoot, l.Label)
	if !strings.Contains(res.Stdout, "env|"+wantState) {
		t.Errorf("stdout %q missing exported label/state dir %q", res.Stdout, wantState)
	}
	info, err := os.Stat(wantState)
	if err != nil {
		t.Fatal(err)
	}
	if !info.IsDir() || info.Mode().Perm() != 0o700 {
		t.Errorf("state dir mode = %v, want dir 0700", info.Mode())
	}
	d, _ := Load(r.Path)
	if d.Loops[0].LastExit != 124 || d.Loops[0].Runs != 1 {
		t.Errorf("status not recorded: %+v", d.Loops[0])
	}
	if _, err := os.Stat(LogPath(r.StateRoot, l.Label)); err != nil {
		t.Errorf("log not written: %v", err)
	}
}

func TestRunnerExportsNotifySourceKey(t *testing.T) {
	r := testRunner(t)
	src, err := notifyTestSource(r.StateRoot, "src")
	if err != nil {
		t.Fatal(err)
	}
	l, err := Add(r.Path, Loop{Label: "source-env", Every: "10s", Cmd: "printf %s \"$HOTLINE_NOTIFY_SOURCE\"", Source: src.Label}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	res, err := r.RunOnce(context.Background(), l)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(res.Stdout) != src.Key {
		t.Errorf("HOTLINE_NOTIFY_SOURCE = %q, want key", res.Stdout)
	}
}

func TestLoopLogPath(t *testing.T) {
	root := t.TempDir()
	if got, want := filepath.Dir(LogPath(root, "a")), Dir(root); got != want {
		t.Errorf("log dir = %q, want %q", got, want)
	}
}
