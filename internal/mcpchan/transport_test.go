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

// TestChanConnRoutesHarnessInfo mirrors the permission-request interception:
// the harness_info notification is consumed (never surfaced to the SDK, which
// would reject the unknown method) and dispatched to the handler.
func TestChanConnRoutesHarnessInfo(t *testing.T) {
	infoCh := make(chan AgentInfoParams, 1)
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodHarnessInfo, AgentInfoParams{Harness: "claude-sdk", Model: strPtr("claude-opus-4-8"), Effort: strPtr("xhigh")}),
		call(1, "tools/list"),
	}}
	c := &chanConn{inner: inner, onAgentInfo: func(p AgentInfoParams) {
		infoCh <- p
	}}

	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	req, ok := msg.(*jsonrpc.Request)
	if !ok || req.Method != "tools/list" {
		t.Fatalf("expected tools/list to surface, got %#v", msg)
	}

	select {
	case p := <-infoCh:
		if p.Harness != "claude-sdk" || p.Model == nil || *p.Model != "claude-opus-4-8" || p.Effort == nil || *p.Effort != "xhigh" {
			t.Fatalf("handler got wrong params: %+v", p)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("harness_info handler was not invoked")
	}
}

// TestChanConnDropsHarnessInfoWithoutHandler: nil handler (telegram-only box,
// or the claude/opencode paths that never wire one) still consumes the
// notification so the SDK never sees the unknown method.
func TestChanConnDropsHarnessInfoWithoutHandler(t *testing.T) {
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodHarnessInfo, AgentInfoParams{Harness: "claude-sdk"}),
		call(3, "initialize"),
	}}
	c := &chanConn{inner: inner}
	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if req := msg.(*jsonrpc.Request); req.Method != "initialize" {
		t.Fatalf("expected initialize to surface past the dropped notification, got %q", req.Method)
	}
}

// TestChanConnRoutesSDKApplyResult mirrors the harness_info interception: the
// sdk_apply_result notification is consumed (never surfaced to the SDK, which
// would reject the unknown method) and dispatched to the handler.
func TestChanConnRoutesSDKApplyResult(t *testing.T) {
	resCh := make(chan SDKApplyResultParams, 1)
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodSDKApplyResult, SDKApplyResultParams{RID: "01J2ZKB0HOT00001", OK: true, Model: "claude-sonnet-4-6"}),
		call(1, "tools/list"),
	}}
	c := &chanConn{inner: inner, onSDKApplyResult: func(p SDKApplyResultParams) {
		resCh <- p
	}}

	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	req, ok := msg.(*jsonrpc.Request)
	if !ok || req.Method != "tools/list" {
		t.Fatalf("expected tools/list to surface, got %#v", msg)
	}

	select {
	case p := <-resCh:
		if p.RID != "01J2ZKB0HOT00001" || !p.OK || p.Model != "claude-sonnet-4-6" || p.Code != "" {
			t.Fatalf("handler got wrong params: %+v", p)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("sdk_apply_result handler was not invoked")
	}
}

// TestChanConnRoutesSDKApplyResultFailure: the failure shape (code + detail)
// survives the interception intact.
func TestChanConnRoutesSDKApplyResultFailure(t *testing.T) {
	resCh := make(chan SDKApplyResultParams, 1)
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodSDKApplyResult, SDKApplyResultParams{RID: "r1", OK: false, Code: "unknown_model", Detail: "model not in the CLI's supported list"}),
		call(1, "tools/list"),
	}}
	c := &chanConn{inner: inner, onSDKApplyResult: func(p SDKApplyResultParams) {
		resCh <- p
	}}
	if _, err := c.Read(context.Background()); err != nil {
		t.Fatal(err)
	}
	select {
	case p := <-resCh:
		if p.OK || p.Code != "unknown_model" || p.Detail != "model not in the CLI's supported list" {
			t.Fatalf("handler got wrong params: %+v", p)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("sdk_apply_result handler was not invoked")
	}
}

// TestChanConnDropsSDKApplyResultWithoutHandler: nil handler (every non-sdk
// wiring) still consumes the notification so the SDK never sees the unknown
// method.
func TestChanConnDropsSDKApplyResultWithoutHandler(t *testing.T) {
	inner := &fakeConn{incoming: []jsonrpc.Message{
		notification(MethodSDKApplyResult, SDKApplyResultParams{RID: "r1", OK: true}),
		call(3, "initialize"),
	}}
	c := &chanConn{inner: inner}
	msg, err := c.Read(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if req := msg.(*jsonrpc.Request); req.Method != "initialize" {
		t.Fatalf("expected initialize to surface past the dropped notification, got %q", req.Method)
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

// TestNotifierSendSDKApply: the hot-apply forward is a zero-ID notification
// with the exact wire method and params; a clear (pointer-to-"") serializes the
// field explicitly as "", while an omitted (nil) field is dropped from the wire.
func TestNotifierSendSDKApply(t *testing.T) {
	inner := &fakeConn{}
	n := &Notifier{conn: inner}
	strp := func(s string) *string { return &s }
	// model set, effort omitted (nil).
	if err := n.SendSDKApply(context.Background(), "01J2ZKB0HOT00001", strp("claude-sonnet-4-6"), nil); err != nil {
		t.Fatal(err)
	}
	// model clear (pointer-to-""), effort omitted.
	if err := n.SendSDKApply(context.Background(), "01J2ZKB2CLR00001", strp(""), nil); err != nil {
		t.Fatal(err)
	}
	if len(inner.writes) != 2 {
		t.Fatalf("expected 2 writes, got %d", len(inner.writes))
	}
	req, ok := inner.writes[0].(*jsonrpc.Request)
	if !ok {
		t.Fatalf("write is not a *jsonrpc.Request: %T", inner.writes[0])
	}
	if req.ID.IsValid() {
		t.Fatal("notification must have a zero/invalid ID")
	}
	if req.Method != MethodSDKApply {
		t.Fatalf("method = %q", req.Method)
	}
	var raw0 map[string]any
	if err := json.Unmarshal(req.Params, &raw0); err != nil {
		t.Fatal(err)
	}
	if raw0["rid"] != "01J2ZKB0HOT00001" || raw0["model"] != "claude-sonnet-4-6" {
		t.Fatalf("params mismatch: %v", raw0)
	}
	if _, present := raw0["effort"]; present {
		t.Fatalf("omitted effort must not serialize: %v", raw0)
	}
	clear := inner.writes[1].(*jsonrpc.Request)
	var raw map[string]any
	if err := json.Unmarshal(clear.Params, &raw); err != nil {
		t.Fatal(err)
	}
	if model, present := raw["model"]; !present || model != "" {
		t.Fatalf("clear must serialize model explicitly as \"\": %v", raw)
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

func strPtr(s string) *string { return &s }

// TestChanConnHarnessInfoPresence: the three states the wire must carry
// (hot-clear amendment). An omitted field is nil ("leave it alone"); an
// explicit "" is a pointer to "" (a CLEAR the box must apply). Collapsing the
// two is what let a cleared model keep being advertised.
func TestChanConnHarnessInfoPresence(t *testing.T) {
	for _, tc := range []struct {
		name      string
		raw       string
		wantModel *string
	}{
		{"omitted is nil", `{"harness":"claude-sdk"}`, nil},
		{"explicit clear is a pointer to empty", `{"harness":"claude-sdk","model":""}`, strPtr("")},
		{"a value is the value", `{"harness":"claude-sdk","model":"claude-opus-4-8"}`, strPtr("claude-opus-4-8")},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var p AgentInfoParams
			if err := json.Unmarshal([]byte(tc.raw), &p); err != nil {
				t.Fatal(err)
			}
			switch {
			case tc.wantModel == nil && p.Model != nil:
				t.Fatalf("omitted model decoded as %q, want nil (leave-alone)", *p.Model)
			case tc.wantModel != nil && p.Model == nil:
				t.Fatalf("model %q decoded as nil", *tc.wantModel)
			case tc.wantModel != nil && *p.Model != *tc.wantModel:
				t.Fatalf("model = %q, want %q", *p.Model, *tc.wantModel)
			}
		})
	}
}
