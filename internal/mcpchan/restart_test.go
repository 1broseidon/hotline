package mcpchan

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// listToolNames connects an in-process client and returns the advertised tool
// names.
func listToolNames(t *testing.T, server *mcp.Server) (map[string]bool, *mcp.ClientSession) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)

	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { session.Close() })

	lr, err := session.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("list tools: %v", err)
	}
	names := map[string]bool{}
	for _, tool := range lr.Tools {
		names[tool.Name] = true
	}
	return names, session
}

// TestRestartToolOnlySupervised: the restart tool exists exactly when a
// supervisor dir is wired in (the session runs under `hotline up`).
func TestRestartToolOnlySupervised(t *testing.T) {
	names, _ := listToolNames(t, NewServer(&fakeToolSet{}, false, "/t", nil, "", "", "", ""))
	if names["restart"] {
		t.Error("restart tool must not be registered without a supervisor")
	}
	names, _ = listToolNames(t, NewServer(&fakeToolSet{}, false, "/t", nil, "", "", "", t.TempDir()))
	if !names["restart"] {
		t.Error("restart tool missing under a supervisor")
	}
}

// TestRestartToolWritesControlFile: calling the tool writes the supervisor's
// restart.request with a flattened, logged-only reason.
func TestRestartToolWritesControlFile(t *testing.T) {
	dir := t.TempDir()
	_, session := listToolNames(t, NewServer(&fakeToolSet{}, false, "/t", nil, "", "", "", dir))

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	res, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name:      "restart",
		Arguments: map[string]any{"reason": "user asked\nfor a restart"},
	})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if res.IsError {
		t.Fatalf("restart tool errored: %v", res.Content)
	}
	raw, err := os.ReadFile(filepath.Join(dir, "restart.request"))
	if err != nil {
		t.Fatalf("control file not written: %v", err)
	}
	line := strings.TrimSpace(string(raw))
	if strings.Count(line, "\n") != 0 || !strings.Contains(line, "user asked for a restart") {
		t.Errorf("control file = %q, want one flattened line with the reason", line)
	}
	tc, ok := res.Content[0].(*mcp.TextContent)
	if !ok || !strings.Contains(tc.Text, "Restart requested") {
		t.Errorf("tool result = %#v", res.Content[0])
	}
}
