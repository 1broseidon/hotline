package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/supervise"
)

// Sample lines below are minimized-but-faithful copies of real events from a
// live pi box's <box-root>/supervisor/harness.log (scrubbed of long thinking/text
// bodies and device ids). The schema — field names and nesting — is verbatim.

func TestRenderEvent(t *testing.T) {
	cases := []struct {
		name    string
		line    string
		want    string // substring the rendered line must contain
		wantNil bool   // true if the event must render to nothing
	}{
		{
			name: "inbound user turn unwraps channel envelope",
			line: `{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"<channel source=\"app\" chat_id=\"app\" message_id=\"u-12299\" user=\"dev-abc\" ts=\"2026-07-16T03:31:27Z\">\nWe're good\n</channel>"}],"timestamp":1784171487000}}`,
			want: "◀ user: We're good",
		},
		{
			name: "assistant tool call renders arg digest",
			line: `{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"pondering"},{"type":"toolCall","id":"call_x","name":"reply","arguments":{"bubbles":["From the audit, the Gmail scripts were fine"],"chat_id":"dev-abc"}}],"model":"glm-5.2","usage":{"input":100,"output":5,"cacheRead":0,"totalTokens":105},"timestamp":1784171124990}}`,
			want: `⚙ reply {"bubbles":["From the audit`,
		},
		{
			name: "assistant text block renders model-prefixed reply",
			line: `{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Here is the summary you asked for."}],"model":"anthropic/claude-opus","usage":{"input":10,"output":8,"totalTokens":18},"timestamp":1784171124990}}`,
			want: "▶ claude-opus: Here is the summary you asked for.",
		},
		{
			name:    "assistant thinking-only content renders nothing",
			line:    `{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"just thinking"}],"model":"glm-5.2","timestamp":1784171124990}}`,
			wantNil: true,
		},
		{
			name:    "user message_end is a duplicate and renders nothing",
			line:    `{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"hi"}],"timestamp":1784171124990}}`,
			wantNil: true,
		},
		{
			name: "tool_execution_end ok shows result text digest",
			line: `{"type":"tool_execution_end","toolCallId":"call_x","toolName":"reply","result":{"content":[{"type":"text","text":"queued 9 bubble(s) (ids: a-12276, a-12278)"}],"details":{}},"isError":false}`,
			want: "✓ reply queued 9 bubble(s) (ids: a-12276, a-12278)",
		},
		{
			name: "tool_execution_end error uses cross glyph",
			line: `{"type":"tool_execution_end","toolCallId":"call_y","toolName":"schedule","result":{"content":[{"type":"text","text":"error: bad cron"}]},"isError":true}`,
			want: "✗ schedule error: bad cron",
		},
		{
			name: "turn_end shows usage digest",
			line: `{"type":"turn_end","message":{"role":"assistant","content":[],"model":"glm-5.2","usage":{"input":762,"output":3,"cacheRead":498112,"totalTokens":498877},"timestamp":1784171124990}}`,
			want: "── turn (762→3 tok, 498877 total)",
		},
		{
			name: "agent_end shows top-level usage",
			line: `{"type":"agent_end","messages":[],"model":"glm-5.2","usage":{"input":497450,"output":662,"totalTokens":498816}}`,
			want: "══ agent done (497450→662 tok, 498816 total)",
		},
		{
			name:    "tool_execution_start renders nothing (records duration only)",
			line:    `{"type":"tool_execution_start","toolCallId":"call_x","toolName":"reply","args":{"bubbles":["hi"]}}`,
			wantNil: true,
		},
		{
			name:    "message_update deltas are silent",
			line:    `{"type":"message_update","assistantMessageEvent":{"type":"toolcall_delta","delta":"ONG"}}`,
			wantNil: true,
		},
		{
			name:    "extension_ui_request is silent",
			line:    `{"type":"extension_ui_request","id":"ac18","method":"setStatus","statusKey":"hotline","statusText":"hotline: starting"}`,
			wantNil: true,
		},
		{
			name:    "turn_start is silent",
			line:    `{"type":"turn_start"}`,
			wantNil: true,
		},
		{
			name: "unknown event type falls through dim, never crashes",
			line: `{"type":"some_future_event","payload":{"a":1}}`,
			want: "· some_future_event",
		},
		{
			name: "plaintext supervisor line passes through dim",
			line: `hotline: harness=pi`,
			want: "· hotline: harness=pi",
		},
		{
			name: "pi INFO plaintext line passes through dim",
			line: `[hotline-pi 2026-07-16T04:15:13.030Z] INFO spawning hotline run`,
			want: "· [hotline-pi 2026-07-16T04:15:13.030Z] INFO spawning hotline run",
		},
		{
			name:    "truncated/garbage JSON does not crash and renders as plaintext",
			line:    `{"type":"message_end","message":{"role":"assis`,
			want:    "·",
			wantNil: false,
		},
		{
			name:    "empty line renders nothing",
			line:    "",
			wantNil: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r := newTailRenderer(false) // no color: deterministic assertions
			got := r.render(tc.line)
			if tc.wantNil {
				if got != "" {
					t.Fatalf("expected no output, got %q", got)
				}
				return
			}
			if !strings.Contains(got, tc.want) {
				t.Fatalf("render()\n  got:  %q\n  want substring: %q", got, tc.want)
			}
		})
	}
}

func TestRenderTimestampPrefix(t *testing.T) {
	r := newTailRenderer(false)
	out := r.render(`{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"hi"}],"timestamp":1784171487000}}`)
	// HH:MM:SS prefix, local tz — assert shape, not the tz-dependent value.
	prefix := strings.SplitN(out, " ", 2)[0]
	if _, err := time.Parse("15:04:05", prefix); err != nil {
		t.Fatalf("expected HH:MM:SS prefix, got %q (full: %q)", prefix, out)
	}
}

func TestRenderDurationDuringFollow(t *testing.T) {
	r := newTailRenderer(false)
	r.live = true
	base := time.Unix(1784171124, 0)
	clock := base
	r.now = func() time.Time { return clock }

	// start records the wall clock; end 1.5s later derives the duration.
	if out := r.render(`{"type":"tool_execution_start","toolCallId":"call_z","toolName":"Bash"}`); out != "" {
		t.Fatalf("start should be silent, got %q", out)
	}
	clock = base.Add(1500 * time.Millisecond)
	out := r.render(`{"type":"tool_execution_end","toolCallId":"call_z","toolName":"Bash","result":{"content":[{"type":"text","text":"done"}]},"isError":false}`)
	if !strings.Contains(out, "(1.5s)") {
		t.Fatalf("expected duration (1.5s) in %q", out)
	}
}

func TestRenderNoDurationDuringBackfill(t *testing.T) {
	r := newTailRenderer(false) // live=false (backfill)
	r.render(`{"type":"tool_execution_start","toolCallId":"call_z","toolName":"Bash"}`)
	out := r.render(`{"type":"tool_execution_end","toolCallId":"call_z","toolName":"Bash","result":{"content":[{"type":"text","text":"done"}]}}`)
	if strings.Contains(out, "(") {
		t.Fatalf("backfill must not fabricate a duration, got %q", out)
	}
}

func TestRenderColorWrapsANSI(t *testing.T) {
	r := newTailRenderer(true)
	out := r.render(`{"type":"tool_execution_end","toolName":"reply","result":{"content":[{"type":"text","text":"ok"}]},"isError":true}`)
	if !strings.Contains(out, cRed) || !strings.Contains(out, cReset) {
		t.Fatalf("expected red ANSI codes in error line, got %q", out)
	}
}

func TestJSONPassthroughFiltersPlaintext(t *testing.T) {
	if _, ok := jsonPassthrough(`hotline: harness=pi`); ok {
		t.Fatal("plaintext line must be filtered out of --json output")
	}
	if s, ok := jsonPassthrough(`{"type":"turn_start"}`); !ok || s != `{"type":"turn_start"}` {
		t.Fatalf("valid event should pass through verbatim, got %q ok=%v", s, ok)
	}
}

func TestUnwrapChannel(t *testing.T) {
	in := "<channel source=\"app\" ts=\"2026-07-16T03:31:27Z\">\nLet's recap\n</channel>"
	if got := unwrapChannel(in); got != "Let's recap" {
		t.Fatalf("unwrapChannel = %q, want %q", got, "Let's recap")
	}
	if got := unwrapChannel("plain text"); got != "plain text" {
		t.Fatalf("non-wrapped passthrough failed: %q", got)
	}
}

func TestShortModel(t *testing.T) {
	for in, want := range map[string]string{
		"glm-5.2":               "glm-5.2",
		"anthropic/claude-opus": "claude-opus",
		"":                      "assistant",
	} {
		if got := shortModel(in); got != want {
			t.Fatalf("shortModel(%q) = %q, want %q", in, got, want)
		}
	}
}

// ── file following ─────────────────────────────────────────────────────────

func TestCmdTailUsesSelectedBoxAndKeepsExplicitStateDirDirect(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Ada")
	boxRoot := filepath.Join(base, "bots", "Ada")

	writeLog := func(root, line string) {
		t.Helper()
		path := filepath.Join(supervise.Dir(root), supervise.HarnessLogName)
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(line+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	writeLog(base, "shared-root-log")
	writeLog(boxRoot, "named-box-log")

	var out, errout bytes.Buffer
	if code := cmdTail("", []string{"--no-follow", "-n", "1"}, &out, &errout); code != exitAccepted {
		t.Fatalf("default tail exit=%d stderr=%s", code, errout.String())
	}
	if !strings.Contains(out.String(), "named-box-log") || strings.Contains(out.String(), "shared-root-log") {
		t.Fatalf("default tail read wrong root: %q", out.String())
	}

	out.Reset()
	errout.Reset()
	if code := cmdTail("", []string{"--state-dir", base, "--no-follow", "-n", "1"}, &out, &errout); code != exitAccepted {
		t.Fatalf("explicit tail exit=%d stderr=%s", code, errout.String())
	}
	if !strings.Contains(out.String(), "shared-root-log") || strings.Contains(out.String(), "named-box-log") {
		t.Fatalf("explicit --state-dir lost direct-directory meaning: %q", out.String())
	}
}

func TestProjectMCPEnvTailTargetsNamedBox(t *testing.T) {
	baseRoot := setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram")
	t.Setenv("HOTLINE_BOT", "")
	os.Unsetenv("HOTLINE_BOT")
	projectDir := t.TempDir()
	writeMCPJSON(t, projectDir, map[string]string{
		"HOTLINE_PROVIDERS": "telegram:Ada",
		"HOTLINE_MC_DIR":    filepath.Join(baseRoot, "mc-ada"),
	})

	writeLog := func(root, line string) {
		t.Helper()
		path := filepath.Join(supervise.Dir(root), supervise.HarnessLogName)
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(line+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	writeLog(baseRoot, "base-log-must-not-appear")
	writeLog(filepath.Join(baseRoot, "bots", "Ada"), "ada-project-log")

	botName, _, args, cmd, _, err := resolveInvocation([]string{"tail", "--no-follow", "-n", "1"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if cmd != "tail" {
		t.Fatalf("resolved command = %q, want tail", cmd)
	}
	var out, errOut bytes.Buffer
	if code := cmdTail(botName, args[1:], &out, &errOut); code != exitAccepted {
		t.Fatalf("tail exit=%d stderr=%s", code, errOut.String())
	}
	if !strings.Contains(out.String(), "ada-project-log") || strings.Contains(out.String(), "base-log-must-not-appear") {
		t.Fatalf("project tail read wrong box: %q", out.String())
	}
}

func TestReadChunkPartialAndComplete(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "harness.log")

	// A complete line plus a partial (no trailing newline yet).
	if err := os.WriteFile(path, []byte("{\"type\":\"turn_start\"}\n{\"type\":\"partia"), 0o600); err != nil {
		t.Fatal(err)
	}
	lines, off, reset, err := readChunk(path, 0)
	if err != nil || reset {
		t.Fatalf("readChunk err=%v reset=%v", err, reset)
	}
	if len(lines) != 1 || lines[0] != `{"type":"turn_start"}` {
		t.Fatalf("expected 1 complete line, got %v", lines)
	}

	// The partial line completes on the next write; offset resumes from it.
	f, _ := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	f.WriteString("l\"}\n")
	f.Close()
	lines2, off2, _, err := readChunk(path, off)
	if err != nil {
		t.Fatal(err)
	}
	if len(lines2) != 1 || lines2[0] != `{"type":"partial"}` {
		t.Fatalf("expected the completed partial line, got %v", lines2)
	}
	if off2 <= off {
		t.Fatalf("offset should advance: %d -> %d", off, off2)
	}
}

func TestReadChunkDetectsRotation(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "harness.log")
	if err := os.WriteFile(path, []byte("line-a\nline-b\nline-c\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, off, _, err := readChunk(path, 0)
	if err != nil {
		t.Fatal(err)
	}

	// Rotation: the writer renamed the old file away and started a fresh,
	// smaller one. Size < offset must signal a reset so we re-read from 0.
	if err := os.WriteFile(path, []byte("fresh\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	lines, newOff, reset, err := readChunk(path, off)
	if err != nil {
		t.Fatal(err)
	}
	if !reset {
		t.Fatalf("expected reset on rotation (old off=%d, new size smaller)", off)
	}
	if newOff != 0 || lines != nil {
		t.Fatalf("reset should rewind to 0 with no lines, got off=%d lines=%v", newOff, lines)
	}
}

func TestSplitLines(t *testing.T) {
	if got := splitLines([]byte("a\nb\nc\n")); len(got) != 3 {
		t.Fatalf("expected 3 lines, got %v", got)
	}
	if got := splitLines([]byte("")); got != nil {
		t.Fatalf("empty buffer should yield nil, got %v", got)
	}
	if got := splitLines([]byte("solo")); len(got) != 1 || got[0] != "solo" {
		t.Fatalf("unterminated single line, got %v", got)
	}
}
