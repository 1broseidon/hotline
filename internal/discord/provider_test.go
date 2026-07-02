package discord

import (
	"context"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/provider/stubprovider"
)

// testProvider wraps a fake-session handler + tools into a *Provider (fields
// are in-package accessible), so the router can drive it like production.
func testProvider(t *testing.T, name string, mutate func(*access.Access)) (*Provider, *fakeSession) {
	t.Helper()
	h, tools, fs, _ := testEnv(t, mutate)
	return &Provider{name: name, cfg: tools.Cfg, tools: tools, handler: h}, fs
}

func TestCapabilities(t *testing.T) {
	p, _ := testProvider(t, "discord", nil)
	p.dg = nil // token-less: no permission relay
	caps := p.Capabilities()
	if !caps.Buttons || !caps.Reactions || !caps.Edits || !caps.TypingPause {
		t.Fatalf("caps %+v", caps)
	}
	if caps.PermissionRelay {
		t.Fatal("permission relay declared without a session")
	}
}

func TestRouterSourceRoutingWithTwoProviders(t *testing.T) {
	stub := &stubprovider.Stub{
		ProviderName: "telegram",
		Caps:         provider.Capabilities{Buttons: true},
	}
	dp, fs := testProvider(t, "discord", func(a *access.Access) {
		a.AllowFrom = []string{"u1"}
		a.BubbleMode = "instant"
	})
	recordDMChannel(dp.cfg.StateDir, "dchan", "u1")

	r, err := provider.NewRouter(stub, dp)
	if err != nil {
		t.Fatal(err)
	}

	// Outbound routes by source.
	if out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{Source: "discord", ChatID: "dchan", Text: "hi"}); isErr {
		t.Fatalf("discord reply errored: %s", out)
	}
	if len(fs.Sent) != 1 || len(stub.Replies) != 0 {
		t.Fatalf("discord=%d stub=%d", len(fs.Sent), len(stub.Replies))
	}
	if out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{Source: "telegram", ChatID: "55", Text: "yo"}); isErr {
		t.Fatalf("stub reply errored: %s", out)
	}
	if len(stub.Replies) != 1 || len(fs.Sent) != 1 {
		t.Fatalf("stub=%d discord=%d", len(stub.Replies), len(fs.Sent))
	}

	// Missing source with two providers is an error naming both.
	out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "x", Text: "?"})
	if !isErr || !strings.Contains(out, "telegram") || !strings.Contains(out, "discord") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}

// syncSink is a mutex-guarded sink shared across router goroutines.
type syncSink struct {
	mu    sync.Mutex
	metas []map[string]string
}

func (s *syncSink) SendChannel(_ context.Context, _ string, meta map[string]string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.metas = append(s.metas, meta)
	return nil
}

func (s *syncSink) SendVerdict(context.Context, string, string) error { return nil }

func TestRouterTagsDiscordInboundSource(t *testing.T) {
	stub := &stubprovider.Stub{
		ProviderName:  "telegram",
		InboundEvents: []stubprovider.Inbound{{Content: "from tg", Meta: map[string]string{"chat_id": "1"}}},
	}
	dp, _ := testProvider(t, "discord", func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})

	r, err := provider.NewRouter(stub, dp)
	if err != nil {
		t.Fatal(err)
	}

	// testEnv pre-binds a capture sink; clear it so we can observe the router
	// binding its source-tagging wrapper.
	dp.handler.BindNotifier(nil)

	sink := &syncSink{}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- r.Start(ctx, sink) }()

	// Wait for Start to bind the discord handler's sink, then push an inbound.
	deadline := time.Now().Add(2 * time.Second)
	for dp.handler.Notifier() == nil {
		if time.Now().After(deadline) {
			t.Fatal("router never bound the discord sink")
		}
		time.Sleep(5 * time.Millisecond)
	}
	dp.handler.HandleMessage(ctx, inboundMsg("9", "dchan", "u1", "from discord."))
	dp.handler.FlushAll(ctx)

	cancel()
	if err := <-done; err != nil {
		t.Fatalf("router start: %v", err)
	}

	sink.mu.Lock()
	defer sink.mu.Unlock()
	sources := map[string]bool{}
	for _, m := range sink.metas {
		sources[m["source"]] = true
	}
	if !sources["telegram"] || !sources["discord"] {
		t.Fatalf("sources seen: %v", sink.metas)
	}
}
