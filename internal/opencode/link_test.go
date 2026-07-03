package opencode

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

// mockServer is a minimal in-memory OpenCode server for the Link flow test.
type mockServer struct {
	mu           sync.Mutex
	prompts      []recordedPrompt
	permAnswers  []recordedAnswer
	permEvent    string // the raw JSON permission.asked event to emit once
	sessionsJSON string
}

type recordedPrompt struct {
	path string
	body promptRequest
}

type recordedAnswer struct {
	path string
	body permissionAnswer
}

func (m *mockServer) handler() http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/session" && r.Method == http.MethodGet:
			_, _ = w.Write([]byte(m.sessionsJSON))

		case r.URL.Path == "/event" && r.Method == http.MethodGet:
			w.Header().Set("Content-Type", "text/event-stream")
			w.WriteHeader(http.StatusOK)
			fl, _ := w.(http.Flusher)
			if m.permEvent != "" {
				_, _ = w.Write([]byte("data:" + m.permEvent + "\n\n"))
				if fl != nil {
					fl.Flush()
				}
			}
			// Hold the stream open until the client disconnects.
			<-r.Context().Done()

		case strings.HasSuffix(r.URL.Path, "/prompt_async") && r.Method == http.MethodPost:
			var body promptRequest
			_ = json.NewDecoder(r.Body).Decode(&body)
			m.mu.Lock()
			m.prompts = append(m.prompts, recordedPrompt{path: r.URL.Path, body: body})
			m.mu.Unlock()
			w.WriteHeader(http.StatusOK) // empty body: the #2168 caveat

		case strings.Contains(r.URL.Path, "/permissions/") && r.Method == http.MethodPost:
			var body permissionAnswer
			_ = json.NewDecoder(r.Body).Decode(&body)
			m.mu.Lock()
			m.permAnswers = append(m.permAnswers, recordedAnswer{path: r.URL.Path, body: body})
			m.mu.Unlock()
			w.WriteHeader(http.StatusOK)

		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}
}

// TestLinkFullFlow exercises the whole seam offline: session resolution, an SSE
// permission.asked surfaced as a relay-coded PermissionRequest, answering it via
// the permissions endpoint, inbound push via prompt_async, and the
// empty-response caveat (results never taken from the POST body).
func TestLinkFullFlow(t *testing.T) {
	mock := &mockServer{
		// GET /session lists an older session; the live SSE event re-pins onto
		// ses_live, so pushes and answers must target THAT session.
		sessionsJSON: `[{"id":"ses_list","time":{"created":10,"updated":10}}]`,
		permEvent:    `{"type":"permission.asked","properties":{"id":"perm_xyz","sessionID":"ses_live","title":"Run rm -rf","type":"bash","metadata":{"command":"rm -rf /tmp/x"}}}`,
	}
	srv := httptest.NewServer(mock.handler())
	defer srv.Close()

	link := NewLink(srv.URL, "", "") // auto-resolve session
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	done := make(chan struct{})
	go func() { _ = link.Start(ctx); close(done) }()

	// 1. Permission surfaces as a PermissionRequest with a relay-safe code.
	var req harness.PermissionRequest
	select {
	case req = <-link.Permissions():
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for permission event")
	}
	if !mcpchan.PermReplyRe.MatchString("yes " + req.ID) {
		t.Fatalf("permission code %q is not relay-safe", req.ID)
	}
	if req.ToolName != "bash" {
		t.Fatalf("tool name %q, want bash", req.ToolName)
	}
	if req.Description != "Run rm -rf" {
		t.Fatalf("description %q", req.Description)
	}
	if !strings.Contains(req.InputPreview, "rm -rf") {
		t.Fatalf("preview %q missing metadata", req.InputPreview)
	}

	// 2. Answer it: the native permissionID + re-pinned session must round-trip.
	if err := link.AnswerPermission(ctx, req.ID, true); err != nil {
		t.Fatalf("AnswerPermission: %v", err)
	}
	// Answering an unknown code errors.
	if err := link.AnswerPermission(ctx, "zzzzz", true); err == nil {
		t.Fatal("expected error answering unknown code")
	}

	// 3. Push an inbound turn.
	if err := link.PushInbound(ctx, harness.Inbound{Content: "deploy it"}); err != nil {
		t.Fatalf("PushInbound: %v", err)
	}

	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.permAnswers) != 1 {
		t.Fatalf("permission answers: %+v", mock.permAnswers)
	}
	if mock.permAnswers[0].path != "/session/ses_live/permissions/perm_xyz" {
		t.Fatalf("answer path %q (session re-pin failed?)", mock.permAnswers[0].path)
	}
	if mock.permAnswers[0].body.Response != "once" {
		t.Fatalf("answer response %q", mock.permAnswers[0].body.Response)
	}
	if len(mock.prompts) != 1 {
		t.Fatalf("prompts: %+v", mock.prompts)
	}
	if mock.prompts[0].path != "/session/ses_live/prompt_async" {
		t.Fatalf("prompt path %q (expected re-pinned ses_live)", mock.prompts[0].path)
	}
	if len(mock.prompts[0].body.Parts) != 1 || mock.prompts[0].body.Parts[0].Text != "deploy it" {
		t.Fatalf("prompt body %+v", mock.prompts[0].body)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("link did not stop on cancel")
	}
	// The permissions channel closes when Start returns.
	if _, ok := <-link.Permissions(); ok {
		t.Fatal("permissions channel should be closed after Start returns")
	}
}

// TestLinkPushResolvesLazily proves a push with no prior session resolves via
// GET /session on demand (no SSE event needed).
func TestLinkPushResolvesLazily(t *testing.T) {
	mock := &mockServer{sessionsJSON: `[{"id":"ses_only","time":{"created":1,"updated":5}}]`}
	srv := httptest.NewServer(mock.handler())
	defer srv.Close()

	// Don't run Start: exercise PushInbound's lazy resolution directly.
	link := NewLink(srv.URL, "", "")
	if err := link.PushInbound(context.Background(), harness.Inbound{Content: "hi"}); err != nil {
		t.Fatalf("PushInbound: %v", err)
	}
	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.prompts) != 1 || mock.prompts[0].path != "/session/ses_only/prompt_async" {
		t.Fatalf("prompts: %+v", mock.prompts)
	}
}

// TestLinkPinnedSession keeps the pinned session even when SSE events name a
// different one.
func TestLinkPinnedSession(t *testing.T) {
	mock := &mockServer{
		sessionsJSON: `[{"id":"ses_list","time":{"created":1,"updated":9}}]`,
		permEvent:    `{"type":"permission.asked","properties":{"id":"perm_1","sessionID":"ses_other","title":"x","type":"bash"}}`,
	}
	srv := httptest.NewServer(mock.handler())
	defer srv.Close()

	link := NewLink(srv.URL, "", "ses_pinned")
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = link.Start(ctx) }()

	select {
	case <-link.Permissions():
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for permission event")
	}
	if err := link.PushInbound(ctx, harness.Inbound{Content: "yo"}); err != nil {
		t.Fatalf("PushInbound: %v", err)
	}
	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.prompts) != 1 || mock.prompts[0].path != "/session/ses_pinned/prompt_async" {
		t.Fatalf("pinned push went to wrong session: %+v", mock.prompts)
	}
}
