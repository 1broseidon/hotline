package telegram

import (
	"context"
	"fmt"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Provider adapts the Telegram transport (poller, handler, outbound tools) to
// the provider.Provider interface. It is a thin lifecycle wrapper: all
// behavior lives in the existing Handler/Tools/Poll machinery.
type Provider struct {
	name    string
	cfg     *config.Config
	bot     *gotgbot.Bot // nil when no token is configured (handshake-only)
	tools   *Tools
	handler *Handler
}

// NewProvider builds the Telegram provider. name is its source tag (usually
// "telegram", or "telegram:<instance>" for a named bot). With no token in cfg
// the provider still serves the outbound tools (which report "no bot token
// configured") and Start blocks idle — exactly the old token-less mode.
func NewProvider(name string, cfg *config.Config, log *transcript.Logger) (*Provider, error) {
	p := &Provider{name: name, cfg: cfg}
	if cfg.Token != "" {
		b, err := NewBot(cfg.Token)
		if err != nil {
			return nil, fmt.Errorf("initializing bot: %w", err)
		}
		p.bot = b
		p.handler = NewHandler(b, cfg, nil, log)
	}
	p.tools = NewTools(p.bot, cfg, log)
	return p, nil
}

// Name implements provider.Provider.
func (p *Provider) Name() string { return p.name }

// TranscriptFile is the durable conversation log path for this provider's
// state dir (baked into the channel instructions so the assistant can recover
// the thread across restarts).
func (p *Provider) TranscriptFile() string { return p.cfg.TranscriptFile }

// Capabilities implements provider.Provider. Telegram supports the full
// feature set natively; the permission relay needs a running bot (the access
// gate authenticates repliers).
func (p *Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{
		Buttons:         true,
		Reactions:       true,
		Edits:           true,
		TypingPause:     true,
		PermissionRelay: p.bot != nil,
	}
}

// Start implements provider.Provider: claim the single-poller slot for this
// bot token, bind the sink, and long-poll until ctx is cancelled or the poll
// loop gives up (e.g. persistent 409 Conflict). Without a token it blocks
// idle so the MCP handshake stays up.
func (p *Provider) Start(ctx context.Context, sink provider.InboundSink) error {
	if p.bot == nil {
		<-ctx.Done()
		return nil
	}
	if err := lifecycle.ClaimPollerSlot(p.cfg.PidFile); err != nil {
		return fmt.Errorf("claiming poller slot: %w", err)
	}
	defer lifecycle.ReleasePollerSlot(p.cfg.PidFile)

	p.handler.Notifier = sink
	err := Poll(ctx, p.bot, p.handler.Dispatch)
	// Drain any burst still in the coalescing window before teardown.
	p.handler.FlushAll(context.Background())
	return err
}

// OnPermissionRequest implements provider.Provider: fan the prompt out to
// allowlisted DMs. No-op without a running bot.
func (p *Provider) OnPermissionRequest(ctx context.Context, params mcpchan.PermissionRequestParams) {
	if p.handler != nil {
		p.handler.OnPermissionRequest(ctx, params)
	}
}

// Outbound half: delegate to the existing tool set.

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
