package codex

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

type pipeRWC struct {
	r *io.PipeReader
	w *io.PipeWriter
}

func (p *pipeRWC) Read(b []byte) (int, error)  { return p.r.Read(b) }
func (p *pipeRWC) Write(b []byte) (int, error) { return p.w.Write(b) }
func (p *pipeRWC) Close() error {
	_ = p.r.Close()
	_ = p.w.Close()
	return nil
}

func pipePair() (*pipeRWC, *pipeRWC) {
	ar, bw := io.Pipe()
	br, aw := io.Pipe()
	return &pipeRWC{r: ar, w: aw}, &pipeRWC{r: br, w: bw}
}

type fakeServer struct {
	t *testing.T

	rwc *pipeRWC
	sc  *bufio.Scanner

	mu       sync.Mutex
	messages []rpcMessage
}

func newFakeServer(t *testing.T, rwc *pipeRWC) *fakeServer {
	t.Helper()
	sc := bufio.NewScanner(rwc)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	return &fakeServer{t: t, rwc: rwc, sc: sc}
}

func (s *fakeServer) next(timeout time.Duration) rpcMessage {
	s.t.Helper()
	ch := make(chan rpcMessage, 1)
	go func() {
		if !s.sc.Scan() {
			ch <- rpcMessage{}
			return
		}
		var msg rpcMessage
		if err := json.Unmarshal(s.sc.Bytes(), &msg); err != nil {
			s.t.Errorf("decode client message: %v", err)
		}
		s.mu.Lock()
		s.messages = append(s.messages, msg)
		s.mu.Unlock()
		ch <- msg
	}()
	select {
	case msg := <-ch:
		return msg
	case <-time.After(timeout):
		s.t.Fatalf("timed out waiting for client message")
		return rpcMessage{}
	}
}

func (s *fakeServer) write(v any) {
	s.t.Helper()
	raw, err := json.Marshal(v)
	if err != nil {
		s.t.Fatal(err)
	}
	if _, err := s.rwc.Write(append(raw, '\n')); err != nil {
		s.t.Fatalf("write server message: %v", err)
	}
}

func (s *fakeServer) respond(id *int64, result any) {
	s.t.Helper()
	if id == nil {
		s.t.Fatal("response id is nil")
	}
	s.write(map[string]any{"id": *id, "result": result})
}

func (s *fakeServer) handshake(threadID string) map[string]any {
	s.t.Helper()
	init := s.next(5 * time.Second)
	if init.Method != "initialize" {
		s.t.Fatalf("first method %q, want initialize", init.Method)
	}
	s.respond(init.ID, map[string]any{"userAgent": "fake", "codexHome": "/tmp/codex"})

	initialized := s.next(5 * time.Second)
	if initialized.Method != "initialized" {
		s.t.Fatalf("second method %q, want initialized", initialized.Method)
	}

	start := s.next(5 * time.Second)
	if start.Method != "thread/start" {
		s.t.Fatalf("third method %q, want thread/start", start.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(start.Params, &params); err != nil {
		s.t.Fatal(err)
	}
	s.respond(start.ID, map[string]any{
		"thread": map[string]any{"id": threadID, "path": "/tmp/thread.jsonl"},
	})
	return params
}

func newStartedLink(t *testing.T, opts Options) (*Link, *fakeServer, context.CancelFunc) {
	t.Helper()
	clientRWC, serverRWC := pipePair()
	opts.RWC = clientRWC
	if opts.CWD == "" {
		opts.CWD = t.TempDir()
	}
	link := NewLink(opts)
	server := newFakeServer(t, serverRWC)
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		_ = link.Start(ctx)
	}()
	return link, server, cancel
}

func TestLinkHandshakeDeveloperInstructionsAndClose(t *testing.T) {
	link, server, cancel := newStartedLink(t, Options{DeveloperInstructions: "DIRECT-FORWARD"})
	params := server.handshake("thread_1")

	if params["developerInstructions"] != "DIRECT-FORWARD" {
		t.Fatalf("developerInstructions %q", params["developerInstructions"])
	}
	if params["approvalPolicy"] != "untrusted" || params["approvalsReviewer"] != "user" || params["sandbox"] != "workspace-write" {
		t.Fatalf("thread/start params %+v", params)
	}

	cancel()
	_ = link.client.Close()
	select {
	case _, ok := <-link.Permissions():
		if ok {
			t.Fatal("permissions channel should close after Start returns")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("permissions channel did not close")
	}
}

func TestLinkPushInboundStartsTurnAndForwardsCompletedAgentMessage(t *testing.T) {
	link, server, cancel := newStartedLink(t, Options{})
	defer cancel()
	server.handshake("thread_1")

	var forwardsMu sync.Mutex
	var forwards []struct {
		text string
		meta map[string]string
	}
	link.SetForwarder(func(_ context.Context, text string, meta map[string]string) error {
		forwardsMu.Lock()
		defer forwardsMu.Unlock()
		forwards = append(forwards, struct {
			text string
			meta map[string]string
		}{text: text, meta: meta})
		return nil
	})

	errCh := make(chan error, 1)
	go func() {
		errCh <- link.PushInbound(context.Background(), harness.Inbound{
			Content: "look",
			Meta: map[string]string{
				"source":     "telegram",
				"chat_id":    "42",
				"image_path": "/tmp/pixel.png",
			},
		})
	}()

	turnStart := server.next(5 * time.Second)
	if turnStart.Method != "turn/start" {
		t.Fatalf("method %q, want turn/start", turnStart.Method)
	}
	var params struct {
		ThreadID string           `json:"threadId"`
		Input    []map[string]any `json:"input"`
	}
	if err := json.Unmarshal(turnStart.Params, &params); err != nil {
		t.Fatal(err)
	}
	if params.ThreadID != "thread_1" {
		t.Fatalf("thread id %q", params.ThreadID)
	}
	if len(params.Input) != 2 || params.Input[1]["type"] != "localImage" {
		t.Fatalf("input %+v, want text + localImage", params.Input)
	}
	text, _ := params.Input[0]["text"].(string)
	if !strings.Contains(text, `chat_id="42"`) || !strings.Contains(text, "look") {
		t.Fatalf("rendered channel missing meta/content:\n%s", text)
	}
	sid := *turnStart.ID
	server.respond(&sid, map[string]any{"turn": map[string]any{"id": "turn_1"}})
	if err := <-errCh; err != nil {
		t.Fatalf("PushInbound: %v", err)
	}

	server.write(map[string]any{
		"method": "item/completed",
		"params": map[string]any{
			"threadId": "thread_1",
			"turnId":   "turn_1",
			"item": map[string]any{
				"type":  "agentMessage",
				"id":    "msg_1",
				"text":  "hello back",
				"phase": "final_answer",
			},
		},
	})
	deadline := time.After(5 * time.Second)
	for {
		forwardsMu.Lock()
		n := len(forwards)
		forwardsMu.Unlock()
		if n > 0 {
			break
		}
		select {
		case <-deadline:
			t.Fatal("timed out waiting for forward")
		case <-time.After(5 * time.Millisecond):
		}
	}
	forwardsMu.Lock()
	defer forwardsMu.Unlock()
	if forwards[0].text != "hello back" || forwards[0].meta["chat_id"] != "42" {
		t.Fatalf("forwards %+v", forwards)
	}
}

func TestLinkApprovalRoundTrip(t *testing.T) {
	link, server, cancel := newStartedLink(t, Options{})
	defer cancel()
	server.handshake("thread_1")

	server.write(map[string]any{
		"id":     0,
		"method": "item/commandExecution/requestApproval",
		"params": map[string]any{
			"threadId": "thread_1",
			"turnId":   "turn_1",
			"command":  "/usr/bin/zsh -lc 'echo hi'",
			"reason":   "command failed; retry without sandbox?",
			"commandActions": []any{
				map[string]any{"type": "unknown", "command": "echo hi"},
			},
			"availableDecisions": []any{"accept", map[string]any{"acceptWithExecpolicyAmendment": map[string]any{}}, "cancel"},
		},
	})

	var req harness.PermissionRequest
	select {
	case req = <-link.Permissions():
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for permission")
	}
	if !mcpchan.PermReplyRe.MatchString("yes " + req.ID) {
		t.Fatalf("permission code %q is not relay-safe", req.ID)
	}
	if req.ToolName != "Bash" || !strings.Contains(req.InputPreview, "echo hi") {
		t.Fatalf("permission %+v", req)
	}

	answerErr := make(chan error, 1)
	go func() {
		answerErr <- link.AnswerPermission(context.Background(), req.ID, true)
	}()
	resp := server.next(5 * time.Second)
	if err := <-answerErr; err != nil {
		t.Fatalf("AnswerPermission: %v", err)
	}
	if resp.ID == nil || *resp.ID != 0 {
		t.Fatalf("response id %+v, want 0", resp.ID)
	}
	var result map[string]string
	if err := json.Unmarshal(resp.Result, &result); err != nil {
		t.Fatal(err)
	}
	if result["decision"] != "accept" {
		t.Fatalf("decision %q, want accept", result["decision"])
	}
}

func TestLinkSteersWhenTurnActive(t *testing.T) {
	link, server, cancel := newStartedLink(t, Options{})
	defer cancel()
	server.handshake("thread_1")

	first := make(chan error, 1)
	go func() {
		first <- link.PushInbound(context.Background(), harness.Inbound{Content: "first", Meta: map[string]string{"chat_id": "1"}})
	}()
	start := server.next(5 * time.Second)
	if start.Method != "turn/start" {
		t.Fatalf("method %q, want turn/start", start.Method)
	}
	server.respond(start.ID, map[string]any{"turn": map[string]any{"id": "turn_1"}})
	if err := <-first; err != nil {
		t.Fatalf("first push: %v", err)
	}

	second := make(chan error, 1)
	go func() {
		second <- link.PushInbound(context.Background(), harness.Inbound{Content: "second", Meta: map[string]string{"chat_id": "1"}})
	}()
	steer := server.next(5 * time.Second)
	if steer.Method != "turn/steer" {
		t.Fatalf("method %q, want turn/steer", steer.Method)
	}
	var params map[string]any
	if err := json.Unmarshal(steer.Params, &params); err != nil {
		t.Fatal(err)
	}
	if params["expectedTurnId"] != "turn_1" {
		t.Fatalf("expectedTurnId %q", params["expectedTurnId"])
	}
	server.respond(steer.ID, map[string]any{"turnId": "turn_1"})
	if err := <-second; err != nil {
		t.Fatalf("second push: %v", err)
	}
}

func TestChooseDecisionDenyPrefersCancel(t *testing.T) {
	if got := chooseDecision([]string{"accept", "cancel"}, false); got != "cancel" {
		t.Fatalf("deny decision %q, want cancel", got)
	}
	if got := chooseDecision([]string{"accept", "decline"}, false); got != "decline" {
		t.Fatalf("deny decision %q, want decline fallback", got)
	}
}
