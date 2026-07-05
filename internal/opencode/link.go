package opencode

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sort"
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
	// agent is the opencode agent every inbound turn (and the reply nudge) is
	// pinned to (HOTLINE_OPENCODE_AGENT). Empty omits the agent field, so the
	// session's default agent handles the turn — the backward-compatible default.
	agent string

	sessMu  sync.RWMutex
	session string // currently targeted session id (resolved / re-pinned)

	perms chan harness.PermissionRequest

	codeMu sync.Mutex
	codes  map[string]pendingPerm // relay code -> native (sessionID, permissionID)

	turnMu sync.Mutex
	turns  map[string]*turnState // session id -> current assistant-turn state

	// forward delivers assistant text straight to the messaging channel for the
	// direct-forward backstop (wired in cmd/hotline from the provider router). A
	// nil forward disables the backstop; the nudge still runs.
	forward ForwardFunc
}

// ForwardFunc delivers assistant text straight to the messaging channel,
// bypassing the model. It is the direct-forward backstop's send path — meta
// carries the routing keys (source, chat_id) captured from the inbound turn.
type ForwardFunc func(ctx context.Context, text string, meta map[string]string) error

// turnState tracks one session's in-flight assistant turn for the reply-delivery
// fallback: whether a reply was already delivered through the reply MCP tool,
// whether we have already nudged, and a buffer of the assistant's user-facing
// text parts so the backstop can forward them verbatim. It is reset on every
// inbound user turn (PushInbound); the injected nudge deliberately does NOT reset
// it. All fields are guarded by Link.turnMu.
type turnState struct {
	replied   bool
	nudged    bool
	assistant map[string]bool   // assistant message ids seen this turn
	text      map[string]string // assistant text part id -> latest (full) text
	meta      map[string]string // routing meta of the inbound turn (source, chat_id)
}

// assistantText assembles the buffered text parts in part-id order (the order
// the model appends them), joined with blank lines. Empty when the turn produced
// no user-facing text.
func (t *turnState) assistantText() string {
	if len(t.text) == 0 {
		return ""
	}
	ids := make([]string, 0, len(t.text))
	for id := range t.text {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	var b strings.Builder
	for _, id := range ids {
		if strings.TrimSpace(t.text[id]) == "" {
			continue
		}
		if b.Len() > 0 {
			b.WriteString("\n\n")
		}
		b.WriteString(t.text[id])
	}
	return b.String()
}

// replyNudge is the one-shot prompt injected when an assistant turn completes
// without calling the reply tool. It is compiled in (never user-supplied) and
// pushed straight into the opencode session, so it is invisible to the user and
// does not reset the turn's fallback state.
const replyNudge = "⚠ You produced a response but never sent it. The user only sees messages sent through the hotline `reply` tool — your plain text is invisible to them. Send your answer now using `reply`."

// pendingPerm maps a relay code back to OpenCode's native addressing.
type pendingPerm struct {
	sessionID    string
	permissionID string
	at           time.Time
}

// permCacheTTL bounds how long an unanswered permission mapping is retained.
const permCacheTTL = 10 * time.Minute

// NewLink builds an OpenCode Link. pinnedSession, when non-empty, fixes the
// target session (OPENCODE_SESSION) and disables event-driven re-pinning. agent,
// when non-empty, pins every pushed turn to a named opencode agent
// (HOTLINE_OPENCODE_AGENT); "" preserves the pre-agent default-agent behavior.
func NewLink(serverURL, password, pinnedSession, agent string) *Link {
	return &Link{
		client: NewClient(serverURL, password),
		pinned: pinnedSession,
		agent:  agent,
		perms:  make(chan harness.PermissionRequest, 16),
		codes:  make(map[string]pendingPerm),
		turns:  make(map[string]*turnState),
	}
}

// Permissions implements harness.Link.
func (l *Link) Permissions() <-chan harness.PermissionRequest { return l.perms }

// SetForwarder installs the direct-forward backstop's send path. Wire it before
// Start; the wiring (cmd/hotline) closes over the provider router's reply.
func (l *Link) SetForwarder(fn ForwardFunc) { l.forward = fn }

// MarkReplied records that the current session's assistant turn delivered a
// reply through the reply MCP tool, so the session-idle fallback skips the
// nudge/direct-forward ladder for it. The reply handler (mcpchan, via the
// provider router) calls this. Single process, shared state — guarded by turnMu.
func (l *Link) MarkReplied() {
	session := l.currentSession()
	if session == "" {
		return
	}
	l.turnMu.Lock()
	l.turnLocked(session).replied = true
	l.turnMu.Unlock()
}

// turnLocked returns the session's turn state, creating an empty one if needed.
// The caller must hold turnMu.
func (l *Link) turnLocked(session string) *turnState {
	ts := l.turns[session]
	if ts == nil {
		ts = &turnState{assistant: map[string]bool{}, text: map[string]string{}}
		l.turns[session] = ts
	}
	return ts
}

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
	// A new user turn resets the reply-delivery fallback: the model gets a fresh
	// chance to call reply, buffered assistant text is cleared, and the inbound
	// routing meta is stashed for the direct-forward backstop.
	l.turnMu.Lock()
	l.turns[session] = &turnState{
		assistant: map[string]bool{},
		text:      map[string]string{},
		meta:      copyMeta(in.Meta),
	}
	l.turnMu.Unlock()
	// Frame the turn as the <channel …> envelope so the agent reads chat_id and
	// source off the tag and echoes them back into hotline_reply. Claude Code
	// renders this same envelope client-side from the notification meta; here we
	// render it into the prompt text since OpenCode only speaks plain text. Using
	// in.Content alone would drop in.Meta — the routing keys — and the agent
	// would have no chat_id to reply to.
	return l.client.PromptAsync(ctx, session, l.agent, harness.RenderChannel(in))
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
	case "session.created":
		// Follow the active session: re-pin onto whatever is doing work (no-op
		// in pinned mode). session.idle re-pins nothing — an idle session is the
		// one we want to leave, not target — but it does drive the fallback.
		l.followSession(be.Properties)
	case "message.updated":
		l.handleMessageUpdated(be.Properties)
	case "message.part.updated":
		l.handlePartUpdated(be.Properties)
	case "session.idle":
		l.handleSessionIdle(ctx, be.Properties)
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

// messageUpdatedProps is the payload of a message.updated event: the message
// envelope under "info". Verified against captured opencode events — info
// carries id, role ("user"/"assistant"), and sessionID.
type messageUpdatedProps struct {
	Info struct {
		ID        string `json:"id"`
		Role      string `json:"role"`
		SessionID string `json:"sessionID"`
	} `json:"info"`
}

// partUpdatedProps is the payload of a message.part.updated event: one message
// part under "part". A text part carries the running assistant text; the
// verified shape is {"part":{"id":…,"sessionID":…,"messageID":…,"type":"text",
// "text":…},"delta":…}. Parts stream incrementally — each event carries the full
// text so far — so we overwrite by part id.
type partUpdatedProps struct {
	Part struct {
		ID        string `json:"id"`
		SessionID string `json:"sessionID"`
		MessageID string `json:"messageID"`
		Type      string `json:"type"`
		Text      string `json:"text"`
	} `json:"part"`
}

// sessionIdleProps is the payload of a session.idle event: the id of the session
// whose assistant turn just completed.
type sessionIdleProps struct {
	SessionID string `json:"sessionID"`
}

// handleMessageUpdated re-pins onto the active session and records which message
// ids belong to the assistant, so handlePartUpdated buffers only assistant text
// (never the user's own echoed prompt).
func (l *Link) handleMessageUpdated(raw json.RawMessage) {
	var p messageUpdatedProps
	if err := json.Unmarshal(raw, &p); err != nil {
		return
	}
	if p.Info.SessionID != "" {
		l.setSession(p.Info.SessionID)
	}
	if p.Info.Role != "assistant" || p.Info.SessionID == "" || p.Info.ID == "" {
		return
	}
	l.turnMu.Lock()
	l.turnLocked(p.Info.SessionID).assistant[p.Info.ID] = true
	l.turnMu.Unlock()
}

// handlePartUpdated buffers the latest text of each assistant text part for the
// current turn. Non-text parts, and parts on messages not yet marked assistant,
// are ignored.
func (l *Link) handlePartUpdated(raw json.RawMessage) {
	var p partUpdatedProps
	if err := json.Unmarshal(raw, &p); err != nil {
		return
	}
	part := p.Part
	if part.Type != "text" || part.SessionID == "" || part.MessageID == "" {
		return
	}
	l.turnMu.Lock()
	ts := l.turnLocked(part.SessionID)
	if ts.assistant[part.MessageID] {
		ts.text[part.ID] = part.Text
	}
	l.turnMu.Unlock()
}

// handleSessionIdle drives the reply-delivery fallback when an assistant turn
// completes. See evaluateFallback for the ladder.
func (l *Link) handleSessionIdle(ctx context.Context, raw json.RawMessage) {
	var p sessionIdleProps
	if err := json.Unmarshal(raw, &p); err != nil {
		return
	}
	if p.SessionID == "" {
		return
	}
	l.evaluateFallback(ctx, p.SessionID)
}

// evaluateFallback is the "nudge once, then force-forward" ladder, run at each
// session-idle for a session with a tracked turn:
//
//  1. If the model already delivered a reply this turn, or the turn produced no
//     user-facing text, do nothing (never force a spurious reply).
//  2. Otherwise, if we have not nudged yet, push a one-shot nudge asking the
//     model to resend via the reply tool, and stop. The nudge runs one more turn
//     and normally ends in a reply (clean path). It is pushed straight into the
//     session — not via PushInbound — so it neither resets the turn flags nor
//     renders as a channel message.
//  3. If we already nudged and the turn STILL idled with no reply, the model is
//     not going to comply: direct-forward its buffered text to the channel so the
//     user is never left hanging. Never nudge twice (no loops).
func (l *Link) evaluateFallback(ctx context.Context, session string) {
	l.turnMu.Lock()
	ts := l.turns[session]
	if ts == nil || ts.replied || ts.assistantText() == "" {
		l.turnMu.Unlock()
		return
	}
	if !ts.nudged {
		ts.nudged = true
		l.turnMu.Unlock()
		if err := l.client.PromptAsync(ctx, session, l.agent, replyNudge); err != nil {
			// The nudge could not even be delivered — skip to the backstop so the
			// answer still reaches the user.
			fmt.Fprintf(os.Stderr, "hotline: opencode reply nudge failed: %v — forwarding directly\n", err)
			l.directForward(ctx, session)
		}
		return
	}
	l.turnMu.Unlock()
	// Already nudged and still no reply: guaranteed-delivery backstop.
	l.directForward(ctx, session)
}

// directForward sends the turn's buffered assistant text to the messaging
// channel verbatim, bypassing the model, then marks the turn replied so no later
// idle re-sends it. A nil forwarder (unwired) logs and drops.
func (l *Link) directForward(ctx context.Context, session string) {
	l.turnMu.Lock()
	ts := l.turns[session]
	if ts == nil || ts.replied {
		l.turnMu.Unlock()
		return
	}
	text := ts.assistantText()
	meta := copyMeta(ts.meta)
	ts.replied = true // prevent loops / double-sends on subsequent idles
	l.turnMu.Unlock()

	if text == "" {
		return
	}
	if l.forward == nil {
		fmt.Fprintf(os.Stderr, "hotline: opencode reply fallback: no forwarder wired — dropping %d bytes of assistant text\n", len(text))
		return
	}
	if err := l.forward(ctx, text, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: opencode direct-forward failed: %v\n", err)
	}
}

// copyMeta returns a shallow copy of m (nil for an empty map) so buffered routing
// meta is insulated from later mutation of the caller's map.
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
	preview := string(raw)
	// Unwrap a flat metadata object into its human-relevant value so the preview
	// reads as a plain command/path ("echo hi") rather than raw JSON
	// ({"command":"echo hi"}). Non-object metadata falls through to the raw form.
	var obj map[string]any
	if err := json.Unmarshal(raw, &obj); err == nil && len(obj) > 0 {
		if v := pickMetaValue(obj); v != "" {
			preview = v
		}
	}
	preview = strings.Join(strings.Fields(preview), " ")
	if len(preview) > 300 {
		preview = preview[:300] + "…"
	}
	return preview
}

// pickMetaValue extracts the human-relevant string from a permission's metadata:
// a well-known field naming what the action touches (command / path / url / …),
// else every string value in sorted-key order for a stable result. Returns "" if
// nothing string-like is present (caller keeps the raw form).
func pickMetaValue(obj map[string]any) string {
	for _, k := range []string{"command", "filePath", "filepath", "path", "file", "url", "pattern", "glob", "query"} {
		if v, ok := obj[k].(string); ok && strings.TrimSpace(v) != "" {
			return v
		}
	}
	keys := make([]string, 0, len(obj))
	for k := range obj {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	var vals []string
	for _, k := range keys {
		if s, ok := obj[k].(string); ok && strings.TrimSpace(s) != "" {
			vals = append(vals, s)
		}
	}
	return strings.Join(vals, " ")
}
