package main

import (
	"bytes"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
)

func loopState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	return dir
}

func TestCmdLoopAddAutoMintsSource(t *testing.T) {
	dir := loopState(t)
	var out bytes.Buffer
	if err := cmdLoop([]string{"add", "reddit-watch", "--every", "6h", "--notify-llm", "--level", "low", "--cmd", "echo hit"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatalf("add: %v", err)
	}
	d, err := loop.Load(loop.Path(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Loops) != 1 {
		t.Fatalf("loops = %d, want 1", len(d.Loops))
	}
	l := d.Loops[0]
	if l.Source != "reddit-watch" || !l.NotifyLLM || l.Level != "low" {
		t.Errorf("stored loop wrong: %+v", l)
	}
	if l.Approved {
		t.Error("normal loop add without -y should be pending")
	}
	reg, err := notify.LoadRegistry(notify.SourcesPath(dir))
	if err != nil {
		t.Fatal(err)
	}
	foundLoopSource := false
	for _, src := range reg.Sources {
		if src.Label == "reddit-watch" {
			foundLoopSource = true
		}
	}
	if !foundLoopSource {
		t.Fatalf("auto source not created: %+v", reg.Sources)
	}
	for _, src := range reg.Sources {
		if strings.Contains(readLoopFile(t, loop.Path(dir)), src.Key) {
			t.Fatal("loops.json must not store source keys")
		}
	}
	if !strings.Contains(out.String(), "Added notify source") {
		t.Errorf("operator output missing auto-mint note: %q", out.String())
	}
}

func TestCmdLoopAddApproveFlagCreatesApproved(t *testing.T) {
	dir := loopState(t)
	var out bytes.Buffer
	if err := cmdLoop([]string{"add", "watch", "--every", "1m", "--cmd", "true", "-y"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	d, _ := loop.Load(loop.Path(dir))
	if len(d.Loops) != 1 || !d.Loops[0].Approved {
		t.Fatalf("-y loop not approved: %+v", d.Loops)
	}
	if !strings.Contains(out.String(), "Approval: approved") {
		t.Errorf("output = %q", out.String())
	}
}

func TestCmdLoopAddRequiresExistingSource(t *testing.T) {
	loopState(t)
	var out bytes.Buffer
	err := cmdLoop([]string{"add", "watch", "--every=1m", "--cmd=true", "--notify-llm", "--source", "missing"}, &out, &bytes.Buffer{})
	if err == nil || !strings.Contains(err.Error(), "notify source") {
		t.Fatalf("missing source should error, got %v", err)
	}
}

func TestCmdLoopListPauseResumeRemove(t *testing.T) {
	dir := loopState(t)
	var out bytes.Buffer
	if err := cmdLoop([]string{"add", "watch", "--every", "1m", "--cmd", "echo hi"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	out.Reset()
	if err := cmdLoop([]string{"list"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "watch") || !strings.Contains(out.String(), "pending") || !strings.Contains(out.String(), "script-owned") {
		t.Errorf("list output = %q", out.String())
	}
	if err := cmdLoop([]string{"pause", "watch"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	d, _ := loop.Load(loop.Path(dir))
	if !d.Loops[0].Paused {
		t.Fatal("pause did not persist")
	}
	if err := cmdLoop([]string{"resume", "watch"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if err := cmdLoop([]string{"remove", "watch"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	d, _ = loop.Load(loop.Path(dir))
	if len(d.Loops) != 0 {
		t.Errorf("remove left loops: %+v", d.Loops)
	}
}

func TestCmdLoopApproveAndDeny(t *testing.T) {
	dir := loopState(t)
	var out bytes.Buffer
	if err := cmdLoop([]string{"add", "watch", "--every", "1m", "--cmd", "true"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if err := cmdLoop([]string{"approve", "watch"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	d, _ := loop.Load(loop.Path(dir))
	if !d.Loops[0].Approved {
		t.Fatal("approve did not flip approved")
	}
	if err := cmdLoop([]string{"deny", "watch"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	d, _ = loop.Load(loop.Path(dir))
	if len(d.Loops) != 0 {
		t.Fatalf("deny should remove loop: %+v", d.Loops)
	}
}

func TestCmdLoopRunExitPassthrough(t *testing.T) {
	loopState(t)
	var out, errout bytes.Buffer
	if err := cmdLoop([]string{"add", "fail", "--every", "1m", "--cmd", "exit 7", "-y"}, &out, &errout); err != nil {
		t.Fatal(err)
	}
	err := cmdLoop([]string{"run", "fail", "--once"}, &out, &errout)
	var coder interface{ Code() int }
	if !errors.As(err, &coder) || coder.Code() != 7 {
		t.Fatalf("run err = %v, want exit code 7", err)
	}
}

func readLoopFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}
