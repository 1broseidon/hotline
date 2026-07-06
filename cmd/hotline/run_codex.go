package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"github.com/1broseidon/hotline/internal/codex"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
)

// runCodexHarness wires hotline to a Codex app-server subprocess owned by this
// process. Unlike Claude/OpenCode modes, Phase 1 exposes no hotline MCP tools to
// Codex; completed agent messages are forwarded straight to the messaging
// provider.
func runCodexHarness(botName string, router *provider.Router, permission bool, transcriptPath, voice string, cleanup func()) error {
	ccfg, err := config.LoadCodex(botName)
	if err != nil {
		return err
	}
	link := codex.NewLink(codex.Options{
		CWD:                   ccfg.CWD,
		ThreadID:              ccfg.ThreadID,
		ThreadFile:            ccfg.ThreadFile,
		ApprovalPolicy:        ccfg.ApprovalPolicy,
		Sandbox:               ccfg.Sandbox,
		DeveloperInstructions: mcpchan.CodexDeveloperInstructions(transcriptPath, voice),
		AutoDenyPermissions:   !permission,
	})
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

	fmt.Fprintf(os.Stderr, "hotline: harness=codex cwd=%s thread=%s\n", ccfg.CWD, sessionLabel(firstNonEmpty(ccfg.ThreadID, readFileTrim(ccfg.ThreadFile))))

	sink := &codexSink{link: link}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	defer stop()
	defer cleanup()

	return runCodexLoop(ctx, router, link, permission, sink)
}

func runCodexLoop(ctx context.Context, router *provider.Router, link harness.Link, permission bool, sink provider.InboundSink) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, 3)
	go func() { errCh <- link.Start(ctx) }()
	go func() { errCh <- pumpPermissions(ctx, router, link, permission) }()
	go func() { errCh <- router.Start(ctx, sink) }()

	err := <-errCh
	if ctx.Err() != nil {
		cancel()
		return nil
	}
	cancel()
	return err
}

type codexSink struct {
	link harness.Link
}

func (s *codexSink) SendChannel(ctx context.Context, content string, meta map[string]string) error {
	return s.link.PushInbound(ctx, harness.Inbound{Content: content, Meta: meta})
}

func (s *codexSink) SendVerdict(ctx context.Context, requestID, behavior string) error {
	return s.link.AnswerPermission(ctx, requestID, behavior == "allow")
}

func readFileTrim(path string) string {
	raw, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(raw))
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}
