package codex

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
)

// ForwardFunc delivers completed Codex agent messages straight to the channel.
// Codex Phase 1 does not expose hotline's MCP reply tool surface to the model;
// the Link is the delivery path.
type ForwardFunc func(ctx context.Context, text string, meta map[string]string) error

// Options configures a Codex Link.
type Options struct {
	CWD                   string
	ThreadID              string
	ThreadFile            string
	ApprovalPolicy        string
	Sandbox               string
	DeveloperInstructions string
	AutoDenyPermissions   bool

	// RWC injects a JSONL app-server stream for tests. When nil, Start spawns
	// `codex app-server`.
	RWC ioReadWriteCloser
}

type ioReadWriteCloser interface {
	Read([]byte) (int, error)
	Write([]byte) (int, error)
	Close() error
}

// Link implements harness.Link for codex app-server.
type Link struct {
	opts Options

	client *Client

	perms chan harness.PermissionRequest

	ready     chan struct{}
	readyOnce sync.Once
	startErr  error

	threadMu sync.RWMutex
	threadID string

	turnMu     sync.Mutex
	activeTurn string
	activeMeta map[string]string
	deltaText  map[string]string
	sentItems  map[string]bool
	forward    ForwardFunc

	codeMu sync.Mutex
	codes  map[string]pendingApproval
}

type pendingApproval struct {
	requestID int64
	method    string
	choices   []string
	at        time.Time
}

const approvalCacheTTL = 10 * time.Minute

// NewLink builds a Codex Link. Defaults are intentionally the empirically
// verified app-server values from codex-cli 0.142.5.
func NewLink(opts Options) *Link {
	if opts.ApprovalPolicy == "" {
		opts.ApprovalPolicy = "untrusted"
	}
	if opts.Sandbox == "" {
		opts.Sandbox = "workspace-write"
	}
	return &Link{
		opts:      opts,
		perms:     make(chan harness.PermissionRequest, 16),
		ready:     make(chan struct{}),
		deltaText: make(map[string]string),
		sentItems: make(map[string]bool),
		codes:     make(map[string]pendingApproval),
	}
}

// Permissions implements harness.Link.
func (l *Link) Permissions() <-chan harness.PermissionRequest { return l.perms }

// SetForwarder installs the direct-send path to the messaging channel.
func (l *Link) SetForwarder(fn ForwardFunc) { l.forward = fn }

// Start initializes app-server, resumes or starts the single Codex thread, and
// processes app-server events until ctx is cancelled.
func (l *Link) Start(ctx context.Context) error {
	defer close(l.perms)

	client, err := l.openClient()
	if err != nil {
		l.markReady(err)
		return err
	}
	l.client = client
	defer client.Close()

	if err := client.Initialize(ctx); err != nil {
		l.markReady(err)
		return err
	}
	if err := l.openThread(ctx); err != nil {
		l.markReady(err)
		return err
	}
	l.markReady(nil)

	for {
		select {
		case <-ctx.Done():
			return nil
		case req, ok := <-client.Requests():
			if !ok {
				return linkDone(ctx, client)
			}
			l.handleServerRequest(ctx, req)
		case n, ok := <-client.Notifications():
			if !ok {
				return linkDone(ctx, client)
			}
			l.handleNotification(ctx, n)
		case err, ok := <-client.Done():
			if !ok {
				return nil
			}
			if ctx.Err() != nil {
				return nil
			}
			return err
		}
	}
}

func linkDone(ctx context.Context, client *Client) error {
	if ctx.Err() != nil {
		return nil
	}
	if err, ok := <-client.Done(); ok {
		return err
	}
	return nil
}

func (l *Link) openClient() (*Client, error) {
	if l.opts.RWC != nil {
		return NewClient(l.opts.RWC), nil
	}
	return StartAppServer(l.opts.CWD)
}

func (l *Link) markReady(err error) {
	l.startErr = err
	l.readyOnce.Do(func() { close(l.ready) })
}

func (l *Link) waitReady(ctx context.Context) error {
	select {
	case <-l.ready:
		return l.startErr
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (l *Link) openThread(ctx context.Context) error {
	threadID := firstNonEmpty(l.opts.ThreadID, readThreadID(l.opts.ThreadFile))
	if threadID != "" {
		if err := l.resumeThread(ctx, threadID); err == nil {
			l.setThread(threadID)
			return nil
		} else {
			// Codex context does survive process restarts in the 0.142.5 live
			// verifier when the on-disk rollout exists. If this local resume
			// fails, start fresh and rely on hotline's transcript.jsonl as the
			// cross-restart memory backstop instead of pretending continuity was
			// preserved.
			fmt.Fprintf(os.Stderr, "hotline: codex thread %s not resumed: %v; starting a new thread\n", threadID, err)
		}
	}
	started, err := l.startThread(ctx)
	if err != nil {
		return err
	}
	l.setThread(started)
	writeThreadID(l.opts.ThreadFile, started)
	return nil
}

func (l *Link) setThread(id string) {
	l.threadMu.Lock()
	l.threadID = id
	l.threadMu.Unlock()
}

func (l *Link) currentThread() string {
	l.threadMu.RLock()
	defer l.threadMu.RUnlock()
	return l.threadID
}

func (l *Link) startThread(ctx context.Context) (string, error) {
	var out threadResponse
	err := l.client.Request(ctx, "thread/start", l.threadParams(""), &out)
	if err != nil {
		return "", err
	}
	if out.Thread.ID == "" {
		return "", fmt.Errorf("codex thread/start returned no thread id")
	}
	return out.Thread.ID, nil
}

func (l *Link) resumeThread(ctx context.Context, threadID string) error {
	var out threadResponse
	if err := l.client.Request(ctx, "thread/resume", l.threadParams(threadID), &out); err != nil {
		return err
	}
	if out.Thread.ID == "" {
		return fmt.Errorf("codex thread/resume returned no thread id")
	}
	return nil
}

func (l *Link) threadParams(threadID string) map[string]any {
	params := map[string]any{
		"cwd":                   l.opts.CWD,
		"runtimeWorkspaceRoots": []string{l.opts.CWD},
		"approvalPolicy":        l.opts.ApprovalPolicy,
		"approvalsReviewer":     "user",
		"sandbox":               l.opts.Sandbox,
		"developerInstructions": l.opts.DeveloperInstructions,
	}
	if threadID != "" {
		params["threadId"] = threadID
	}
	return params
}

type threadResponse struct {
	Thread struct {
		ID   string `json:"id"`
		Path string `json:"path"`
	} `json:"thread"`
}

// PushInbound implements harness.Link.
func (l *Link) PushInbound(ctx context.Context, in harness.Inbound) error {
	if err := l.waitReady(ctx); err != nil {
		return err
	}
	threadID := l.currentThread()
	if threadID == "" {
		return fmt.Errorf("codex thread is not ready")
	}

	input := userInput(in)
	l.turnMu.Lock()
	defer l.turnMu.Unlock()

	if l.activeTurn != "" {
		err := l.steer(ctx, threadID, l.activeTurn, input)
		if err == nil {
			l.activeMeta = copyMeta(in.Meta)
			return nil
		}
		if !strings.Contains(err.Error(), "no active turn to steer") {
			return err
		}
		l.clearActiveLocked()
	}

	turnID, err := l.startTurn(ctx, threadID, input)
	if err != nil {
		return err
	}
	l.activeTurn = turnID
	l.activeMeta = copyMeta(in.Meta)
	l.deltaText = make(map[string]string)
	l.sentItems = make(map[string]bool)
	return nil
}

func userInput(in harness.Inbound) []map[string]any {
	items := []map[string]any{{
		"type":          "text",
		"text":          harness.RenderChannel(in),
		"text_elements": []any{},
	}}
	if imagePath := in.Meta["image_path"]; strings.TrimSpace(imagePath) != "" {
		items = append(items, map[string]any{
			"type": "localImage",
			"path": imagePath,
		})
	}
	return items
}

func (l *Link) startTurn(ctx context.Context, threadID string, input []map[string]any) (string, error) {
	var out struct {
		Turn struct {
			ID string `json:"id"`
		} `json:"turn"`
	}
	if err := l.client.Request(ctx, "turn/start", map[string]any{
		"threadId": threadID,
		"input":    input,
	}, &out); err != nil {
		return "", err
	}
	if out.Turn.ID == "" {
		return "", fmt.Errorf("codex turn/start returned no turn id")
	}
	return out.Turn.ID, nil
}

func (l *Link) steer(ctx context.Context, threadID, turnID string, input []map[string]any) error {
	return l.client.Request(ctx, "turn/steer", map[string]any{
		"threadId":       threadID,
		"expectedTurnId": turnID,
		"input":          input,
	}, nil)
}

func (l *Link) handleNotification(ctx context.Context, n Notification) {
	switch n.Method {
	case "item/agentMessage/delta":
		l.handleAgentDelta(n.Params)
	case "item/completed":
		l.handleItemCompleted(ctx, n.Params)
	case "turn/completed":
		l.handleTurnCompleted(ctx, n.Params)
	case "thread/status/changed":
		// Status notifications are observed in live app-server traces but not
		// needed for routing; turn/completed is the authoritative clear.
	}
}

type agentDeltaParams struct {
	TurnID string `json:"turnId"`
	ItemID string `json:"itemId"`
	Delta  string `json:"delta"`
}

func (l *Link) handleAgentDelta(raw json.RawMessage) {
	var p agentDeltaParams
	if err := json.Unmarshal(raw, &p); err != nil || p.ItemID == "" {
		return
	}
	l.turnMu.Lock()
	if p.TurnID == l.activeTurn {
		l.deltaText[p.ItemID] += p.Delta
	}
	l.turnMu.Unlock()
}

type itemCompletedParams struct {
	TurnID string `json:"turnId"`
	Item   struct {
		Type  string `json:"type"`
		ID    string `json:"id"`
		Text  string `json:"text"`
		Phase string `json:"phase"`
	} `json:"item"`
}

func (l *Link) handleItemCompleted(ctx context.Context, raw json.RawMessage) {
	var p itemCompletedParams
	if err := json.Unmarshal(raw, &p); err != nil || p.Item.Type != "agentMessage" {
		return
	}

	l.turnMu.Lock()
	if p.TurnID != l.activeTurn || p.Item.ID == "" || l.sentItems[p.Item.ID] {
		l.turnMu.Unlock()
		return
	}
	text := p.Item.Text
	if text == "" {
		text = l.deltaText[p.Item.ID]
	}
	if strings.TrimSpace(text) == "" {
		l.turnMu.Unlock()
		return
	}
	l.sentItems[p.Item.ID] = true
	meta := copyMeta(l.activeMeta)
	l.turnMu.Unlock()

	l.directForward(ctx, text, meta)
}

type turnCompletedParams struct {
	TurnID string `json:"turnId"`
	Turn   struct {
		ID     string `json:"id"`
		Status string `json:"status"`
		Error  any    `json:"error"`
	} `json:"turn"`
}

func (l *Link) handleTurnCompleted(ctx context.Context, raw json.RawMessage) {
	var p turnCompletedParams
	if err := json.Unmarshal(raw, &p); err != nil {
		return
	}
	turnID := p.Turn.ID
	if turnID == "" {
		turnID = p.TurnID
	}

	var fallbackText string
	var meta map[string]string
	l.turnMu.Lock()
	if turnID == l.activeTurn {
		for id, text := range l.deltaText {
			if !l.sentItems[id] && strings.TrimSpace(text) != "" {
				if fallbackText != "" {
					fallbackText += "\n\n"
				}
				fallbackText += text
			}
		}
		meta = copyMeta(l.activeMeta)
		l.clearActiveLocked()
	}
	l.turnMu.Unlock()

	if fallbackText != "" {
		l.directForward(ctx, fallbackText, meta)
	}
}

func (l *Link) clearActiveLocked() {
	l.activeTurn = ""
	l.activeMeta = nil
	l.deltaText = make(map[string]string)
	l.sentItems = make(map[string]bool)
}

func (l *Link) directForward(ctx context.Context, text string, meta map[string]string) {
	if l.forward == nil {
		fmt.Fprintf(os.Stderr, "hotline: codex direct-forward: no forwarder wired; dropping %d bytes\n", len(text))
		return
	}
	if err := l.forward(ctx, text, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: codex direct-forward failed: %v\n", err)
	}
}

func (l *Link) handleServerRequest(ctx context.Context, req ServerRequest) {
	switch req.Method {
	case "item/commandExecution/requestApproval", "item/fileChange/requestApproval":
		l.handleApprovalRequest(ctx, req)
	case "currentTime/read":
		_ = l.client.Respond(ctx, req.ID, map[string]any{"currentTimeAt": time.Now().Unix()})
	default:
		_ = l.client.RespondError(ctx, req.ID, -32601, "unsupported server request: "+req.Method)
	}
}

type approvalParams struct {
	ThreadID           string            `json:"threadId"`
	TurnID             string            `json:"turnId"`
	Reason             string            `json:"reason"`
	Command            string            `json:"command"`
	CWD                string            `json:"cwd"`
	CommandActions     []commandAction   `json:"commandActions"`
	FileChanges        map[string]any    `json:"fileChanges"`
	AvailableDecisions []json.RawMessage `json:"availableDecisions"`
}

type commandAction struct {
	Type    string `json:"type"`
	Command string `json:"command"`
	Name    string `json:"name"`
	Path    string `json:"path"`
}

func (l *Link) handleApprovalRequest(ctx context.Context, req ServerRequest) {
	var p approvalParams
	if err := json.Unmarshal(req.Params, &p); err != nil {
		_ = l.client.Respond(ctx, req.ID, map[string]any{"decision": "cancel"})
		return
	}
	choices := decisionChoices(p.AvailableDecisions)
	if l.opts.AutoDenyPermissions {
		_ = l.client.Respond(ctx, req.ID, map[string]any{"decision": chooseDecision(choices, false)})
		return
	}

	code := l.remember(req.ID, req.Method, choices)
	perm := harness.PermissionRequest{
		ID:           code,
		ToolName:     approvalToolName(req.Method, p),
		Description:  approvalDescription(req.Method, p),
		InputPreview: approvalPreview(req.Method, p),
	}
	select {
	case l.perms <- perm:
	case <-ctx.Done():
	}
}

func approvalToolName(method string, p approvalParams) string {
	if method == "item/fileChange/requestApproval" {
		return "Edit"
	}
	if len(p.CommandActions) > 0 && p.CommandActions[0].Type != "" && p.CommandActions[0].Type != "unknown" {
		return p.CommandActions[0].Type
	}
	return "Bash"
}

func approvalDescription(method string, p approvalParams) string {
	if strings.TrimSpace(p.Reason) != "" {
		return p.Reason
	}
	if method == "item/fileChange/requestApproval" {
		return "apply file changes"
	}
	return firstNonEmpty(p.Command, "run command")
}

func approvalPreview(method string, p approvalParams) string {
	if method == "item/fileChange/requestApproval" {
		keys := make([]string, 0, len(p.FileChanges))
		for k := range p.FileChanges {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		return strings.Join(keys, "\n")
	}
	return p.Command
}

func decisionChoices(raw []json.RawMessage) []string {
	var out []string
	for _, item := range raw {
		var s string
		if err := json.Unmarshal(item, &s); err == nil && s != "" {
			out = append(out, s)
		}
	}
	return out
}

// chooseDecision mirrors the live codex-cli 0.142.5 approval protocol. Command
// approval requests advertised "accept" and "cancel"; responding "cancel" made
// the turn complete with status "interrupted" in the live verifier.
func chooseDecision(choices []string, allow bool) string {
	has := func(want string) bool {
		for _, c := range choices {
			if c == want {
				return true
			}
		}
		return false
	}
	if allow {
		if has("accept") {
			return "accept"
		}
		if has("approved") {
			return "approved"
		}
		return "accept"
	}
	if has("cancel") {
		return "cancel"
	}
	if has("decline") {
		return "decline"
	}
	if has("denied") {
		return "denied"
	}
	return "cancel"
}

// AnswerPermission implements harness.Link.
func (l *Link) AnswerPermission(ctx context.Context, id string, allow bool) error {
	l.codeMu.Lock()
	ap, ok := l.codes[id]
	if ok {
		delete(l.codes, id)
	}
	l.codeMu.Unlock()
	if !ok {
		return fmt.Errorf("unknown permission code %q", id)
	}
	return l.client.Respond(ctx, ap.requestID, map[string]any{
		"decision": chooseDecision(ap.choices, allow),
	})
}

func (l *Link) remember(requestID int64, method string, choices []string) string {
	l.codeMu.Lock()
	defer l.codeMu.Unlock()
	l.purgeLocked()
	code := newCode()
	for _, taken := l.codes[code]; taken; _, taken = l.codes[code] {
		code = newCode()
	}
	l.codes[code] = pendingApproval{requestID: requestID, method: method, choices: choices, at: time.Now()}
	return code
}

func (l *Link) purgeLocked() {
	cutoff := time.Now().Add(-approvalCacheTTL)
	for code, ap := range l.codes {
		if ap.at.Before(cutoff) {
			delete(l.codes, code)
		}
	}
}

func readThreadID(path string) string {
	if path == "" {
		return ""
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(raw))
}

func writeThreadID(path, id string) {
	if path == "" || id == "" {
		return
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: codex could not create thread state dir: %v\n", err)
		return
	}
	if err := os.WriteFile(path, []byte(id+"\n"), 0o600); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: codex could not write thread id: %v\n", err)
	}
}

func copyMeta(m map[string]string) map[string]string {
	if len(m) == 0 {
		return nil
	}
	out := make(map[string]string, len(m))
	for k, v := range m {
		out[k] = v
	}
	return out
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}
