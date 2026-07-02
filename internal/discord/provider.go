package discord

import (
	"context"
	"fmt"
	"os"

	"github.com/bwmarrin/discordgo"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Provider adapts the Discord transport (gateway session, handler, outbound
// tools) to the provider.Provider interface — the same thin lifecycle wrapper
// shape as telegram.Provider.
type Provider struct {
	name    string
	cfg     *config.Config
	dg      *discordgo.Session // nil when no token is configured (handshake-only)
	tools   *Tools
	handler *Handler
}

// NewProvider builds the Discord provider. name is its source tag ("discord",
// or "discord:<instance>"). With no token in cfg the provider still serves the
// outbound tools (which report "no bot token configured") and Start blocks
// idle — the same token-less handshake mode telegram has.
func NewProvider(name string, cfg *config.Config, log *transcript.Logger) (*Provider, error) {
	p := &Provider{name: name, cfg: cfg}
	var sess Session
	if cfg.Token != "" {
		dg, err := discordgo.New("Bot " + cfg.Token)
		if err != nil {
			return nil, fmt.Errorf("initializing discord session: %w", err)
		}
		dg.Identify.Intents = discordgo.IntentGuildMessages |
			discordgo.IntentDirectMessages |
			discordgo.IntentMessageContent
		p.dg = dg
		sess = &realSession{s: dg}
		p.handler = NewHandler(sess, cfg, log)
	}
	p.tools = NewTools(sess, cfg, log)
	return p, nil
}

// Name implements provider.Provider.
func (p *Provider) Name() string { return p.name }

// TranscriptFile is the durable conversation log path for this provider's
// state dir.
func (p *Provider) TranscriptFile() string { return p.cfg.TranscriptFile }

// Capabilities implements provider.Provider. Discord supports the full
// feature set natively — buttons (message components), reactions, edits, and a
// typing indicator. The permission relay uses the same allow/deny button flow
// as telegram (component custom_ids instead of callback data) and needs a
// running session to authenticate repliers.
func (p *Provider) Capabilities() provider.Capabilities {
	return provider.Capabilities{
		Buttons:         true,
		Reactions:       true,
		Edits:           true,
		TypingPause:     true,
		PermissionRelay: p.dg != nil,
	}
}

// Start implements provider.Provider: claim the single-session slot for this
// state dir, open the gateway websocket, bind the sink, and block until ctx is
// cancelled. Without a token it blocks idle so the MCP handshake stays up.
func (p *Provider) Start(ctx context.Context, sink provider.InboundSink) error {
	if p.dg == nil {
		<-ctx.Done()
		return nil
	}
	if err := lifecycle.ClaimPollerSlot(p.cfg.PidFile); err != nil {
		return fmt.Errorf("claiming gateway slot: %w", err)
	}
	defer lifecycle.ReleasePollerSlot(p.cfg.PidFile)

	p.handler.Notifier = sink

	p.dg.AddHandler(func(_ *discordgo.Session, m *discordgo.MessageCreate) {
		defer recoverPanic("message")
		p.handler.HandleMessage(ctx, m.Message)
	})
	p.dg.AddHandler(func(_ *discordgo.Session, i *discordgo.InteractionCreate) {
		defer recoverPanic("interaction")
		p.handler.HandleInteraction(ctx, i.Interaction)
	})

	if err := p.dg.Open(); err != nil {
		return fmt.Errorf("opening discord gateway: %w", err)
	}
	if u := p.dg.State.User; u != nil {
		p.handler.BotUserID = u.ID
		fmt.Fprintf(os.Stderr, "hotline: discord connected as %s\n", u.Username)
	}

	<-ctx.Done()
	_ = p.dg.Close()
	// Drain any burst still in the coalescing window before teardown.
	p.handler.FlushAll(context.Background())
	return nil
}

// recoverPanic keeps a panicking event handler from killing the gateway
// goroutine (discordgo dispatches handlers on its own goroutines).
func recoverPanic(kind string) {
	if r := recover(); r != nil {
		fmt.Fprintf(os.Stderr, "hotline: recovered panic in discord %s handler: %v\n", kind, r)
	}
}

// OnPermissionRequest implements provider.Provider: fan the prompt out to
// allowlisted DMs. No-op without a running session.
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
