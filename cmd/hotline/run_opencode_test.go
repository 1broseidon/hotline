package main

import (
	"context"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/provider/stubprovider"
	"github.com/1broseidon/hotline/internal/schedule"
)

// robustSchedDir returns a temp dir whose cleanup retries RemoveAll: the
// fan-in loops (runPollers/runOpenCodeLoop) return on the first goroutine exit,
// so the scheduler goroutine may still be flushing one write to the store when
// the test ends. A plain t.TempDir() would then race with the writer and fail
// cleanup with "directory not empty"; the retry absorbs that sub-ms window.
func robustSchedDir(t *testing.T) string {
	t.Helper()
	d, err := os.MkdirTemp("", "hotline-sched")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		for i := 0; i < 100; i++ {
			if err := os.RemoveAll(d); err == nil {
				return
			}
			time.Sleep(10 * time.Millisecond)
		}
	})
	return d
}

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

// captureInbound is a provider.InboundSink that records delivered channel turns.
type captureInbound struct {
	mu       sync.Mutex
	channels []map[string]string
	contents []string
}

func (c *captureInbound) SendChannel(_ context.Context, content string, meta map[string]string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.channels = append(c.channels, meta)
	c.contents = append(c.contents, content)
	return nil
}
func (c *captureInbound) SendVerdict(_ context.Context, _, _ string) error { return nil }
func (c *captureInbound) len() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return len(c.channels)
}

// seedDueSchedule writes a single overdue daily schedule to a temp store and
// returns a scheduler over it (real clock — the eager scan fires it at once).
func seedDueSchedule(t *testing.T, sources []string) *schedule.Scheduler {
	t.Helper()
	path := filepath.Join(robustSchedDir(t), "schedules.json")
	past := time.Now().Add(-time.Hour).UTC().Format(time.RFC3339)
	doc := &schedule.Doc{Schedules: []schedule.Schedule{{
		ID: "aaaaaa", Prompt: "check the deploy", Source: "stub", ChatID: "123",
		Recurrence: schedule.Recurrence{Kind: schedule.KindDaily, TimeOfDay: "09:00"},
		NextFire:   past,
	}}}
	if err := schedule.Save(doc, path); err != nil {
		t.Fatal(err)
	}
	return schedule.NewScheduler(path, sources, nil)
}

// TestRunPollersDeliversScheduleFire proves the Claude-path fan-in (runPollers)
// starts the scheduler alongside the providers and its eager catch-up scan
// delivers the fire into the shared sink, tagged kind=schedule.
func TestRunPollersDeliversScheduleFire(t *testing.T) {
	stub := &stubprovider.Stub{ProviderName: "stub"}
	router, err := provider.NewRouter(stub)
	if err != nil {
		t.Fatal(err)
	}
	sched := seedDueSchedule(t, router.Sources())
	sink := &captureInbound{}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- runPollers(ctx, router, sched, sink) }()

	deadline := time.After(5 * time.Second)
	for sink.len() == 0 {
		select {
		case <-deadline:
			t.Fatal("schedule fire not delivered through runPollers")
		case <-time.After(5 * time.Millisecond):
		}
	}
	sink.mu.Lock()
	meta := sink.channels[0]
	sink.mu.Unlock()
	if meta["kind"] != "schedule" || meta["schedule_id"] != "aaaaaa" || meta["source"] != "stub" {
		t.Errorf("fire meta wrong: %+v", meta)
	}

	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("runPollers returned %v, want nil on cancel", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("runPollers did not return on cancel")
	}
}

// TestRunOpenCodeLoopExitsCleanly wires the OpenCode fan-in (with a scheduler
// over an empty store) and asserts it returns nil on ctx cancel.
func TestRunOpenCodeLoopExitsCleanly(t *testing.T) {
	stub := &stubprovider.Stub{ProviderName: "stub"}
	router, err := provider.NewRouter(stub)
	if err != nil {
		t.Fatal(err)
	}
	sched := schedule.NewScheduler(filepath.Join(robustSchedDir(t), "schedules.json"), router.Sources(), nil)
	link := newFakeLink()
	sink := &captureInbound{}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- runOpenCodeLoop(ctx, router, sched, link, false, sink) }()
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("runOpenCodeLoop returned %v, want nil on cancel", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("runOpenCodeLoop did not return on cancel")
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
