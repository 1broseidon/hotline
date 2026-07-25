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
	inner            mcp.Transport
	onPerm           PermissionHandler
	onAgentInfo      func(AgentInfoParams)
	onHarnessCatalog func(HarnessCatalogParams)
	onSDKApplyResult func(SDKApplyResultParams)
	notifier         *Notifier
}

// SetAgentInfoHandler installs the handler for inbound MethodHarnessInfo
// notifications (injected-harness hosts reporting harness/model identity).
// Nil-safe: without a handler the notification is consumed and dropped so it
// never reaches the SDK. Must be called before Connect.
func (t *ChannelTransport) SetAgentInfoHandler(fn func(AgentInfoParams)) {
	t.onAgentInfo = fn
}

// SetSDKApplyResultHandler installs the handler for inbound
// MethodSDKApplyResult notifications (the injected-harness host answering a
// hot model apply — SDK hot-model amendment 2026-07-19). Same posture as
// SetAgentInfoHandler: nil-safe, consume-and-drop without a handler, must be
// called before Connect.
func (t *ChannelTransport) SetSDKApplyResultHandler(fn func(SDKApplyResultParams)) {
	t.onSDKApplyResult = fn
}

// SetHarnessCatalogHandler installs the handler for inbound
// MethodHarnessCatalog notifications (an injected-harness host reporting the
// SELECTABLE model list — model catalog amendment 2026-07-20). Same posture as
// SetAgentInfoHandler: nil-safe, consume-and-drop without a handler, must be
// called before Connect.
func (t *ChannelTransport) SetHarnessCatalogHandler(fn func(HarnessCatalogParams)) {
	t.onHarnessCatalog = fn
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
	return &chanConn{inner: inner, onPerm: t.onPerm, onAgentInfo: t.onAgentInfo, onHarnessCatalog: t.onHarnessCatalog, onSDKApplyResult: t.onSDKApplyResult}, nil
}

// chanConn is the intercepting Connection.
type chanConn struct {
	inner            mcp.Connection
	onPerm           PermissionHandler
	onAgentInfo      func(AgentInfoParams)
	onHarnessCatalog func(HarnessCatalogParams)
	onSDKApplyResult func(SDKApplyResultParams)
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
		if ok && !req.ID.IsValid() && req.Method == MethodHarnessInfo {
			// Same posture as the permission interception: consume before the
			// SDK rejects the unknown method, never block the read loop, and
			// recover so a malformed notification can't crash the process.
			var p AgentInfoParams
			if err := json.Unmarshal(req.Params, &p); err == nil && c.onAgentInfo != nil {
				go func() {
					defer func() {
						if r := recover(); r != nil {
							fmt.Fprintf(os.Stderr, "hotline: recovered from harness_info handler panic: %v\n", r)
						}
					}()
					c.onAgentInfo(p)
				}()
			}
			continue
		}
		if ok && !req.ID.IsValid() && req.Method == MethodHarnessCatalog {
			// Byte-for-byte the MethodHarnessInfo posture. The payload is the
			// biggest frame on this stdio (a whole model list), which is
			// exactly why it arrives once per child-ready instead of riding
			// every identity restamp.
			var p HarnessCatalogParams
			if err := json.Unmarshal(req.Params, &p); err == nil && c.onHarnessCatalog != nil {
				go func() {
					defer func() {
						if r := recover(); r != nil {
							fmt.Fprintf(os.Stderr, "hotline: recovered from harness_catalog handler panic: %v\n", r)
						}
					}()
					c.onHarnessCatalog(p)
				}()
			}
			continue
		}
		if ok && !req.ID.IsValid() && req.Method == MethodSDKApplyResult {
			// Byte-for-byte the MethodHarnessInfo posture: consume before the
			// SDK rejects the unknown method, dispatch in a recovering
			// goroutine, never block the read loop.
			var p SDKApplyResultParams
			if err := json.Unmarshal(req.Params, &p); err == nil && c.onSDKApplyResult != nil {
				go func() {
					defer func() {
						if r := recover(); r != nil {
							fmt.Fprintf(os.Stderr, "hotline: recovered from sdk_apply_result handler panic: %v\n", r)
						}
					}()
					c.onSDKApplyResult(p)
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
