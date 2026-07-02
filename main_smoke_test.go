package main

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
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
	cmd.Env = append(cmd.Environ(),
		"TELEGRAM_BOT_TOKEN=",
		"TELEGRAM_STATE_DIR="+dir,
		"TELE_GO_STATE_DIR="+dir,
		"HOTLINE_STATE_DIR="+dir,
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
			// must not grow a "source" property in any tool schema.
			if schema, ok := tm["inputSchema"].(map[string]any); ok {
				if props, ok := schema["properties"].(map[string]any); ok {
					if _, has := props["source"]; has {
						t.Errorf("tool %v schema must not expose source with a single provider", tm["name"])
					}
				}
			}
		}
	}
	for _, want := range []string{"reply", "react", "edit_message", "download_attachment"} {
		if !got[want] {
			t.Errorf("tools/list missing %q; got %v", want, got)
		}
	}
	if len(got) != 4 {
		t.Errorf("expected exactly 4 tools, got %d: %v", len(got), got)
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

func containsSub(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
