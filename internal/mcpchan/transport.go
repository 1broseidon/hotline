package mcpchan

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// ChannelTransport wraps the SDK's StdioTransport. On Connect it returns a
// chanConn that intercepts the custom claude/channel inbound notifications
// (which the SDK would otherwise reject as unknown methods) and routes them to
// our handler, while passing every standard MCP frame through untouched.
//
// Outbound custom notifications are written through the SAME Connection.Write
// the SDK uses, so they are serialized by the underlying connection's write
// mutex and can never interleave mid-line with SDK frames.
type ChannelTransport struct {
	inner    mcp.Transport
	onPerm   PermissionHandler
	notifier *Notifier
}

// NewChannelTransport builds a transport over stdio with the given inbound
// permission handler.
func NewChannelTransport(onPerm PermissionHandler) *ChannelTransport {
	return &ChannelTransport{
		inner:    &mcp.StdioTransport{},
		onPerm:   onPerm,
		notifier: &Notifier{},
	}
}

// Notifier returns the Notifier bound to this transport. It is only usable
// after Connect has run (its connection is bound there).
func (t *ChannelTransport) Notifier() *Notifier { return t.notifier }

// Connect implements mcp.Transport.
func (t *ChannelTransport) Connect(ctx context.Context) (mcp.Connection, error) {
	inner, err := t.inner.Connect(ctx)
	if err != nil {
		return nil, err
	}
	t.notifier.conn = inner
	return &chanConn{inner: inner, onPerm: t.onPerm}, nil
}

// chanConn is the intercepting Connection.
type chanConn struct {
	inner  mcp.Connection
	onPerm PermissionHandler
}

// Read returns the next message, transparently consuming and dispatching custom
// claude/channel notifications so they never reach the SDK (which would reject
// them as unknown methods before any middleware could see them).
func (c *chanConn) Read(ctx context.Context) (jsonrpc.Message, error) {
	for {
		msg, err := c.inner.Read(ctx)
		if err != nil {
			return nil, err
		}
		req, ok := msg.(*jsonrpc.Request)
		// A notification is a *jsonrpc.Request with an invalid (zero) ID.
		if ok && !req.ID.IsValid() && req.Method == MethodPermissionRequest {
			var p PermissionRequestParams
			if err := json.Unmarshal(req.Params, &p); err == nil && c.onPerm != nil {
				// Never block the read loop on handler work. Recover from any
				// panic so a malformed permission request can't crash the whole
				// process — mirrors dispatchSafely on the poll-loop side.
				go func() {
					defer func() {
						if r := recover(); r != nil {
							fmt.Fprintf(os.Stderr, "hotline: recovered from permission handler panic: %v\n", r)
						}
					}()
					c.onPerm(ctx, p)
				}()
			}
			continue
		}
		return msg, nil
	}
}

// Write delegates to the inner connection (whose Write is concurrency-safe).
func (c *chanConn) Write(ctx context.Context, msg jsonrpc.Message) error {
	return c.inner.Write(ctx, msg)
}

// Close delegates to the inner connection.
func (c *chanConn) Close() error { return c.inner.Close() }

// SessionID delegates to the inner connection.
func (c *chanConn) SessionID() string { return c.inner.SessionID() }
