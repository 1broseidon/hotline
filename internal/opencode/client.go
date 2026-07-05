// Package opencode adapts an OpenCode harness (opencode.ai; repo
// anomalyco/opencode) to hotline's harness.Link seam. Like the signal adapter
// it holds no model credentials and speaks pure net/http against a locally
// running server (`opencode serve`, default port 4096) over two surfaces:
//
//   - the request endpoints — POST /session/:id/prompt_async (fire-and-forget
//     inbound push), POST /session/:id/permissions/:permID (permission answer),
//     GET /session (session list for target resolution)
//   - GET /event — an SSE stream of bus events (permission.asked,
//     message.updated, session.idle, session.created)
//
// Results are NOT read from the prompt_async / message POST response body: a
// known build returns empty responses (sst/opencode#2168). The assistant's work
// surfaces to the user through the existing outbound MCP tool surface, which
// OpenCode drives as any other MCP server; this adapter only pushes turns IN and
// relays permission prompts.
package opencode

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"
	"time"
)

// Client is a minimal net/http client for the OpenCode server. A single Client
// is shared by the request path and the (separately dialed) SSE stream.
type Client struct {
	// BaseURL is the server root, e.g. "http://127.0.0.1:4096" (no trailing
	// slash).
	BaseURL string
	// Password is the optional basic-auth secret (OPENCODE_SERVER_PASSWORD).
	// Empty means no Authorization header is sent.
	Password string
	// HTTP is the client for request-path calls; a 30s-timeout client when nil.
	// The long-lived SSE stream uses its own untimed client (see sse.go).
	HTTP *http.Client
}

// NewClient builds a Client for the server at baseURL with an optional
// basic-auth password.
func NewClient(baseURL, password string) *Client {
	return &Client{
		BaseURL:  strings.TrimRight(baseURL, "/"),
		Password: password,
		HTTP:     &http.Client{Timeout: 30 * time.Second},
	}
}

// newRequest builds a request with the base URL, JSON content type (when body
// is non-nil), and basic auth (when a password is set).
//
// Verified against opencode 1.17.11: when OPENCODE_SERVER_PASSWORD is set the
// server requires HTTP Basic auth with the literal username "opencode" and the
// password as the secret. An empty username (or any other username) yields 401;
// only "opencode:<password>" is accepted. When no password is set the server is
// unsecured and no Authorization header is sent.
func (c *Client) newRequest(ctx context.Context, method, path string, body io.Reader) (*http.Request, error) {
	req, err := http.NewRequestWithContext(ctx, method, c.BaseURL+path, body)
	if err != nil {
		return nil, err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if c.Password != "" {
		req.SetBasicAuth("opencode", c.Password)
	}
	return req, nil
}

// doJSON sends a JSON request (body may be nil) and returns the response after
// checking for a 2xx status. The body is drained and closed; callers that don't
// need it (prompt_async, permission answer — see the empty-response caveat) pass
// out=nil.
func (c *Client) doJSON(ctx context.Context, method, path string, in, out any) error {
	var rdr io.Reader
	if in != nil {
		raw, err := json.Marshal(in)
		if err != nil {
			return err
		}
		rdr = bytes.NewReader(raw)
	}
	req, err := c.newRequest(ctx, method, path, rdr)
	if err != nil {
		return err
	}
	client := c.HTTP
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("opencode server unreachable: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("opencode server: %s %s -> HTTP %d: %s", method, path, resp.StatusCode, strings.TrimSpace(string(snippet)))
	}
	if out == nil {
		// Deliberately ignore the body: prompt_async/permission answers carry
		// nothing we rely on (results come over SSE).
		_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 1<<20))
		return nil
	}
	return json.NewDecoder(io.LimitReader(resp.Body, 100<<20)).Decode(out)
}

// sessionInfo is the subset of a GET /session entry we use for target
// resolution.
//
// Verified against opencode 1.17.11: each entry exposes a string "id" (pattern
// ^ses) and a nested "time" object with integer "created"/"updated" epoch-ms
// timestamps (both required). JSON numbers decode cleanly into float64.
type sessionInfo struct {
	ID   string `json:"id"`
	Time struct {
		Created float64 `json:"created"`
		Updated float64 `json:"updated"`
	} `json:"time"`
}

// activity is the timestamp used to rank recency: the later of updated/created.
func (s sessionInfo) activity() float64 {
	if s.Time.Updated > s.Time.Created {
		return s.Time.Updated
	}
	return s.Time.Created
}

// ListSessions fetches GET /session.
func (c *Client) ListSessions(ctx context.Context) ([]sessionInfo, error) {
	var out []sessionInfo
	if err := c.doJSON(ctx, http.MethodGet, "/session", nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// ResolveSession returns pinned when non-empty, otherwise the most-recently
// active session id from GET /session.
//
// Heuristic (a design choice, not a gap): the list endpoint does not expose a
// per-session idle flag, so "most-recent non-idle" is approximated by the
// session with the latest time.updated (falling back to time.created). Whatever
// session the user is actively driving is the one being touched most recently,
// so it sorts first. The Link additionally re-pins onto whichever session emits
// live events (see link.go), which corrects the choice as soon as work happens.
func (c *Client) ResolveSession(ctx context.Context, pinned string) (string, error) {
	if pinned != "" {
		return pinned, nil
	}
	sessions, err := c.ListSessions(ctx)
	if err != nil {
		return "", err
	}
	if len(sessions) == 0 {
		return "", fmt.Errorf("no opencode sessions found at %s (start one, or pin OPENCODE_SESSION)", c.BaseURL)
	}
	sort.SliceStable(sessions, func(i, j int) bool {
		return sessions[i].activity() > sessions[j].activity()
	})
	return sessions[0].ID, nil
}

// promptPart is one part of an OpenCode prompt message. Only text parts are
// sent for an injected user turn.
type promptPart struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// promptRequest is the POST /session/:id/prompt_async body.
//
// Verified against opencode 1.17.11: the body is an object whose only required
// field is "parts" (an array of typed parts). agent/model/messageID/noReply are
// all optional — omitting them lets the session's configured agent+model handle
// the turn. A text part is {"type":"text","text":…} (both required). A live POST
// with just {"parts":[{"type":"text","text":…}]} returns HTTP 204 and the turn
// is processed by the default agent.
//
// Agent, when non-empty, pins the turn to a named agent (opencode's optional
// "agent" field). hotline sets it to the scaffolded "hotline" agent so inbound
// turns run hotline's mechanics+voice instead of the session's default "build"
// agent. The omitempty tag keeps the key ABSENT when Agent is "" — that is the
// backward-compatible default-agent behavior, and tests assert the key is gone.
type promptRequest struct {
	Parts []promptPart `json:"parts"`
	Agent string       `json:"agent,omitempty"`
}

// PromptAsync fires a user turn into a session (fire-and-forget). The response
// body is intentionally discarded (sst/opencode#2168): a build is known to
// return empty here, so results are taken from the SSE stream, never this call.
//
// agent pins the turn to a named opencode agent; pass "" to omit the field
// entirely and let the session's default agent handle the turn.
func (c *Client) PromptAsync(ctx context.Context, sessionID, agent, text string) error {
	body := promptRequest{Parts: []promptPart{{Type: "text", Text: text}}, Agent: agent}
	return c.doJSON(ctx, http.MethodPost, "/session/"+sessionID+"/prompt_async", body, nil)
}

// permissionAnswer is the POST /session/:id/permissions/:permID body.
//
// Verified against opencode 1.17.11: the body is {"response": …} where response
// is one of exactly "once", "always", or "reject" (there is no "allow"/"deny").
// We map allow -> "once" (approve this one call) and deny -> "reject"; "always"
// is unused (our seam has no remember bit). A live POST with {"response":"once"}
// returns HTTP 200 with body `true` and emits a "permission.replied" event.
// NOTE: this endpoint is marked deprecated in the OpenAPI spec (superseded by
// POST /session/:id/permission/:requestID/reply under the /api prefix) but is
// still fully functional in 1.17.11.
type permissionAnswer struct {
	Response string `json:"response"`
}

// answerResponse maps the seam's allow bool to OpenCode's response token.
func answerResponse(allow bool) string {
	if allow {
		return "once"
	}
	return "reject"
}

// AnswerPermission answers a pending permission request.
func (c *Client) AnswerPermission(ctx context.Context, sessionID, permissionID string, allow bool) error {
	body := permissionAnswer{Response: answerResponse(allow)}
	return c.doJSON(ctx, http.MethodPost, "/session/"+sessionID+"/permissions/"+permissionID, body, nil)
}
