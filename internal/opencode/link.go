package opencode

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/1broseidon/hotline/internal/harness"
)

// Link is the OpenCode implementation of harness.Link. It pushes inbound turns
// via prompt_async, tails the /event SSE stream, and surfaces permission.asked
// events as harness.PermissionRequest values keyed by a short relay code.
type Link struct {
	client *Client
	pinned string // OPENCODE_SESSION; empty means auto-resolve

	sessMu  sync.RWMutex
	session string // currently targeted session id (resolved / re-pinned)

	perms chan harness.PermissionRequest

	codeMu sync.Mutex
	codes  map[string]pendingPerm // relay code -> native (sessionID, permissionID)
}

// pendingPerm maps a relay code back to OpenCode's native addressing.
type pendingPerm struct {
	sessionID    string
	permissionID string
	at           time.Time
}

// permCacheTTL bounds how long an unanswered permission mapping is retained.
const permCacheTTL = 10 * time.Minute

// NewLink builds an OpenCode Link. pinnedSession, when non-empty, fixes the
// target session (OPENCODE_SESSION) and disables event-driven re-pinning.
func NewLink(serverURL, password, pinnedSession string) *Link {
	return &Link{
		client: NewClient(serverURL, password),
		pinned: pinnedSession,
		perms:  make(chan harness.PermissionRequest, 16),
		codes:  make(map[string]pendingPerm),
	}
}

// Permissions implements harness.Link.
func (l *Link) Permissions() <-chan harness.PermissionRequest { return l.perms }

// currentSession returns the resolved target session (may be empty before
// resolution).
func (l *Link) currentSession() string {
	l.sessMu.RLock()
	defer l.sessMu.RUnlock()
	return l.session
}

// setSession records the target session. Pinned mode is immutable once set.
func (l *Link) setSession(id string) {
	if id == "" {
		return
	}
	l.sessMu.Lock()
	defer l.sessMu.Unlock()
	if l.pinned != "" {
		l.session = l.pinned
		return
	}
	l.session = id
}

// Start implements harness.Link: resolve the initial target session, then tail
// the event stream (with reconnect/backoff) until ctx is cancelled. A failed
// initial resolution is non-fatal — the stream still runs and PushInbound
// resolves lazily — so a harness with no session yet doesn't crash the process.
func (l *Link) Start(ctx context.Context) error {
	defer close(l.perms)

	if id, err := l.client.ResolveSession(ctx, l.pinned); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: opencode session not resolved yet: %v (will retry on first inbound)\n", err)
	} else {
		l.setSession(id)
		fmt.Fprintf(os.Stderr, "hotline: opencode targeting session %s at %s\n", id, l.client.BaseURL)
	}

	l.client.runEventLoop(ctx, func(ev sseEvent) {
		l.handleEvent(ctx, ev)
	}, func(err error) {
		fmt.Fprintf(os.Stderr, "hotline: opencode event stream dropped: %v — reconnecting\n", err)
	})
	return nil
}

// PushInbound implements harness.Link: inject a user turn via prompt_async into
// the current session, resolving one lazily if none is set yet.
//
// Verified against opencode 1.17.11: an injected prompt_async turn becomes a
// first-class user message in the session — it appears in the message stream
// (message.updated / message.part.updated events) and the session's configured
// agent processes it, running tools and going idle as usual. RESIDUAL RISK
// (unverified): whether an interactive opencode TUI attached to the same session
// re-renders a server-injected turn live could not be confirmed from a headless
// box (no TUI). Since the turn is persisted to the session's message list, a TUI
// that reads that list should show it, but real-time repaint is untested.
func (l *Link) PushInbound(ctx context.Context, in harness.Inbound) error {
	session := l.currentSession()
	if session == "" {
		id, err := l.client.ResolveSession(ctx, l.pinned)
		if err != nil {
			return fmt.Errorf("no target opencode session: %w", err)
		}
		l.setSession(id)
		session = id
	}
	return l.client.PromptAsync(ctx, session, in.Content)
}

// AnswerPermission implements harness.Link: resolve the relay code back to the
// native (sessionID, permissionID) and POST the answer.
func (l *Link) AnswerPermission(ctx context.Context, id string, allow bool) error {
	l.codeMu.Lock()
	pp, ok := l.codes[id]
	if ok {
		delete(l.codes, id)
	}
	l.codeMu.Unlock()
	if !ok {
		return fmt.Errorf("unknown permission code %q", id)
	}
	return l.client.AnswerPermission(ctx, pp.sessionID, pp.permissionID, allow)
}

// busEvent is the SSE envelope: a type discriminator plus a raw properties
// payload decoded per-type.
type busEvent struct {
	Type       string          `json:"type"`
	Properties json.RawMessage `json:"properties"`
}

// permissionAskedProps is the payload of a permission.asked event.
//
// Verified against opencode 1.17.11 (OpenAPI EventPermissionAsked + a live
// event). The SSE envelope is {"id":"evt_…","type":"permission.asked",
// "properties":{…}} and the permission fields sit DIRECTLY under "properties"
// (not nested one level deeper). A real event:
//
//	{"id":"evt_…","type":"permission.asked","properties":{
//	  "id":"per_…","sessionID":"ses_…","permission":"bash",
//	  "patterns":["echo perm-check-2"],"metadata":{"command":"echo perm-check-2"},
//	  "always":["echo *"],"tool":{"messageID":"msg_…","callID":"call_…"}}}
//
// The permission NAME is "permission" (not "type"), the match patterns are a
// "patterns" ARRAY (not a scalar "pattern"), and there is NO "title" field.
type permissionAskedProps struct {
	ID         string          `json:"id"`
	SessionID  string          `json:"sessionID"`
	Permission string          `json:"permission"`
	Patterns   []string        `json:"patterns"`
	Metadata   json.RawMessage `json:"metadata"`
}

// sessionRef is the minimal shape of events that carry a session id, used to
// re-pin the target session onto whatever is actively doing work.
type sessionRef struct {
	SessionID string `json:"sessionID"`
	Info      struct {
		ID string `json:"id"`
	} `json:"info"`
}

// handleEvent decodes one SSE bus event and reacts to the types we care about.
// Unknown types are ignored.
func (l *Link) handleEvent(ctx context.Context, ev sseEvent) {
	if strings.TrimSpace(ev.Data) == "" {
		return
	}
	var be busEvent
	if err := json.Unmarshal([]byte(ev.Data), &be); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: opencode event decode failed: %v\n", err)
		return
	}
	switch be.Type {
	case "permission.asked":
		l.handlePermissionAsked(ctx, be.Properties)
	case "message.updated", "session.created":
		// Follow the active session: re-pin onto whatever is doing work (no-op
		// in pinned mode). session.idle is deliberately NOT followed — an idle
		// session is the one we want to leave, not target.
		l.followSession(be.Properties)
	}
}

// handlePermissionAsked mints a relay code for the prompt, remembers the native
// addressing, and emits a harness.PermissionRequest.
func (l *Link) handlePermissionAsked(ctx context.Context, raw json.RawMessage) {
	var p permissionAskedProps
	if err := json.Unmarshal(raw, &p); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: opencode permission decode failed: %v\n", err)
		return
	}
	if p.ID == "" {
		fmt.Fprintf(os.Stderr, "hotline: opencode permission.asked missing id — dropping\n")
		return
	}
	// A permission is a strong signal that this session is the active one.
	l.setSession(p.SessionID)

	code := l.remember(p.SessionID, p.ID)
	firstPattern := ""
	if len(p.Patterns) > 0 {
		firstPattern = p.Patterns[0]
	}
	req := harness.PermissionRequest{
		ID:           code,
		ToolName:     firstNonEmpty(p.Permission, firstPattern, "permission"),
		Description:  firstNonEmpty(firstPattern, p.Permission),
		InputPreview: previewMetadata(p.Metadata),
	}
	select {
	case l.perms <- req:
	case <-ctx.Done():
	}
}

// followSession re-pins the target session from an event that carries one.
func (l *Link) followSession(raw json.RawMessage) {
	var r sessionRef
	if err := json.Unmarshal(raw, &r); err != nil {
		return
	}
	if r.SessionID != "" {
		l.setSession(r.SessionID)
	} else if r.Info.ID != "" {
		l.setSession(r.Info.ID)
	}
}

// remember stores a code->native mapping (purging stale entries) and returns the
// code.
func (l *Link) remember(sessionID, permissionID string) string {
	l.codeMu.Lock()
	defer l.codeMu.Unlock()
	l.purgeLocked()
	code := newCode()
	for _, taken := l.codes[code]; taken; _, taken = l.codes[code] {
		code = newCode()
	}
	l.codes[code] = pendingPerm{sessionID: sessionID, permissionID: permissionID, at: time.Now()}
	return code
}

// purgeLocked drops permission mappings older than permCacheTTL.
func (l *Link) purgeLocked() {
	cutoff := time.Now().Add(-permCacheTTL)
	for code, pp := range l.codes {
		if pp.at.Before(cutoff) {
			delete(l.codes, code)
		}
	}
}

// firstNonEmpty returns the first non-empty string, or "".
func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}

// previewMetadata renders a compact one-line preview of a permission's metadata
// for the relayed prompt. Empty/absent metadata yields "".
func previewMetadata(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	s := strings.Join(strings.Fields(string(raw)), " ")
	if len(s) > 300 {
		s = s[:300] + "…"
	}
	return s
}
