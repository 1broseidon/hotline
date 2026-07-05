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
)

// feedServer is a controllable in-memory OpenCode server: it streams SSE events
// fed on demand (so a test can order turn-lifecycle events precisely) and
// records prompt_async pushes.
type feedServer struct {
	sessionsJSON string
	feed         chan string // raw JSON event bodies to emit on /event

	mu      sync.Mutex
	prompts []string // prompt_async text bodies, in order
}

func newFeedServer(sessionsJSON string) *feedServer {
	return &feedServer{sessionsJSON: sessionsJSON, feed: make(chan string, 16)}
}

func (m *feedServer) handler() http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/session" && r.Method == http.MethodGet:
			_, _ = w.Write([]byte(m.sessionsJSON))

		case r.URL.Path == "/event" && r.Method == http.MethodGet:
			w.Header().Set("Content-Type", "text/event-stream")
			w.WriteHeader(http.StatusOK)
			fl, _ := w.(http.Flusher)
			for {
				select {
				case <-r.Context().Done():
					return
				case body := <-m.feed:
					_, _ = w.Write([]byte("data:" + body + "\n\n"))
					if fl != nil {
						fl.Flush()
					}
				}
			}

		case strings.HasSuffix(r.URL.Path, "/prompt_async") && r.Method == http.MethodPost:
			var body promptRequest
			_ = json.NewDecoder(r.Body).Decode(&body)
			m.mu.Lock()
			if len(body.Parts) > 0 {
				m.prompts = append(m.prompts, body.Parts[0].Text)
			}
			m.mu.Unlock()
			w.WriteHeader(http.StatusOK) // empty body: the #2168 caveat

		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}
}

func (m *feedServer) promptsSnapshot() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]string(nil), m.prompts...)
}

// TestReplyFallbackNudgeThenForward mirrors the live glm-5.2 failure: the model
// answers in plain text (assistant text part, no reply tool call), the turn goes
// idle, and the user would otherwise see nothing. On the first idle the Link must
// nudge once; if the nudge turn ALSO idles with no reply, the Link must
// direct-forward the buffered assistant text to the channel.
func TestReplyFallbackNudgeThenForward(t *testing.T) {
	const answer = "the available subagents are: Explore, Plan, general-purpose."
	mock := newFeedServer(`[{"id":"ses_live","time":{"created":10,"updated":10}}]`)
	srv := httptest.NewServer(mock.handler())
	defer srv.Close()

	// forwarded captures direct-forward calls (the backstop's send path).
	type forwarded struct {
		text string
		meta map[string]string
	}
	fwdCh := make(chan forwarded, 4)

	link := NewLink(srv.URL, "", "")
	link.SetForwarder(func(_ context.Context, text string, meta map[string]string) error {
		fwdCh <- forwarded{text: text, meta: meta}
		return nil
	})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = link.Start(ctx) }()

	// Inbound user turn (resets fallback state, stashes routing meta for the
	// backstop). This targets ses_live via GET /session.
	in := harness.Inbound{
		Content: "what subagents are available",
		Meta:    map[string]string{"source": "telegram", "chat_id": "412407481"},
	}
	if err := link.PushInbound(ctx, in); err != nil {
		t.Fatalf("PushInbound: %v", err)
	}

	// The model answers in plain text: an assistant message with a text part, no
	// reply tool call. Order matters — the message must be seen as assistant
	// before its text part is buffered.
	mock.feed <- `{"type":"message.updated","properties":{"info":{"id":"msg_a","sessionID":"ses_live","role":"assistant"}}}`
	mock.feed <- `{"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_live","messageID":"msg_a","type":"text","text":"` + answer + `"},"delta":"` + answer + `"}}`
	mock.feed <- `{"type":"session.idle","properties":{"sessionID":"ses_live"}}`

	// First idle: exactly one nudge is pushed, and nothing is forwarded yet.
	waitFor(t, 3*time.Second, func() bool {
		return containsPrompt(mock.promptsSnapshot(), replyNudge)
	}, "nudge prompt was not pushed on first idle")

	select {
	case f := <-fwdCh:
		t.Fatalf("direct-forward fired on the FIRST idle (should nudge first): %+v", f)
	case <-time.After(150 * time.Millisecond):
	}

	// The nudge turn also idles with no reply: the backstop must forward.
	mock.feed <- `{"type":"session.idle","properties":{"sessionID":"ses_live"}}`

	select {
	case f := <-fwdCh:
		if f.text != answer {
			t.Fatalf("forwarded text = %q, want the assistant answer %q", f.text, answer)
		}
		if f.meta["chat_id"] != "412407481" || f.meta["source"] != "telegram" {
			t.Fatalf("forwarded meta = %+v, want the inbound routing keys", f.meta)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("backstop did not direct-forward on the second idle")
	}

	// The nudge must fire exactly once (no loops), even across the second idle.
	if n := countPrompt(mock.promptsSnapshot(), replyNudge); n != 1 {
		t.Fatalf("nudge fired %d times, want exactly 1", n)
	}
}

// TestReplyFallbackSkippedWhenReplied proves a turn that DID call the reply tool
// gets no nudge and no direct-forward — the fallback only rescues dropped
// messages.
func TestReplyFallbackSkippedWhenReplied(t *testing.T) {
	mock := newFeedServer(`[{"id":"ses_live","time":{"created":10,"updated":10}}]`)
	srv := httptest.NewServer(mock.handler())
	defer srv.Close()

	fwded := make(chan struct{}, 2)
	link := NewLink(srv.URL, "", "")
	link.SetForwarder(func(_ context.Context, _ string, _ map[string]string) error {
		fwded <- struct{}{}
		return nil
	})

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go func() { _ = link.Start(ctx) }()

	if err := link.PushInbound(ctx, harness.Inbound{
		Content: "hi",
		Meta:    map[string]string{"source": "telegram", "chat_id": "1"},
	}); err != nil {
		t.Fatalf("PushInbound: %v", err)
	}

	mock.feed <- `{"type":"message.updated","properties":{"info":{"id":"msg_a","sessionID":"ses_live","role":"assistant"}}}`
	mock.feed <- `{"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_live","messageID":"msg_a","type":"text","text":"answered here"},"delta":"answered here"}}`

	// The model actually called reply this turn.
	link.MarkReplied()

	mock.feed <- `{"type":"session.idle","properties":{"sessionID":"ses_live"}}`

	// Give the idle time to be processed: no nudge, no forward.
	time.Sleep(200 * time.Millisecond)
	if containsPrompt(mock.promptsSnapshot(), replyNudge) {
		t.Fatal("nudged a turn that already replied")
	}
	select {
	case <-fwded:
		t.Fatal("direct-forwarded a turn that already replied")
	default:
	}
}

// waitFor polls cond until it is true or the timeout elapses, failing with msg.
func waitFor(t *testing.T, timeout time.Duration, cond func() bool, msg string) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal(msg)
}

func containsPrompt(prompts []string, want string) bool {
	return countPrompt(prompts, want) > 0
}

func countPrompt(prompts []string, want string) int {
	n := 0
	for _, p := range prompts {
		if p == want {
			n++
		}
	}
	return n
}
