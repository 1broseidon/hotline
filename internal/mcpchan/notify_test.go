package mcpchan

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
)

// TestMethodConstants pins the wire method names. These strings are the
// contract with Claude Code; changing one silently breaks the channel.
func TestMethodConstants(t *testing.T) {
	cases := map[string]string{
		MethodChannel:           "notifications/claude/channel",
		MethodPermissionRequest: "notifications/claude/channel/permission_request",
		MethodPermissionVerdict: "notifications/claude/channel/permission",
	}
	for got, want := range cases {
		if got != want {
			t.Errorf("method constant = %q, want %q", got, want)
		}
	}
}

// TestChannelNotificationWireShape encodes a SendChannel frame the way it goes
// out on stdio and asserts the full JSON-RPC 2.0 notification shape documented
// in the design: jsonrpc "2.0", the channel method, NO id, and a params object
// of {content, meta}.
func TestChannelNotificationWireShape(t *testing.T) {
	inner := &fakeConn{}
	n := &Notifier{conn: inner}
	meta := map[string]string{
		"chat_id":    "412587349",
		"message_id": "57",
		"user":       "george",
		"user_id":    "412587349",
		"ts":         "2026-06-28T12:00:00.000Z",
	}
	if err := n.SendChannel(context.Background(), "hello", meta); err != nil {
		t.Fatal(err)
	}
	if len(inner.writes) != 1 {
		t.Fatalf("expected 1 write, got %d", len(inner.writes))
	}

	line, err := jsonrpc.EncodeMessage(inner.writes[0])
	if err != nil {
		t.Fatalf("encode: %v", err)
	}

	var envelope struct {
		JSONRPC string           `json:"jsonrpc"`
		ID      *json.RawMessage `json:"id"`
		Method  string           `json:"method"`
		Params  InboundParams    `json:"params"`
	}
	if err := json.Unmarshal(line, &envelope); err != nil {
		t.Fatalf("unmarshal wire line %s: %v", line, err)
	}
	if envelope.JSONRPC != "2.0" {
		t.Errorf("jsonrpc = %q, want 2.0", envelope.JSONRPC)
	}
	if envelope.ID != nil {
		t.Errorf("a notification must have no id; got %s", *envelope.ID)
	}
	if envelope.Method != MethodChannel {
		t.Errorf("method = %q, want %q", envelope.Method, MethodChannel)
	}
	if envelope.Params.Content != "hello" {
		t.Errorf("content = %q", envelope.Params.Content)
	}
	if envelope.Params.Meta["user"] != "george" || envelope.Params.Meta["chat_id"] != "412587349" {
		t.Errorf("meta mismatch: %v", envelope.Params.Meta)
	}
	if len(envelope.Params.Meta) != len(meta) {
		t.Errorf("meta key count = %d, want %d (no empty keys injected)", len(envelope.Params.Meta), len(meta))
	}
}

// TestVerdictNotificationWireShape does the same for the outbound permission
// verdict.
func TestVerdictNotificationWireShape(t *testing.T) {
	inner := &fakeConn{}
	n := &Notifier{conn: inner}
	if err := n.SendVerdict(context.Background(), "abcde", "allow"); err != nil {
		t.Fatal(err)
	}
	line, err := jsonrpc.EncodeMessage(inner.writes[0])
	if err != nil {
		t.Fatalf("encode: %v", err)
	}
	var envelope struct {
		JSONRPC string                  `json:"jsonrpc"`
		ID      *json.RawMessage        `json:"id"`
		Method  string                  `json:"method"`
		Params  PermissionVerdictParams `json:"params"`
	}
	if err := json.Unmarshal(line, &envelope); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if envelope.JSONRPC != "2.0" || envelope.ID != nil {
		t.Errorf("bad envelope: jsonrpc=%q id=%v", envelope.JSONRPC, envelope.ID)
	}
	if envelope.Method != MethodPermissionVerdict {
		t.Errorf("method = %q, want %q", envelope.Method, MethodPermissionVerdict)
	}
	if envelope.Params.RequestID != "abcde" || envelope.Params.Behavior != "allow" {
		t.Errorf("verdict params = %+v", envelope.Params)
	}
}
