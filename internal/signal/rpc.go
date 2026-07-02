// Package signal adapts the Signal transport to the provider.Provider
// interface. Unlike telegram/discord it holds no credentials: it talks to a
// locally running `signal-cli --account +E164 daemon --http HOST:PORT`
// (a linked secondary device) over two plain HTTP surfaces:
//
//   - POST /api/v1/rpc     — JSON-RPC requests (send, sendReaction,
//     sendTyping, getAttachment, …)
//   - GET  /api/v1/events  — an SSE stream of incoming envelopes
//
// Both endpoints are documented in signal-cli's man/signal-cli-jsonrpc.5.adoc.
package signal

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync/atomic"
	"time"
)

// groupChatPrefix marks a group chat_id. DM chat_ids are the peer's E.164
// number; group chat_ids are "group:" + the base64 groupId from the envelope's
// groupInfo, so the two id spaces can't collide.
const groupChatPrefix = "group:"

// Client is a minimal JSON-RPC-over-HTTP client for the signal-cli daemon.
// The daemon is expected to run in single-account mode (`-a ACCOUNT daemon
// --http`), so requests carry no account param.
type Client struct {
	// BaseURL is the daemon root, e.g. "http://127.0.0.1:8080" (no trailing
	// slash).
	BaseURL string
	// Account is the linked account's E.164 number. Used to recognize our own
	// envelopes and as the default reaction target author.
	Account string
	// HTTP is the underlying client; a 60s-timeout client when nil.
	HTTP *http.Client

	reqID atomic.Int64
}

// NewClient builds a Client for the daemon at baseURL.
func NewClient(baseURL, account string) *Client {
	return &Client{
		BaseURL: strings.TrimRight(baseURL, "/"),
		Account: account,
		HTTP:    &http.Client{Timeout: 60 * time.Second},
	}
}

// rpcError is a JSON-RPC error object.
type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

// Call posts one JSON-RPC request to /api/v1/rpc and returns the raw result.
func (c *Client) Call(ctx context.Context, method string, params map[string]any) (json.RawMessage, error) {
	req := map[string]any{
		"jsonrpc": "2.0",
		"id":      fmt.Sprintf("hotline-%d", c.reqID.Add(1)),
		"method":  method,
	}
	if params != nil {
		req["params"] = params
	}
	body, err := json.Marshal(req)
	if err != nil {
		return nil, err
	}

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, c.BaseURL+"/api/v1/rpc", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	httpReq.Header.Set("Content-Type", "application/json")

	client := c.HTTP
	if client == nil {
		client = &http.Client{Timeout: 60 * time.Second}
	}
	resp, err := client.Do(httpReq)
	if err != nil {
		return nil, fmt.Errorf("signal daemon unreachable: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		return nil, fmt.Errorf("signal daemon: HTTP %d", resp.StatusCode)
	}

	var out struct {
		Result json.RawMessage `json:"result"`
		Error  *rpcError       `json:"error"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 100<<20)).Decode(&out); err != nil {
		return nil, fmt.Errorf("decoding signal daemon response: %w", err)
	}
	if out.Error != nil {
		return nil, fmt.Errorf("signal daemon: %s (code %d)", out.Error.Message, out.Error.Code)
	}
	return out.Result, nil
}

// targetParams translates a hotline chat_id into signal-cli addressing:
// "group:<base64>" becomes groupId, anything else a single recipient.
func targetParams(chatID string) map[string]any {
	if g, ok := strings.CutPrefix(chatID, groupChatPrefix); ok {
		return map[string]any{"groupId": g}
	}
	return map[string]any{"recipient": []string{chatID}}
}

// Send sends a message (with optional local-path attachments); when
// editTimestamp is non-zero it edits the previous message with that timestamp
// instead (signal-cli's send --edit-timestamp). Returns the new message's
// timestamp — Signal's message identity.
func (c *Client) Send(ctx context.Context, chatID, message string, attachments []string, editTimestamp int64) (int64, error) {
	params := targetParams(chatID)
	if message != "" {
		params["message"] = message
	}
	if len(attachments) > 0 {
		params["attachments"] = attachments
	}
	if editTimestamp != 0 {
		params["editTimestamp"] = editTimestamp
	}
	raw, err := c.Call(ctx, "send", params)
	if err != nil {
		return 0, err
	}
	var res struct {
		Timestamp int64 `json:"timestamp"`
	}
	if err := json.Unmarshal(raw, &res); err != nil {
		return 0, fmt.Errorf("decoding send result: %w", err)
	}
	return res.Timestamp, nil
}

// SendTyping triggers a typing indicator for the chat (shown ~15s or until
// the next message).
func (c *Client) SendTyping(ctx context.Context, chatID string) error {
	_, err := c.Call(ctx, "sendTyping", targetParams(chatID))
	return err
}

// SendReaction reacts to the message sent by targetAuthor at targetTimestamp.
func (c *Client) SendReaction(ctx context.Context, chatID, emoji, targetAuthor string, targetTimestamp int64) error {
	params := targetParams(chatID)
	params["emoji"] = emoji
	params["targetAuthor"] = targetAuthor
	params["targetTimestamp"] = targetTimestamp
	_, err := c.Call(ctx, "sendReaction", params)
	return err
}

// GetAttachment fetches an attachment's raw bytes via the daemon's
// getAttachment command (returned base64-encoded per the signal-cli docs).
func (c *Client) GetAttachment(ctx context.Context, chatID, id string) ([]byte, error) {
	params := targetParams(chatID)
	params["id"] = id
	raw, err := c.Call(ctx, "getAttachment", params)
	if err != nil {
		return nil, err
	}
	var b64 string
	if err := json.Unmarshal(raw, &b64); err != nil {
		// Some signal-cli versions wrap the payload in an object.
		var obj struct {
			Data string `json:"data"`
		}
		if err2 := json.Unmarshal(raw, &obj); err2 != nil || obj.Data == "" {
			return nil, fmt.Errorf("decoding getAttachment result: %w", err)
		}
		b64 = obj.Data
	}
	return base64.StdEncoding.DecodeString(b64)
}
