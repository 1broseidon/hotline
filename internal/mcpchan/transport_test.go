package mcpchan

import (
	"context"
	"encoding/json"
	"io"
	"sync"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
)

// fakeConn is a controllable mcp.Connection for testing chanConn.
type fakeConn struct {
	mu       sync.Mutex
	incoming []jsonrpc.Message
	writes   []jsonrpc.Message
}

func (f *fakeConn) Read(ctx context.Context) (jsonrpc.Message, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.incoming) == 0 {
		return nil, io.EOF
	}
	m := f.incoming[0]
	f.incoming = f.incoming[1:]
	return m, nil
}

func (f *fakeConn) Write(ctx context.Context, m jsonrpc.Message) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.writes = append(f.writes, m)
	return nil
}

func (f *fakeConn) Close() error      { return nil }
func (f *fakeConn) SessionID() string { return "fake" }

func notification(method string, params any) *jsonrpc.Request {
	raw, _ := json.Marshal(params)
	return &jsonrpc.Request{Method: method, Params: raw}
}

func call(id int64, method string) *jsonrpc.Request {
	rid, _ := jsonrpc.MakeID(float64(id))
	return &jsonrpc.Request{ID: rid, Method: method}
}

func TestChanConnRoutesPermissionRequest(t *testing.T) {
	permCh := make(chan PermissionRequestParams, 1)
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodPermissionRequest, PermissionRequestParams{RequestID: "abcde", ToolName: "Bash"}),
		call(1, "tools/list"),
	}}
	c := &chanConn{inner: inner, onPerm: func(ctx context.Context, p PermissionRequestParams) {
		permCh <- p
	}}

	// First Read should consume the permission_request and surface the next
	// real message (tools/list).
	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	req, ok := msg.(*jsonrpc.Request)
	if !ok || req.Method != "tools/list" {
		t.Fatalf("expected tools/list to surface, got %#v", msg)
	}

	select {
	case p := <-permCh:
		if p.RequestID != "abcde" || p.ToolName != "Bash" {
			t.Fatalf("handler got wrong params: %+v", p)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("permission handler was not invoked")
	}
}

func TestChanConnPassesNormalRequest(t *testing.T) {
	inner := &fakeConn{incoming: []jsonrpc.Message{call(2, "initialize")}}
	c := &chanConn{inner: inner, onPerm: func(context.Context, PermissionRequestParams) {
		t.Fatal("handler should not fire for normal request")
	}}
	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if req := msg.(*jsonrpc.Request); req.Method != "initialize" {
		t.Fatalf("got %q", req.Method)
	}
}

func TestNotifierSendEmitsZeroIDRequest(t *testing.T) {
	inner := &fakeConn{}
	n := &Notifier{conn: inner}
	if err := n.SendChannel(context.Background(), "hi", map[string]string{"chat_id": "1"}); err != nil {
		t.Fatal(err)
	}
	if len(inner.writes) != 1 {
		t.Fatalf("expected 1 write, got %d", len(inner.writes))
	}
	req, ok := inner.writes[0].(*jsonrpc.Request)
	if !ok {
		t.Fatalf("write is not a *jsonrpc.Request: %T", inner.writes[0])
	}
	if req.ID.IsValid() {
		t.Fatal("notification must have a zero/invalid ID")
	}
	if req.Method != MethodChannel {
		t.Fatalf("method = %q", req.Method)
	}
	var p InboundParams
	if err := json.Unmarshal(req.Params, &p); err != nil {
		t.Fatal(err)
	}
	if p.Content != "hi" || p.Meta["chat_id"] != "1" {
		t.Fatalf("params mismatch: %+v", p)
	}
}

func TestNotifierVerdictEmitsZeroIDRequest(t *testing.T) {
	inner := &fakeConn{}
	n := &Notifier{conn: inner}
	if err := n.SendVerdict(context.Background(), "abcde", "deny"); err != nil {
		t.Fatal(err)
	}
	req := inner.writes[0].(*jsonrpc.Request)
	if req.ID.IsValid() {
		t.Fatal("verdict notification must have invalid ID")
	}
	if req.Method != MethodPermissionVerdict {
		t.Fatalf("method = %q", req.Method)
	}
}

func TestNotifierNotConnected(t *testing.T) {
	n := &Notifier{}
	if err := n.SendChannel(context.Background(), "x", nil); err == nil {
		t.Fatal("expected error when conn is nil")
	}
}
