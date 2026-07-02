package signal

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

// testProvider assembles a full Provider around a testEnv handler/tools pair.
func testProvider(t *testing.T, name string, mutate func(*access.Access)) (*Provider, *fakeDaemon) {
	t.Helper()
	h, tools, d, _ := testEnv(t, mutate)
	return &Provider{name: name, cfg: h.Cfg, client: h.Client, tools: tools, handler: h}, d
}

func TestCapabilities(t *testing.T) {
	p, _ := testProvider(t, "signal", nil)
	caps := p.Capabilities()
	if caps.Buttons {
		t.Error("signal must not claim native buttons")
	}
	if !caps.Reactions || !caps.Edits || !caps.TypingPause {
		t.Errorf("caps %+v", caps)
	}
	if !caps.PermissionRelay {
		t.Error("configured provider should relay permissions")
	}

	unconfigured := &Provider{name: "signal"}
	if unconfigured.Capabilities().PermissionRelay {
		t.Error("unconfigured provider must not claim permission relay")
	}
}

// syncSink is a mutex-guarded sink shared across router goroutines.
type syncSink struct {
	mu       sync.Mutex
	contents []string
	metas    []map[string]string
}

func (s *syncSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.contents = append(s.contents, content)
	s.metas = append(s.metas, meta)
	return nil
}

func (s *syncSink) SendVerdict(context.Context, string, string) error { return nil }

// TestRouterThreeProviderComposition proves telegram+discord+signal compose:
// outbound routes by source to the right adapter, and signal's inbound events
// come out tagged source=signal.
func TestRouterThreeProviderComposition(t *testing.T) {
	tg := &stubprovider.Stub{
		ProviderName:  "telegram",
		Caps:          provider.Capabilities{Buttons: true},
		InboundEvents: []stubprovider.Inbound{{Content: "from tg", Meta: map[string]string{"chat_id": "1"}}},
	}
	dc := &stubprovider.Stub{
		ProviderName: "discord",
		Caps:         provider.Capabilities{Buttons: true},
	}
	sp, d := testProvider(t, "signal", func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		a.BubbleMode = "instant"
	})

	r, err := provider.NewRouter(tg, dc, sp)
	if err != nil {
		t.Fatal(err)
	}

	// Outbound routes by source.
	if out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{Source: "signal", ChatID: "+15550002222", Text: "hi"}); isErr {
		t.Fatalf("signal reply errored: %s", out)
	}
	if len(d.callsFor("send")) != 1 || len(tg.Replies) != 0 || len(dc.Replies) != 0 {
		t.Fatal("signal reply leaked to another provider")
	}
	if out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{Source: "discord", ChatID: "c", Text: "yo"}); isErr {
		t.Fatalf("discord reply errored: %s", out)
	}
	if len(dc.Replies) != 1 || len(tg.Replies) != 0 {
		t.Fatal("discord reply misrouted")
	}

	// Missing source with three providers is an error naming all of them.
	out, isErr := r.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "x", Text: "?"})
	if !isErr {
		t.Fatalf("missing source accepted: %q", out)
	}
	for _, name := range []string{"telegram", "discord", "signal"} {
		if !strings.Contains(out, name) {
			t.Errorf("error %q lacks %s", out, name)
		}
	}

	// Inbound: the router tags each provider's events with its source.
	sp.handler.BindNotifier(nil) // let the router bind its tagging wrapper
	sink := &syncSink{}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- r.Start(ctx, sink) }()

	deadline := time.After(5 * time.Second)
	for {
		sink.mu.Lock()
		n := len(sink.contents)
		sink.mu.Unlock()
		if n >= 1 {
			break
		}
		select {
		case <-deadline:
			t.Fatal("no inbound delivered")
		case <-time.After(10 * time.Millisecond):
		}
	}

	// Feed a signal envelope through the handler bound to the router's sink.
	sp.handler.HandleEnvelope(ctx, dmEnvelope("+15550002222", "George", "signal ping.", 42))

	var tgMeta, sigMeta map[string]string
	waitUntil := time.After(5 * time.Second)
	for tgMeta == nil || sigMeta == nil {
		sink.mu.Lock()
		for i, m := range sink.metas {
			switch m["source"] {
			case "telegram":
				tgMeta = m
			case "signal":
				sigMeta = m
				if sink.contents[i] != "signal ping." {
					t.Errorf("signal content %q", sink.contents[i])
				}
			}
		}
		sink.mu.Unlock()
		select {
		case <-waitUntil:
			t.Fatalf("missing tagged events: tg=%v sig=%v", tgMeta, sigMeta)
		case <-time.After(10 * time.Millisecond):
		}
	}
	if sigMeta["chat_id"] != "+15550002222" {
		t.Fatalf("signal meta %v", sigMeta)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("router did not stop")
	}
}
