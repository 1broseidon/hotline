package mcpchan

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestExperimentalCaps(t *testing.T) {
	withPerm := experimentalCaps(true)
	if _, ok := withPerm["claude/channel"]; !ok {
		t.Fatal("claude/channel must always be present")
	}
	if _, ok := withPerm["claude/channel/permission"]; !ok {
		t.Fatal("claude/channel/permission must be present when permission=true")
	}
	// Values must be empty objects (map[string]any{}), matching the wire format.
	if m, ok := withPerm["claude/channel"].(map[string]any); !ok || len(m) != 0 {
		t.Fatalf("claude/channel value must be an empty object, got %v", withPerm["claude/channel"])
	}

	noPerm := experimentalCaps(false)
	if _, ok := noPerm["claude/channel"]; !ok {
		t.Fatal("claude/channel must be present when permission=false")
	}
	if _, ok := noPerm["claude/channel/permission"]; ok {
		t.Fatal("claude/channel/permission must NOT be present when permission=false")
	}
}

func TestInstructionsNonEmpty(t *testing.T) {
	s := instructions("/state/transcript.jsonl", "")
	if s == "" {
		t.Fatal("instructions must not be empty")
	}
	// The anti-prompt-injection clause is load-bearing for security.
	if !strings.Contains(s, "prompt injection") {
		t.Error("instructions should warn about prompt injection")
	}
	if !strings.Contains(s, "call reply") {
		t.Error("instructions should mention the reply tool")
	}
}

// TestSchemasAreValidJSON guards against a malformed literal schema sneaking in;
// the SDK passes InputSchema through verbatim, so a broken literal would only
// surface at runtime.
func TestSchemasAreValidJSON(t *testing.T) {
	for name, schema := range map[string]string{
		"reply":               replySchema,
		"react":               reactSchema,
		"edit_message":        editSchema,
		"download_attachment": downloadSchema,
	} {
		var v map[string]any
		if err := json.Unmarshal([]byte(schema), &v); err != nil {
			t.Fatalf("%s schema is not valid JSON: %v", name, err)
		}
		if v["type"] != "object" {
			t.Errorf("%s schema type = %v, want object", name, v["type"])
		}
		if _, ok := v["properties"]; !ok {
			t.Errorf("%s schema missing properties", name)
		}
		req, _ := v["required"].([]any)
		if len(req) == 0 {
			t.Errorf("%s schema should have required fields", name)
		}
	}
}

// fakeToolSet records calls and returns canned results.
type fakeToolSet struct {
	replyCalled bool
	lastReply   ReplyInput
}

func (f *fakeToolSet) Reply(_ context.Context, in ReplyInput) (string, bool) {
	f.replyCalled = true
	f.lastReply = in
	return "no bot token configured", true
}
func (f *fakeToolSet) React(_ context.Context, _ ReactInput) (string, bool) {
	return "reacted", false
}
func (f *fakeToolSet) EditMessage(_ context.Context, _ EditInput) (string, bool) {
	return "edited", false
}
func (f *fakeToolSet) DownloadAttachment(_ context.Context, _ DownloadInput) (string, bool) {
	return "/inbox/x", false
}

// TestServerInProcess drives NewServer over an in-memory transport with a real
// MCP client: it verifies the advertised experimental capabilities, that the
// tool list is exactly the four tools with the verbatim schemas, and that a
// tools/call surfaces a tool-level isError without a protocol error.
func TestServerInProcess(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	fts := &fakeToolSet{}
	server := NewServer(fts, true, "/state/transcript.jsonl", nil, "", "", "", "")

	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}

	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer session.Close()

	// Capabilities from the initialize handshake.
	init := session.InitializeResult()
	if init == nil || init.Capabilities == nil {
		t.Fatal("no capabilities in initialize result")
	}
	exp := init.Capabilities.Experimental
	if _, ok := exp["claude/channel"]; !ok {
		t.Errorf("missing claude/channel; got %v", exp)
	}
	if _, ok := exp["claude/channel/permission"]; !ok {
		t.Errorf("missing claude/channel/permission (permission=true); got %v", exp)
	}
	if init.Capabilities.Tools == nil {
		t.Error("tools capability should be inferred by the SDK")
	}
	if init.Instructions == "" {
		t.Error("instructions should be advertised")
	}

	// tools/list — exactly the channel tools plus publish, with verbatim schemas.
	lr, err := session.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("list tools: %v", err)
	}
	want := map[string]string{
		"reply":               replySchema,
		"react":               reactSchema,
		"edit_message":        editSchema,
		"download_attachment": downloadSchema,
		"publish":             publishSchema,
	}
	if len(lr.Tools) != len(want) {
		t.Fatalf("got %d tools, want %d", len(lr.Tools), len(want))
	}
	for _, tool := range lr.Tools {
		wantSchema, ok := want[tool.Name]
		if !ok {
			t.Errorf("unexpected tool %q", tool.Name)
			continue
		}
		if !jsonEqual(t, []byte(wantSchema), tool.InputSchema) {
			gotBytes, _ := json.Marshal(tool.InputSchema)
			t.Errorf("tool %q schema mismatch:\n got %s\nwant %s", tool.Name, gotBytes, wantSchema)
		}
	}

	// tools/call reply -> tool-level isError, but a successful JSON-RPC call.
	res, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name:      "reply",
		Arguments: map[string]any{"chat_id": "1", "text": "hi"},
	})
	if err != nil {
		t.Fatalf("CallTool returned a protocol error (should be graceful isError): %v", err)
	}
	if !res.IsError {
		t.Error("reply with fake token-less tool set should be isError")
	}
	if !fts.replyCalled {
		t.Error("Reply handler was not invoked")
	}
	if fts.lastReply.ChatID != "1" || fts.lastReply.Text != "hi" {
		t.Errorf("arguments not decoded into ReplyInput: %+v", fts.lastReply)
	}
	if len(res.Content) == 0 {
		t.Fatal("expected content in result")
	}
	tc, ok := res.Content[0].(*mcp.TextContent)
	if !ok || !strings.Contains(tc.Text, "no bot token configured") {
		t.Errorf("expected token error text, got %#v", res.Content[0])
	}
}

// TestServerNoPermissionCap confirms the permission capability is withheld when
// the channel cannot authenticate the replier (no token).
func TestServerNoPermissionCap(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	server := NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", nil, "", "", "", "")
	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatal(err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "t", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()

	exp := session.InitializeResult().Capabilities.Experimental
	if _, ok := exp["claude/channel/permission"]; ok {
		t.Errorf("permission cap must be absent without a token; got %v", exp)
	}
}

// jsonEqual compares a literal JSON blob (want) against a decoded value (got,
// which is whatever the SDK produced for the `any`-typed InputSchema field) by
// round-tripping both through canonical marshaling.
func jsonEqual(t *testing.T, want []byte, got any) bool {
	t.Helper()
	var mw any
	if err := json.Unmarshal(want, &mw); err != nil {
		t.Fatalf("unmarshal want: %v", err)
	}
	gotBytes, err := json.Marshal(got)
	if err != nil {
		t.Fatalf("marshal got: %v", err)
	}
	var mg any
	if err := json.Unmarshal(gotBytes, &mg); err != nil {
		t.Fatalf("unmarshal got: %v", err)
	}
	rw, _ := json.Marshal(mw)
	rg, _ := json.Marshal(mg)
	return string(rw) == string(rg)
}
