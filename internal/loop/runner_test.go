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

func TestRunnerSkipsUnapprovedLoop(t *testing.T) {
	r := testRunner(t)
	t.Setenv("HOTLINE_STATE_DIR", r.StateRoot)
	l, err := Add(r.Path, Loop{Label: "pending", Every: "10s", Cmd: "echo should-not-run"}, time.Now(), WithApprovalGate(r.StateRoot, false))
	if err != nil {
		t.Fatal(err)
	}
	if l.Approved {
		t.Fatal("test setup created approved loop")
	}
	res, err := r.RunOnce(context.Background(), l)
	if err != nil {
		t.Fatal(err)
	}
	if !res.Skipped || strings.Contains(res.Stdout, "should-not-run") {
		t.Fatalf("pending loop should skip without running: %+v", res)
	}
	d, _ := Load(r.Path)
	if d.Loops[0].Runs != 0 {
		t.Errorf("pending skip should not record a run: %+v", d.Loops[0])
	}
}

func TestRunnerContainsPanic(t *testing.T) {
	r := testRunner(t)
	// Inject a panic mid-run via the now seam: the first call sets start, the
	// second (inside runCommand's exit log) panics.
	calls := 0
	r.now = func() time.Time {
		calls++
		if calls == 2 {
			panic("boom")
		}
		return time.Now()
	}
	l, err := Add(r.Path, Loop{Label: "boom", Every: "10s", Cmd: "true"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	// Must not propagate: the barrier contains the panic and returns normally.
	if _, err := r.RunOnce(context.Background(), l); err != nil {
		t.Fatal(err)
	}
	r.mu.Lock()
	inflight := r.inflight[l.Label]
	r.mu.Unlock()
	if inflight {
		t.Fatal("in-flight flag not cleared after panic")
	}
	d, _ := Load(r.Path)
	if d.Loops[0].Runs != 1 || d.Loops[0].LastExit != 1 {
		t.Errorf("panic not recorded as failed run: %+v", d.Loops[0])
	}
	if buf := r.Log.(*bytes.Buffer); !strings.Contains(buf.String(), "boom") {
		t.Errorf("panic not logged with label + value: %q", buf.String())
	}
}

func TestRunnerUnresolvableSource(t *testing.T) {
	cases := []struct {
		name      string
		label     string
		notifyLLM bool
		wantErr   bool
	}{
		{name: "non-llm best-effort still runs", label: "besteffort", notifyLLM: false, wantErr: false},
		{name: "llm fail-closed blocks", label: "failclosed", notifyLLM: true, wantErr: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r := testRunner(t)
			// "ghost" is never registered, so the source cannot resolve.
			l, err := Add(r.Path, Loop{
				Label:     tc.label,
				Every:     "10s",
				Cmd:       "printf 'ran src=[%s]' \"$HOTLINE_NOTIFY_SOURCE\"",
				Source:    "ghost",
				NotifyLLM: tc.notifyLLM,
			}, time.Now())
			if err != nil {
				t.Fatal(err)
			}
			res, err := r.RunOnce(context.Background(), l)
			if tc.wantErr {
				if err == nil {
					t.Fatal("notify_llm=true must fail closed on an unresolvable source")
				}
				if strings.Contains(res.Stdout, "ran") {
					t.Errorf("cmd ran despite fail-closed source: %q", res.Stdout)
				}
				return
			}
			if err != nil {
				t.Fatalf("best-effort run errored: %v", err)
			}
			if !strings.Contains(res.Stdout, "ran src=[]") {
				t.Errorf("cmd stdout = %q, want run with empty HOTLINE_NOTIFY_SOURCE", res.Stdout)
			}
		})
	}
}

func TestLoopLogPath(t *testing.T) {
	root := t.TempDir()
	if got, want := filepath.Dir(LogPath(root, "a")), Dir(root); got != want {
		t.Errorf("log dir = %q, want %q", got, want)
	}
}
