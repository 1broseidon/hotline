package app

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// coreClient is the box's signed HTTP client for the hotline-core control plane
// (core-v1 SPEC §2): room registration, device-token forwarding, wake hints, and
// push-test. Every request is signed with the box identity key (box-key.json)
// using the shared signRequest scheme. It exists only in core mode; unset
// HOTLINE_CORE_MODE never constructs one.
type coreClient struct {
	priv    *ecdsa.PrivateKey
	baseURL string // e.g. https://relay.hotline.dev (no trailing slash)
	client  *http.Client
}

// newCoreClient loads/creates box-key.json and binds a client to the core base
// URL. Core-mode base URLs are https:// (soak points it at the workers.dev URL).
func newCoreClient(stateDir, baseURL string, client *http.Client) (*coreClient, error) {
	priv, err := loadOrCreateBoxKey(stateDir)
	if err != nil {
		return nil, err
	}
	if client == nil {
		client = &http.Client{Timeout: pushTimeout}
	}
	return &coreClient{priv: priv, baseURL: strings.TrimRight(baseURL, "/"), client: client}, nil
}

// wakeResult is the core's reply to a wake / push-test hint (SPEC §2.4).
type wakeResult struct {
	Pushed       bool
	Reason       string // "throttled" | "no_token" (when Pushed == false)
	TokenInvalid bool   // 410 token_invalid: the box must drop its local token
}

// doSigned signs and sends one control-plane request. path is the URL path only
// (the signed canonical string binds the path, not host/query). It returns the
// status code and the (capped) response body.
func (c *coreClient) doSigned(ctx context.Context, method, path string, body []byte) (int, []byte, error) {
	h, err := signRequest(c.priv, method, path, body)
	if err != nil {
		return 0, nil, err
	}
	var rdr io.Reader
	if len(body) > 0 {
		rdr = bytes.NewReader(body)
	}
	req, err := http.NewRequestWithContext(ctx, method, c.baseURL+path, rdr)
	if err != nil {
		return 0, nil, err
	}
	// Content-Type is required by the core on request bodies; a signed empty-body
	// DELETE sends none (the canonical body hash is SHA-256 of zero bytes).
	if len(body) > 0 {
		req.Header.Set("Content-Type", "application/json")
	}
	applySignedHeaders(req, h)
	resp, err := c.client.Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
	return resp.StatusCode, snippet, nil
}

func roomPath(room, suffix string) string { return "/v1/rooms/" + room + suffix }

// register performs the signed, idempotent, TOFU room registration (SPEC §2.2).
func (c *coreClient) register(ctx context.Context, room, name, authHash string) error {
	body, err := json.Marshal(map[string]any{
		"box_pub":   publicJWKFor(c.priv),
		"name":      name,
		"auth_hash": authHash,
		"proto":     "core-v1",
	})
	if err != nil {
		return err
	}
	status, snippet, err := c.doSigned(ctx, http.MethodPost, roomPath(room, "/register"), body)
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("register rejected (%d): %s", status, readCode(bytes.NewReader(snippet)))
	}
	return nil
}

// putDevice forwards a device's push tokens to the core registry (SPEC §2.3,
// full replace). tonight's hello.push carries only the expo token.
func (c *coreClient) putDevice(ctx context.Context, room, deviceID, platform, expoToken string) error {
	body, err := json.Marshal(map[string]any{
		"platform": platform,
		"tokens":   map[string]string{"expo": expoToken},
	})
	if err != nil {
		return err
	}
	status, snippet, err := c.doSigned(ctx, http.MethodPut, roomPath(room, "/devices/"+deviceID), body)
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("put device rejected (%d): %s", status, readCode(bytes.NewReader(snippet)))
	}
	return nil
}

// deleteDevice removes a device from the core registry (SPEC §2.3, idempotent,
// signed empty body). Called on `hotline relay revoke`.
func (c *coreClient) deleteDevice(ctx context.Context, room, deviceID string) error {
	status, snippet, err := c.doSigned(ctx, http.MethodDelete, roomPath(room, "/devices/"+deviceID), nil)
	if err != nil {
		return err
	}
	if status != http.StatusOK {
		return fmt.Errorf("delete device rejected (%d): %s", status, readCode(bytes.NewReader(snippet)))
	}
	return nil
}

// wake sends the content-free wake hint (SPEC §2.4). The push DECISION already
// happened box-side (presence lease / pushEligible); this only swaps the
// transport of "go wake them" from Expo-direct to the core.
func (c *coreClient) wake(ctx context.Context, room, deviceID, preview string) (wakeResult, error) {
	return c.wakeLike(ctx, roomPath(room, "/wake"), deviceID, preview)
}

// pushTest backs `hotline relay push-test` (SPEC §2.5): same pipeline, own cap.
// A push-test never carries a preview (it's a fixed-body probe).
func (c *coreClient) pushTest(ctx context.Context, room, deviceID string) (wakeResult, error) {
	return c.wakeLike(ctx, roomPath(room, "/push-test"), deviceID, "")
}

// wakeLike builds and sends a wake/push-test hint. When preview is non-empty
// (HOTLINE_PUSH_PREVIEW=clear), it rides in the body as "preview" — the core
// uses it as the notification body. It is mutually exclusive with the reserved
// "preview_c" (always null here); the preview is covered by the request
// signature because the canonical string hashes the exact body bytes. An empty
// preview omits the field, keeping the hint byte-identical to the generic form.
func (c *coreClient) wakeLike(ctx context.Context, path, deviceID, preview string) (wakeResult, error) {
	payload := map[string]any{
		"device_id": deviceID,
		"kind":      "message",
		"preview_c": nil,
	}
	if preview != "" {
		payload["preview"] = preview
	}
	body, err := json.Marshal(payload)
	if err != nil {
		return wakeResult{}, err
	}
	status, snippet, err := c.doSigned(ctx, http.MethodPost, path, body)
	if err != nil {
		return wakeResult{}, err
	}
	switch status {
	case http.StatusOK:
		var r struct {
			Pushed bool   `json:"pushed"`
			Reason string `json:"reason"`
		}
		_ = json.Unmarshal(snippet, &r)
		return wakeResult{Pushed: r.Pushed, Reason: r.Reason}, nil
	case http.StatusGone:
		// token_invalid: the core deleted its record AND reports it so the box
		// drops its local copy too.
		return wakeResult{TokenInvalid: true}, nil
	default:
		// 429/503 and unknown codes: transient, retried on the next hint (the box
		// treats unknown codes as retryable-once, SPEC §2.6).
		return wakeResult{}, fmt.Errorf("wake rejected (%d): %s", status, readCode(bytes.NewReader(snippet)))
	}
}

// withTimeout is a small helper for the CLI (revoke/push-test) paths that need a
// bounded context.
func withTimeout(parent context.Context, d time.Duration) (context.Context, context.CancelFunc) {
	return context.WithTimeout(parent, d)
}

// PushTestViaCore is a CLI helper (`hotline relay push-test`): it builds a
// bounded core client from the box key and sends a push-test hint (SPEC §2.5).
// It returns whether a push was sent and any reason the core reported.
func PushTestViaCore(stateDir, baseURL, room, deviceID string) (pushed bool, reason string, err error) {
	cc, err := newCoreClient(stateDir, baseURL, nil)
	if err != nil {
		return false, "", err
	}
	ctx, cancel := withTimeout(context.Background(), pushTimeout)
	defer cancel()
	res, err := cc.pushTest(ctx, room, deviceID)
	if err != nil {
		return false, "", err
	}
	if res.TokenInvalid {
		return false, "token_invalid", nil
	}
	return res.Pushed, res.Reason, nil
}

// DeleteDeviceViaCore is a CLI helper (`hotline relay revoke` in core mode): it
// removes a device from the core registry (SPEC §2.3, idempotent).
func DeleteDeviceViaCore(stateDir, baseURL, room, deviceID string) error {
	cc, err := newCoreClient(stateDir, baseURL, nil)
	if err != nil {
		return err
	}
	ctx, cancel := withTimeout(context.Background(), pushTimeout)
	defer cancel()
	return cc.deleteDevice(ctx, room, deviceID)
}
