// Package provider defines the transport-neutral chat-provider interface and
// the Router that multiplexes several providers behind the single MCP tool
// surface.
//
// A Provider is one chat transport (Telegram today; Discord et al. later). Its
// outbound half is exactly mcpchan.ToolSet — the agent-facing tool contract
// (reply/react/edit_message/download_attachment with bubbles, buttons, string
// ids) never changes per transport. Its inbound half is Start: normalize the
// transport's events and push them into an InboundSink. Whatever a transport
// cannot do natively (inline buttons, reactions, edits, typing pauses) is the
// adapter's job to degrade gracefully inside its outbound methods — e.g.
// buttons become numbered text options — never the agent's job to work around.
package provider

import (
	"context"
	"fmt"
	"strings"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// Capabilities describes what a transport supports natively. It is internal
// bookkeeping for adapters (and tests): a false flag means the adapter must
// degrade that feature itself, not that the tool contract changes.
type Capabilities struct {
	// Buttons: inline tappable options. Without them, adapters render buttons
	// as numbered text options (see ButtonsToNumberedText).
	Buttons bool
	// Reactions: emoji reactions on messages.
	Reactions bool
	// Edits: editing previously sent messages.
	Edits bool
	// TypingPause: a typing indicator / paced delivery between bubbles.
	TypingPause bool
	// PermissionRelay: the provider authenticates repliers and can relay
	// claude/channel/permission prompts.
	PermissionRelay bool
}

// InboundSink is where a provider delivers normalized inbound traffic. The
// channel path carries (content, meta) exactly as the claude/channel protocol
// expects; the verdict path answers a pending permission request.
// *mcpchan.Notifier satisfies this interface.
type InboundSink interface {
	SendChannel(ctx context.Context, content string, meta map[string]string) error
	SendVerdict(ctx context.Context, requestID, behavior string) error
}

// Provider is one chat transport. The embedded mcpchan.ToolSet is the outbound
// half; Start is the inbound half.
type Provider interface {
	mcpchan.ToolSet

	// Name is the provider's source tag ("telegram", "telegram:work",
	// "discord"). It keys outbound routing and is stamped into inbound meta.
	Name() string

	// Capabilities reports what the transport supports natively.
	Capabilities() Capabilities

	// Start runs the provider's inbound loop (poller, gateway connection, …),
	// delivering normalized events to sink until ctx is cancelled or the
	// transport gives up (returned as a non-nil error). A provider with nothing
	// to poll (e.g. no token configured) blocks until ctx is done and returns
	// nil, keeping the MCP handshake alive.
	Start(ctx context.Context, sink InboundSink) error

	// OnPermissionRequest relays a permission prompt to the transport's
	// authenticated operators. Providers without PermissionRelay no-op.
	OnPermissionRequest(ctx context.Context, p mcpchan.PermissionRequestParams)
}

// ButtonsToNumberedText renders inline-button options as a numbered text block
// — the shared degradation for transports whose Capabilities lack Buttons. The
// user answers by typing the number or the option text.
func ButtonsToNumberedText(buttons []string) string {
	var b strings.Builder
	for i, opt := range buttons {
		opt = strings.TrimSpace(opt)
		if opt == "" {
			continue
		}
		if b.Len() > 0 {
			b.WriteByte('\n')
		}
		fmt.Fprintf(&b, "%d. %s", i+1, opt)
	}
	return b.String()
}
