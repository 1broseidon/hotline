package signal

import (
	"context"
	"fmt"
	"os"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Provider adapts the Signal transport (signal-cli HTTP daemon: SSE inbound,
// JSON-RPC outbound) to the provider.Provider interface — the same thin
// lifecycle wrapper shape as the telegram and discord providers.
type Provider struct {
	name    string
	cfg     *config.Config
	client  *Client // nil when SIGNAL_ACCOUNT is not configured (handshake-only)
	tools   *Tools
	handler *Handler
}

// NewProvider builds the Signal provider. name is its source tag ("signal",
// or "signal:<instance>"). With no SIGNAL_ACCOUNT in cfg the provider still
// serves the outbound tools (which report "no signal account configured") and
// Start blocks idle — the same unconfigured handshake mode the other
// providers have.
func NewProvider(name string, cfg *config.Config, log *transcript.Logger) (*Provider, error) {
	p := &Provider{name: name, cfg: cfg}
	opts := newOptionStore()
	if cfg.SignalAccount != "" {
		p.client = NewClient(cfg.SignalDaemonURL, cfg.SignalAccount)
		p.handler = NewHandler(p.client, cfg, log, opts)
	}
	p.tools = NewTools(p.client, cfg, log, opts)
	return p, nil
}

// Name implements provider.Provider.
func (p *Provider) Name() string { return p.name }

// TranscriptFile is the durable conversation log path for this provider's
// state dir.
func (p *Provider) TranscriptFile() string { return p.cfg.TranscriptFile }

// Capabilities implements provider.Provider.
//
//   - Buttons: false — Signal has no inline buttons; the adapter degrades
//     them to numbered text options and maps a bare-number answer back.
//   - Reactions: true — sendReaction.
//   - Edits: true — signal-cli's send --edit-timestamp (JSON-RPC
//     editTimestamp) edits a previously sent message.
//   - TypingPause: true — sendTyping paces bubble bursts.
//   - PermissionRelay: the daemon authenticates senders (Signal's E2E
//     identity), and the "yes <code>" / "no <code>" text path answers
//     prompts without buttons. Needs a configured account.
func (p *Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{
		Buttons:         false,
		Reactions:       true,
		Edits:           true,
		TypingPause:     true,
		PermissionRelay: p.client != nil,
	}
}

// Start implements provider.Provider: claim the single-consumer slot for this
// state dir, bind the sink, and run the daemon's SSE event stream (with
// reconnect/backoff) until ctx is cancelled. Without an account it blocks
// idle so the MCP handshake stays up.
func (p *Provider) Start(ctx context.Context, sink provider.InboundSink) error {
	// Bind the sink whenever a handler exists (tests inject one); production
	// always has one alongside client.
	if p.handler != nil {
		p.handler.BindNotifier(sink)
	}
	if p.client == nil {
		<-ctx.Done()
		return nil
	}
	if err := lifecycle.ClaimPollerSlot(p.cfg.PidFile); err != nil {
		return fmt.Errorf("claiming event-stream slot: %w", err)
	}
	defer lifecycle.ReleasePollerSlot(p.cfg.PidFile)

	fmt.Fprintf(os.Stderr, "hotline: signal streaming events from %s (account %s)\n", p.client.BaseURL, p.client.Account)

	p.client.runEventLoop(ctx, func(ev sseEvent) {
		p.handler.HandleEvent(ctx, ev)
	}, func(err error) {
		fmt.Fprintf(os.Stderr, "hotline: signal event stream dropped: %v — reconnecting\n", err)
	})

	// Drain any burst still in the coalescing window before teardown.
	p.handler.FlushAll(context.Background())
	return nil
}

// OnPermissionRequest implements provider.Provider: fan the prompt out to
// allowlisted numbers as text. No-op without a configured account.
func (p *Provider) OnPermissionRequest(ctx context.Context, params mcpchan.PermissionRequestParams) {
	if p.handler != nil {
		p.handler.OnPermissionRequest(ctx, params)
	}
}

// Outbound half: delegate to the tool set.

// Reply implements mcpchan.ToolSet.
func (p *Provider) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	return p.tools.Reply(ctx, in)
}

// React implements mcpchan.ToolSet.
func (p *Provider) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	return p.tools.React(ctx, in)
}

// EditMessage implements mcpchan.ToolSet.
func (p *Provider) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	return p.tools.EditMessage(ctx, in)
}

// DownloadAttachment implements mcpchan.ToolSet.
func (p *Provider) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	return p.tools.DownloadAttachment(ctx, in)
}
