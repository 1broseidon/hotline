package main

import (
	"context"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/provider/stubprovider"
)

// fakeLink is an in-memory harness.Link for wiring tests.
type fakeLink struct {
	mu      sync.Mutex
	pushes  []harness.Inbound
	answers []fakeAnswer
	perms   chan harness.PermissionRequest
}

type fakeAnswer struct {
	id    string
	allow bool
}

func newFakeLink() *fakeLink {
	return &fakeLink{perms: make(chan harness.PermissionRequest, 4)}
}

func (f *fakeLink) Start(ctx context.Context) error {
	<-ctx.Done()
	close(f.perms)
	return nil
}

func (f *fakeLink) PushInbound(_ context.Context, in harness.Inbound) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.pushes = append(f.pushes, in)
	return nil
}

func (f *fakeLink) Permissions() <-chan harness.PermissionRequest { return f.perms }

func (f *fakeLink) AnswerPermission(_ context.Context, id string, allow bool) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.answers = append(f.answers, fakeAnswer{id: id, allow: allow})
	return nil
}

// TestOpenCodeSinkBridges verifies the provider.InboundSink -> harness.Link
// mapping: SendChannel becomes an inbound push; SendVerdict becomes a permission
// answer with allow/deny derived from the behavior string.
func TestOpenCodeSinkBridges(t *testing.T) {
	link := newFakeLink()
	var sink provider.InboundSink = &opencodeSink{link: link}

	if err := sink.SendChannel(context.Background(), "hello", map[string]string{"user": "george"}); err != nil {
		t.Fatal(err)
	}
	if err := sink.SendVerdict(context.Background(), "abcde", "allow"); err != nil {
		t.Fatal(err)
	}
	if err := sink.SendVerdict(context.Background(), "fghij", "deny"); err != nil {
		t.Fatal(err)
	}

	link.mu.Lock()
	defer link.mu.Unlock()
	if len(link.pushes) != 1 || link.pushes[0].Content != "hello" || link.pushes[0].Meta["user"] != "george" {
		t.Fatalf("pushes %+v", link.pushes)
	}
	want := []fakeAnswer{{"abcde", true}, {"fghij", false}}
	if len(link.answers) != 2 || link.answers[0] != want[0] || link.answers[1] != want[1] {
		t.Fatalf("answers %+v", link.answers)
	}
}

// TestPumpPermissionsRelaysToRouter proves a harness permission prompt fans out
// through the router to a relay-capable provider, code preserved as RequestID.
func TestPumpPermissionsRelaysToRouter(t *testing.T) {
	stub := &stubprovider.Stub{
		ProviderName: "stub",
		Caps:         provider.Capabilities{PermissionRelay: true},
	}
	router, err := provider.NewRouter(stub)
	if err != nil {
		t.Fatal(err)
	}
	link := newFakeLink()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	go func() { _ = pumpPermissions(ctx, router, link, true); close(done) }()

	link.perms <- harness.PermissionRequest{ID: "abcde", ToolName: "bash", Description: "run", InputPreview: "ls"}

	deadline := time.After(5 * time.Second)
	for {
		if stub.PermRequestsLen() > 0 {
			break
		}
		select {
		case <-deadline:
			t.Fatal("permission not relayed")
		case <-time.After(5 * time.Millisecond):
		}
	}
	got := stub.PermRequestsAt(0)
	if got.RequestID != "abcde" || got.ToolName != "bash" || got.InputPreview != "ls" {
		t.Fatalf("relayed %+v", got)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("pump did not stop on cancel")
	}
}
