package main

import (
	"context"
	"fmt"
	"os"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/opencode"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/schedule"
)

// runOpenCodeHarness wires hotline to an OpenCode harness. The outbound MCP tool
// surface (server) is served over plain stdio exactly as OpenCode expects of any
// MCP server; inbound push + permission relay ride a SEPARATE HTTP+SSE control
// plane (the harness.Link), not MCP notifications. The messaging providers are
// unchanged — they fan in through a sink backed by the Link.
func runOpenCodeHarness(router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, permission bool, transcriptPath, voice, publishExposure string, cleanup func()) error {
	ocfg, err := config.LoadOpenCode()
	if err != nil {
		return err
	}
	link := opencode.NewLink(ocfg.ServerURL, ocfg.Password, ocfg.Session, ocfg.Agent)
	sink := &opencodeSink{link: link}

	// Reply-delivery fallback: opencode's reply is a manual tool call, so a model
	// that answers in plain text drops the message. The Link nudges once on
	// session-idle and, failing that, forwards the assistant's text itself. Two
	// wires make that work — the reply tool must tell the Link a reply landed, and
	// the Link needs a send path for the backstop. Both are opencode-only; the
	// Claude Code path (main.go) uses the bare router and is untouched.
	observed := &replyObserver{ToolSet: router, onReply: link.MarkReplied}
	link.SetForwarder(func(ctx context.Context, text string, meta map[string]string) error {
		msg, isErr := router.Reply(ctx, mcpchan.ReplyInput{
			Source: meta["source"],
			ChatID: meta["chat_id"],
			Text:   text,
		})
		if isErr {
			return fmt.Errorf("%s", msg)
		}
		return nil
	})
	schedulesPath := sched.Path()
	// Under `hotline up` the supervisor exports HOTLINE_SUPERVISOR_DIR into
	// `opencode serve`'s environment, and opencode passes its process env
	// through to the MCP children it spawns (verified; merged with any
	// explicit environment block in opencode.json) — so a supervised session
	// gains the restart tool here exactly like the claude path, and an
	// unsupervised one never sees it.
	supervisorDir := os.Getenv("HOTLINE_SUPERVISOR_DIR")
	server := mcpchan.NewServer(observed, permission, transcriptPath, router.Sources(), voice, publishExposure, schedulesPath, supervisorDir)

	fmt.Fprintf(os.Stderr, "hotline: harness=opencode server=%s session=%s\n", ocfg.ServerURL, sessionLabel(ocfg.Session))

	// Plain stdio: OpenCode drives the outbound tools as a normal MCP server. No
	// claude/channel interception — permissions arrive over SSE instead.
	transport := &mcp.StdioTransport{}

	pollFn := func(ctx context.Context) error {
		return runOpenCodeLoop(ctx, router, sched, notifyDisp, link, permission, sink)
	}
	return lifecycle.Run(server, transport, cleanup, pollFn)
}

// runOpenCodeLoop runs the five concurrent halves of OpenCode mode — the Link's
// SSE control plane, the permission pump (Link -> providers), the providers'
// inbound loop, the schedule ticker, and the notify dispatcher — returning the
// first that errors or exits. A clean ctx-cancel returns nil. The scheduler and
// dispatcher share the same opencodeSink, so a fire renders as the same
// <channel kind="schedule"|"notify" …> envelope an inbound message does.
func runOpenCodeLoop(ctx context.Context, router *provider.Router, sched *schedule.Scheduler, notifyDisp *notify.Dispatcher, link harness.Link, permission bool, sink provider.InboundSink) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, 5)
	go func() { errCh <- link.Start(ctx) }()
	go func() { errCh <- pumpPermissions(ctx, router, link, permission) }()
	go func() { errCh <- router.Start(ctx, sink) }()
	go func() { errCh <- sched.Run(ctx, sink) }()
	go func() { errCh <- notifyDisp.Run(ctx, sink) }()

	err := <-errCh
	cancel()
	return err
}

// pumpPermissions relays harness permission prompts to the providers' fan-out.
// With no provider able to relay, it still drains the channel so the Link never
// blocks emitting.
func pumpPermissions(ctx context.Context, router *provider.Router, link harness.Link, permission bool) error {
	for {
		select {
		case <-ctx.Done():
			return nil
		case req, ok := <-link.Permissions():
			if !ok {
				return nil
			}
			if !permission {
				continue
			}
			router.OnPermissionRequest(ctx, mcpchan.PermissionRequestParams{
				RequestID:    req.ID,
				ToolName:     req.ToolName,
				Description:  req.Description,
				InputPreview: req.InputPreview,
			})
		}
	}
}

// replyObserver wraps the provider router's ToolSet so the OpenCode Link learns
// when a reply actually lands. Only the reply tool signals a delivered turn;
// react/edit_message/download_attachment pass straight through to the embedded
// ToolSet. This wrapper is opencode-only — the Claude Code path uses the bare
// router.
type replyObserver struct {
	mcpchan.ToolSet
	onReply func()
}

// Reply forwards to the router and, on success, signals the Link that a reply
// was delivered for the active turn. A tool-level error (isErr) is not a
// delivery, so the fallback ladder still fires.
func (r *replyObserver) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	msg, isErr := r.ToolSet.Reply(ctx, in)
	if !isErr {
		r.onReply()
	}
	return msg, isErr
}

// opencodeSink adapts the provider inbound sink to a harness.Link: inbound
// messages become prompt_async pushes; permission verdicts become permission
// answers. It satisfies provider.InboundSink.
type opencodeSink struct {
	link harness.Link
}

func (s *opencodeSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	return s.link.PushInbound(ctx, harness.Inbound{Content: content, Meta: meta})
}

func (s *opencodeSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return s.link.AnswerPermission(ctx, requestID, behavior == "allow")
}

// sessionLabel renders the pinned session for a startup log line.
func sessionLabel(s string) string {
	if s == "" {
		return "(auto)"
	}
	return s
}
