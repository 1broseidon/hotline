// Package mcpchan wires the channel to Claude Code over MCP. It builds the MCP
// server (initialize handshake, capabilities, tool registration) using the
// official Go SDK, and adds the one piece the SDK can't do natively: sending
// and receiving the custom claude/channel JSON-RPC notifications. Those are
// routed through a custom Transport/Connection so raw notification frames are
// serialized with the SDK's own writes and never interleave mid-line.
package mcpchan

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// JSON-RPC method names for the claude/channel protocol.
const (
	// MethodChannel delivers an inbound message to Claude.
	MethodChannel = "notifications/claude/channel"
	// MethodPermissionRequest is an inbound notification from Claude asking the
	// channel to relay a permission prompt.
	MethodPermissionRequest = "notifications/claude/channel/permission_request"
	// MethodPermissionVerdict is the outbound allow/deny answer to a permission
	// request.
	MethodPermissionVerdict = "notifications/claude/channel/permission"
)

// InboundParams is the payload of a MethodChannel notification.
type InboundParams struct {
	Content string            `json:"content"`
	Meta    map[string]string `json:"meta"`
}

// Notifier sends custom claude/channel notifications over the same logical
// JSON-RPC connection the SDK uses, so raw frames can't interleave mid-line
// with SDK frames.
type Notifier struct {
	conn mcp.Connection
}

// SendChannel delivers an inbound message to Claude. Meta keys with empty
// values are dropped by the caller; this method emits whatever map it's given.
func (n *Notifier) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	return n.send(ctx, MethodChannel, InboundParams{Content: content, Meta: meta})
}

// send marshals params and writes a zero-ID JSON-RPC request (a notification)
// to the connection.
func (n *Notifier) send(ctx context.Context, method string, params any) error {
	if n == nil || n.conn == nil {
		return fmt.Errorf("notifier not connected")
	}
	raw, err := json.Marshal(params)
	if err != nil {
		return fmt.Errorf("marshaling %s params: %w", method, err)
	}
	// A notification is a *jsonrpc.Request with a zero (invalid) ID.
	req := &jsonrpc.Request{Method: method, Params: raw}
	return n.conn.Write(ctx, req)
}
