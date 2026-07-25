package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/loop"
)

// TestSmokeTokenless drives the built binary over a stdio pipe with no bot
// token: it runs the MCP handshake, asserts the advertised capabilities and the
// tool list, calls reply (expecting an isError result, not a protocol error),
// and confirms a clean EOF shutdown. No network is used.
func TestSmokeTokenless(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping integration smoke in -short mode")
	}

	dir := t.TempDir()
	bin := filepath.Join(dir, "hotline")

	build := exec.Command("go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build failed: %v\n%s", err, out)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	cmd := exec.CommandContext(ctx, bin, "run")
	// The smoke asserts the default single-provider, tokenless Claude surface.
	// Scrub ambient daemon/operator posture before pinning that contract so a
	// developer's live multi-provider/core/YOLO shell cannot reshape the child.
	cmd.Env = append(scrubEnv(cmd.Environ(),
		"HOTLINE_PROVIDERS",
		"HOTLINE_HARNESS",
		"HOTLINE_SUPERVISOR_DIR",
		"HOTLINE_CORE_MODE",
		"HOTLINE_CORE_URL",
		"HOTLINE_YOLO",
		"HOTLINE_APP_PUSH",
		"HOTLINE_MISSION_CONTROL",
	),
		"TELEGRAM_BOT_TOKEN=",
		"TELEGRAM_STATE_DIR="+dir,
		"TELE_GO_STATE_DIR="+dir,
		"HOTLINE_STATE_DIR="+dir,
		"HOTLINE_PROVIDERS=telegram",
		"HOTLINE_HARNESS=claude",
		"HOTLINE_CORE_MODE=0",
		"HOTLINE_YOLO=0",
		"HOTLINE_APP_PUSH=0",
		"HOTLINE_MISSION_CONTROL=1",
	)
	stdin, err := cmd.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	cmd.Stderr = io.Discard

	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}

	r := bufio.NewReader(stdout)
	send := func(v any) {
		b, _ := json.Marshal(v)
		b = append(b, '\n')
		if _, werr := stdin.Write(b); werr != nil {
			t.Fatalf("write: %v", werr)
		}
	}
	// readResult reads newline-delimited JSON until a response with the given id
	// appears, skipping notifications and unrelated frames.
	readResult := func(id float64) map[string]any {
		deadline := time.Now().Add(15 * time.Second)
		for time.Now().Before(deadline) {
			line, rerr := r.ReadBytes('\n')
			if len(line) > 0 {
				var m map[string]any
				if json.Unmarshal(line, &m) == nil {
					if rid, ok := m["id"].(float64); ok && rid == id {
						return m
					}
				}
			}
			if rerr != nil {
				t.Fatalf("read (waiting for id %v): %v", id, rerr)
			}
		}
		t.Fatalf("timed out waiting for response id %v", id)
		return nil
	}

	// 1. initialize
	send(map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "initialize",
		"params": map[string]any{
			"protocolVersion": "2025-06-18",
			"capabilities":    map[string]any{},
			"clientInfo":      map[string]any{"name": "smoke", "version": "0"},
		},
	})
	initResp := readResult(1)
	result, _ := initResp["result"].(map[string]any)
	caps, _ := result["capabilities"].(map[string]any)
	exp, _ := caps["experimental"].(map[string]any)
	if _, ok := exp["claude/channel"]; !ok {
		t.Errorf("missing experimental claude/channel capability; got %v", exp)
	}
	if _, ok := exp["claude/channel/permission"]; ok {
		t.Errorf("claude/channel/permission must NOT be advertised without a token; got %v", exp)
	}
	if caps["tools"] == nil {
		t.Errorf("tools capability should be inferred; got %v", caps)
	}

	// 2. initialized notification
	send(map[string]any{"jsonrpc": "2.0", "method": "notifications/initialized"})

	// 3. tools/list
	send(map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
	listResp := readResult(2)
	lr, _ := listResp["result"].(map[string]any)
	toolsArr, _ := lr["tools"].([]any)
	got := map[string]bool{}
	for _, ti := range toolsArr {
		if tm, ok := ti.(map[string]any); ok {
			if name, ok := tm["name"].(string); ok {
				got[name] = true
			}
			// Single-provider wire compat: the default (telegram-only) config
			// must not grow a "source" property in any tool schema via
			// withSourceProperty. Tools that own their own "source" fields are
			// exempt: schedule routes provider source; setup_loop routes notify
			// source labels.
			if name, _ := tm["name"].(string); name != "schedule" && name != "setup_loop" {
				if schema, ok := tm["inputSchema"].(map[string]any); ok {
					if props, ok := schema["properties"].(map[string]any); ok {
						if _, has := props["source"]; has {
							t.Errorf("tool %v schema must not expose source with a single provider", tm["name"])
						}
					}
				}
			}
		}
	}
	// mission mounts by default now that Mission Control is ON for every harness
	// (resolved call #3): a tokenless claude run seeds mc/ and registers the tool.
	for _, want := range []string{"reply", "react", "edit_message", "download_attachment", "publish", "schedule", "list_schedules", "setup_loop", "list_loops", "setup_notify", "job", "mission"} {
		if !got[want] {
			t.Errorf("tools/list missing %q; got %v", want, got)
		}
	}
	if len(got) != 12 {
		t.Errorf("expected exactly 12 tools, got %d: %v", len(got), got)
	}

	// 4. tools/call reply -> isError result, but a successful JSON-RPC response.
	send(map[string]any{
		"jsonrpc": "2.0", "id": 3, "method": "tools/call",
		"params": map[string]any{
			"name":      "reply",
			"arguments": map[string]any{"chat_id": "1", "text": "hi"},
		},
	})
	callResp := readResult(3)
	if callResp["error"] != nil {
		t.Errorf("tools/call should not produce a JSON-RPC error; got %v", callResp["error"])
	}
	cr, _ := callResp["result"].(map[string]any)
	if isErr, _ := cr["isError"].(bool); !isErr {
		t.Errorf("reply without token should be isError; got %v", cr)
	}
	content, _ := cr["content"].([]any)
	foundMsg := false
	for _, c := range content {
		if cm, ok := c.(map[string]any); ok {
			if txt, ok := cm["text"].(string); ok && containsSub(txt, "no bot token configured") {
				foundMsg = true
			}
		}
	}
	if !foundMsg {
		t.Errorf("expected 'no bot token configured' in reply result; got %v", content)
	}

	// 5. Close stdin -> clean EOF shutdown within ~2s.
	_ = stdin.Close()
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case <-done:
		// exited (force-exit timer calls os.Exit(0)); success.
	case <-time.After(5 * time.Second):
		_ = cmd.Process.Kill()
		t.Fatal("process did not exit within 5s of stdin EOF")
	}
}

// TestRunHostsLoopTickerAndRefusesSecondRuntime proves the box-owned ticker is
// live under a plain `hotline run` process (without a supervisor) and that a
// second process on the same box root is refused before it can double-tick.
func TestRunHostsLoopTickerAndRefusesSecondRuntime(t *testing.T) {
	if testing.Short() {
		t.Skip("skipping process integration test in -short mode")
	}

	stateRoot := t.TempDir()
	bin := filepath.Join(t.TempDir(), "hotline")
	build := exec.Command("go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build failed: %v\n%s", err, out)
	}

	const label = "run-hosted"
	marker := filepath.Join(loop.StateDir(stateRoot, label), "ticks")
	if _, err := loop.Add(loop.Path(stateRoot), loop.Loop{
		Label: label,
		Every: "10s",
		Cmd:   "printf 'tick\\n' >> \"$HOTLINE_LOOP_STATE_DIR/ticks\"",
	}, time.Now()); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	first := exec.CommandContext(ctx, bin, "run")
	first.Env = tokenlessRunEnv(first, stateRoot)
	stdin, err := first.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	first.Stdout = io.Discard
	var firstStderr bytes.Buffer
	first.Stderr = &firstStderr
	if err := first.Start(); err != nil {
		t.Fatal(err)
	}
	firstDone := make(chan error, 1)
	go func() { firstDone <- first.Wait() }()
	stopped := false
	defer func() {
		if stopped {
			return
		}
		_ = stdin.Close()
		select {
		case <-firstDone:
		case <-time.After(3 * time.Second):
			_ = first.Process.Kill()
			<-firstDone
		}
	}()

	// Complete the MCP initialization so the channel-serving lifetime is live.
	_, err = io.WriteString(stdin, `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"loop-smoke","version":"0"}}}`+"\n")
	if err != nil {
		t.Fatalf("initialize write: %v", err)
	}
	_, err = io.WriteString(stdin, `{"jsonrpc":"2.0","method":"notifications/initialized"}`+"\n")
	if err != nil {
		t.Fatalf("initialized write: %v", err)
	}

	deadline := time.Now().Add(5 * time.Second)
	for {
		raw, readErr := os.ReadFile(marker)
		if readErr == nil && bytes.Count(raw, []byte("tick\n")) >= 1 {
			break
		}
		select {
		case runErr := <-firstDone:
			stopped = true
			t.Fatalf("plain run exited before its loop tick: %v\nstderr:\n%s", runErr, firstStderr.String())
		default:
		}
		if time.Now().After(deadline) {
			t.Fatalf("plain run did not host loop ticker within 5s; stderr:\n%s", firstStderr.String())
		}
		time.Sleep(10 * time.Millisecond)
	}

	secondCtx, secondCancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer secondCancel()
	second := exec.CommandContext(secondCtx, bin, "run")
	second.Env = tokenlessRunEnv(second, stateRoot)
	secondOut, secondErr := second.CombinedOutput()
	if secondErr == nil {
		t.Fatalf("second run on the same box root succeeded; output:\n%s", secondOut)
	}
	if !strings.Contains(string(secondOut), lifecycle.OwnerConflictMarker) {
		t.Fatalf("second run error lacks ownership refusal marker %q:\n%s", lifecycle.OwnerConflictMarker, secondOut)
	}

	// Both runners would execute a never-run loop immediately. Staying at one
	// tick proves the refused process never reached the runner.
	time.Sleep(200 * time.Millisecond)
	raw, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	if got := bytes.Count(raw, []byte("tick\n")); got != 1 {
		t.Fatalf("tick count after concurrent second run = %d, want 1; marker=%q", got, raw)
	}

	_ = stdin.Close()
	select {
	case err := <-firstDone:
		stopped = true
		if err != nil {
			t.Fatalf("plain run shutdown: %v\nstderr:\n%s", err, firstStderr.String())
		}
	case <-time.After(5 * time.Second):
		t.Fatal("plain run did not stop after stdin EOF")
	}
}

func tokenlessRunEnv(cmd *exec.Cmd, stateRoot string) []string {
	return append(scrubEnv(cmd.Environ(),
		"HOTLINE_PROVIDERS",
		"HOTLINE_HARNESS",
		"HOTLINE_SUPERVISOR_DIR",
		lifecycle.OwnerLeaseEnv,
		"HOTLINE_BOT",
		"TELE_GO_BOT",
		"HOTLINE_MISSION_CONTROL",
	),
		"TELEGRAM_BOT_TOKEN=",
		"TELEGRAM_STATE_DIR="+stateRoot,
		"TELE_GO_STATE_DIR="+stateRoot,
		"HOTLINE_STATE_DIR="+stateRoot,
		"HOTLINE_PROVIDERS=telegram",
		"HOTLINE_HARNESS=claude",
		"HOTLINE_MISSION_CONTROL=0",
	)
}

func containsSub(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
