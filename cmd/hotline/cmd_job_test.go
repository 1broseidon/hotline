package main

import (
	"bytes"
	"testing"

	"github.com/1broseidon/hotline/internal/jobspool"
)

func TestCmdJobUsageErrors(t *testing.T) {
	cases := []struct {
		name string
		args []string
	}{
		{"no action", nil},
		{"bad action", []string{"frobnicate", "--cookie", "c1"}},
		{"missing cookie", []string{"start", "--title", "t"}},
		{"start without title", []string{"start", "--cookie", "c1"}},
		{"bad state", []string{"done", "--cookie", "c1", "--state", "sideways"}},
		{"bad progress", []string{"update", "--cookie", "c1", "--progress", "9"}},
		{"unknown flag", []string{"start", "--cookie", "c1", "--title", "t", "--wat", "x"}},
		{"dangling value", []string{"start", "--cookie"}},
		{"neither cookie nor agent", []string{"done", "--state", "ok"}},
		{"agent addressing a start", []string{"start", "--agent", "a1", "--title", "t"}},
		{"agent addressing an update", []string{"update", "--agent", "a1"}},
		{"dangling agent value", []string{"done", "--agent"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
			t.Setenv("HOTLINE_PROVIDERS", "")
			var out, errb bytes.Buffer
			if code := cmdJob("", tc.args, &out, &errb); code != exitUsage {
				t.Errorf("cmdJob(%v) = %d, want %d (usage)", tc.args, code, exitUsage)
			}
		})
	}
}

func TestCmdJobEnqueuesIntent(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_PROVIDERS", "")
	var out, errb bytes.Buffer

	steps := [][]string{
		{"start", "--cookie", "toolu_1", "--batch", "sess", "--title", "Explore auth", "--detail", "reading"},
		{"update", "--cookie", "toolu_1", "--batch", "sess", "--progress", "0.5"},
		{"done", "--cookie=toolu_1", "--batch=sess", "--state=ok", "--notify=found it"},
	}
	for _, s := range steps {
		if code := cmdJob("", s, &out, &errb); code != exitAccepted {
			t.Fatalf("cmdJob(%v) = %d (%s)", s, code, errb.String())
		}
	}

	sp, err := jobspool.LoadSpool(jobspool.SpoolPath(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(sp.Pending) != 3 {
		t.Fatalf("want 3 pending intents, got %d", len(sp.Pending))
	}
	got := sp.Pending
	if got[0].Action != "start" || got[0].Cookie != "toolu_1" || got[0].Batch != "sess" || got[0].Title != "Explore auth" {
		t.Fatalf("start intent = %+v", got[0])
	}
	if got[1].Action != "update" || got[1].Progress == nil || *got[1].Progress != 0.5 {
		t.Fatalf("update intent = %+v", got[1])
	}
	if got[2].Action != "done" || got[2].State != "ok" || got[2].Notify != "found it" {
		t.Fatalf("done intent = %+v (both --flag=value and --flag value forms must parse)", got[2])
	}
	// Seq is monotonic across appends.
	if got[0].Seq != 1 || got[1].Seq != 2 || got[2].Seq != 3 {
		t.Fatalf("seq not monotonic: %d,%d,%d", got[0].Seq, got[1].Seq, got[2].Seq)
	}
}

func TestCmdJobDefaultsChatToApp(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_PROVIDERS", "")
	var out, errb bytes.Buffer
	if code := cmdJob("", []string{"start", "--cookie", "c", "--title", "t"}, &out, &errb); code != exitAccepted {
		t.Fatalf("exit %d: %s", code, errb.String())
	}
	sp, _ := jobspool.LoadSpool(jobspool.SpoolPath(dir))
	if sp.Pending[0].ChatID != "app" {
		t.Fatalf("chat id = %q, want app", sp.Pending[0].ChatID)
	}
}

// TestCmdJobAgentAddressing covers the correlators an asynchronous harness needs:
// an update BINDS the completion-side agent id to the dispatch cookie, and a
// done may then address the job by that agent id alone — the only id the event
// reporting a background subagent's completion actually carries.
func TestCmdJobAgentAddressing(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_PROVIDERS", "")
	var out, errb bytes.Buffer

	steps := [][]string{
		{"start", "--cookie", "toolu_1", "--batch", "sess", "--title", "Explore auth"},
		{"update", "--cookie", "toolu_1", "--batch", "sess", "--agent", "agent_a1"},
		{"done", "--agent", "agent_a1", "--state", "ok"},
	}
	for _, s := range steps {
		if code := cmdJob("", s, &out, &errb); code != exitAccepted {
			t.Fatalf("cmdJob(%v) = %d (%s)", s, code, errb.String())
		}
	}

	sp, err := jobspool.LoadSpool(jobspool.SpoolPath(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(sp.Pending) != 3 {
		t.Fatalf("want 3 pending intents, got %d", len(sp.Pending))
	}
	if got := sp.Pending[1]; got.Agent != "agent_a1" || got.Cookie != "toolu_1" {
		t.Fatalf("binding update = %+v", got)
	}
	if got := sp.Pending[2]; got.Agent != "agent_a1" || got.Cookie != "" {
		t.Fatalf("agent-addressed done = %+v", got)
	}
}
