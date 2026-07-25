package app

import (
	"context"
	"fmt"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

type Provider struct {
	name  string
	cfg   *config.Config
	srv   *Server
	tools *Tools
}

// NewProvider builds the app-channel provider. agent seeds the box identity
// metadata (harness kind + configured model/effort) resolved by the caller —
// config reads stay out of internal/app; the harness_info notification
// refines it live via AgentInfoSink. boxRoot is this box's state root, so a
// model/effort change lands in THIS box's .env rather than the machine-wide
// one (sol review #10); empty = the default box, which is the base root.
// version stamps the box binary identity into the caps manifest's bin{} field
// (caps-design §1): it lives in package main and is resolved by the caller, keeping
// the version/VCS read out of internal/app exactly like the AgentInfo seed.
func NewProvider(name string, cfg *config.Config, log *transcript.Logger, agent AgentInfo, boxRoot string, version AppVersion) (*Provider, error) {
	srv := NewServer(cfg, log)
	if srv.initErr != nil {
		return nil, fmt.Errorf("initialize app relay: %w", srv.initErr)
	}
	srv.agentInfo = agent // pre-Run: no lock or emit needed
	srv.boxRoot = boxRoot
	srv.appVersion = version // pre-Run: no lock needed
	return &Provider{name: name, cfg: cfg, srv: srv, tools: NewTools(srv, cfg, log)}, nil
}

// AgentInfoSink adapts the run child's harness_info notification (transport
// interception, mcpchan.AgentInfoParams) into the server's live identity
// merge — the same discovery pattern as JobDriver.
func (p *Provider) AgentInfoSink() func(mcpchan.AgentInfoParams) {
	return func(params mcpchan.AgentInfoParams) {
		// Presence-aware: nil leaves a field alone, a pointer to "" is an
		// explicit clear the box must actually apply (hot-clear amendment).
		p.srv.MergeAgentInfo(params.Harness, params.Model, params.Effort)
	}
}

// HarnessCatalogSink adapts the run child's harness_catalog notification
// (transport interception, mcpchan.HarnessCatalogParams) into the server's
// held catalog — the same discovery pattern as AgentInfoSink, one level up:
// where that carries the one live model, this carries the selectable set.
// Everything the harness reports is sanitized and bounded on the way in.
func (p *Provider) HarnessCatalogSink() func(mcpchan.HarnessCatalogParams) {
	return func(params mcpchan.HarnessCatalogParams) {
		models := make([]catalogModel, 0, len(params.Models))
		for _, m := range params.Models {
			models = append(models, catalogModel{ID: m.ID, Label: m.Label, Available: m.Available})
		}
		p.srv.SetAgentCatalog(sanitizeAgentCatalog(params.Harness, params.Source, models, params.Truncated))
	}
}

// SetSDKApplyForwarder binds the harness-bound hot-apply forwarder (SDK
// hot-model amendment 2026-07-19): the server calls it to forward a
// model-only set_sdk_config as a sdk_apply notification to the injected
// harness. Bound pre-Run by the claude-sdk wiring only (same posture as the
// agentInfo seed — no lock needed); nil keeps the restart path.
func (p *Provider) SetSDKApplyForwarder(fn func(ctx context.Context, rid string, model, effort *string) error) {
	p.srv.sdkApplyForward = fn
}

// SDKApplyResultSink adapts the run child's sdk_apply_result notification
// (transport interception) into the server's pending-apply resolution —
// the mirror of AgentInfoSink.
func (p *Provider) SDKApplyResultSink() func(mcpchan.SDKApplyResultParams) {
	return func(params mcpchan.SDKApplyResultParams) {
		p.srv.handleSDKApplyResult(params)
	}
}

func (p *Provider) Name() string           { return p.name }
func (p *Provider) TranscriptFile() string { return p.cfg.TranscriptFile }
func (p *Provider) Capabilities() provider.Capabilities {
	// JobCards: the app channel is the only transport that renders job elements —
	// it is where the jobspool dispatcher's cards actually live.
	return provider.Capabilities{Buttons: true, Reactions: true, Edits: true, TypingPause: true, JobCards: true}
}
func (p *Provider) Start(ctx context.Context, sink provider.InboundSink) error {
	return p.srv.Run(ctx, sink)
}
func (p *Provider) OnPermissionRequest(context.Context, mcpchan.PermissionRequestParams) {}
func (p *Provider) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	return p.tools.Reply(ctx, in)
}
func (p *Provider) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	return p.tools.React(ctx, in)
}
func (p *Provider) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	return p.tools.EditMessage(ctx, in)
}
func (p *Provider) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	return p.tools.DownloadAttachment(ctx, in)
}
func (p *Provider) PublishArtifact(ctx context.Context, in mcpchan.PublishInput) (string, bool, bool) {
	msg, isErr := p.tools.PublishArtifact(ctx, in)
	return msg, isErr, true
}

// Job implements the optional mcpchan.JobRunner interface: the app channel
// owns the job registry and the live job card, so it always handles the tool.
func (p *Provider) Job(ctx context.Context, in mcpchan.JobInput) (string, bool, bool) {
	msg, isErr := p.tools.Job(ctx, in)
	return msg, isErr, true
}
