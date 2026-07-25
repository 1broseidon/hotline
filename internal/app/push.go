package app

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"
)

const (
	expoPushEndpoint = "https://exp.host/--/api/v2/push/send"
	pushBodyMax      = 140
	// No narrower provider title limit is evidenced in this codebase, so custom
	// notification titles use the existing conservative 140-rune push bound.
	pushTitleMax = 140
	pushTimeout  = 10 * time.Second
	// maxAPNsTokenLen matches the gateway's MAX_TOKEN_LEN (PROTOCOL.md §1.2 /
	// index.ts validateToken).
	maxAPNsTokenLen = 200

	// wakeMaxPerMinute / wakeMinIntervalSecs mirror the core's wake throttle
	// (hotline-core index.ts reserveTick: WAKE_MAX_PER_MINUTE=6, 10s min gap).
	// The box pre-gates wake attempts to this same contract so it never burns
	// its shared per-IP CONTROL_PLANE budget on a wake the core would reject
	// (round-2 review S-2, box side).
	wakeMaxPerMinute    = 6
	wakeMinIntervalSecs = 10
)

// wakeGate is the box-side per-device wake pre-throttle. It mirrors the core's
// wake contract exactly (max 6 per rolling 60s AND at least 10s apart), so a
// burst of durable items on an away device can't fire more wake POSTs than the
// core would accept — protecting the shared per-IP budget every bot on the box
// draws from. Concurrency-safe: maybePush fires from many goroutines.
type wakeGate struct {
	mu     sync.Mutex
	times  map[string][]int64 // deviceID -> allowed-wake unix seconds in the last 60s
	logged map[string]int64   // deviceID -> unix sec of the last suppression log
	clock  func() time.Time
}

func newWakeGate() *wakeGate {
	return &wakeGate{times: map[string][]int64{}, logged: map[string]int64{}, clock: time.Now}
}

// allow reserves a wake tick for deviceID when the core's contract would accept
// it. It returns ok=false (reserving nothing) when the wake is locally
// throttled; logNow is true at most once per rolling 60s window per device, so
// the caller can emit a single debug line for a suppressed burst rather than one
// per dropped item.
func (g *wakeGate) allow(deviceID string) (ok, logNow bool) {
	g.mu.Lock()
	defer g.mu.Unlock()
	nowSec := g.clock().Unix()
	cutoff := nowSec - 60

	kept := g.times[deviceID][:0]
	for _, t := range g.times[deviceID] {
		if t > cutoff {
			kept = append(kept, t)
		}
	}

	tooSoon := len(kept) > 0 && nowSec-kept[len(kept)-1] < wakeMinIntervalSecs
	if tooSoon || len(kept) >= wakeMaxPerMinute {
		g.times[deviceID] = kept
		if g.logged[deviceID] <= cutoff { // once per 60s window
			g.logged[deviceID] = nowSec
			logNow = true
		}
		return false, logNow
	}
	g.times[deviceID] = append(kept, nowSec)
	return true, false
}

var (
	expoTokenRe = regexp.MustCompile(`^(ExponentPushToken|ExpoPushToken)\[[A-Za-z0-9._:+/=-]+\]$`)
	// apnsHexTokenRe matches the gateway's iOS token shape: lowercase hex,
	// opaque length (index.ts IOS_TOKEN_RE). Even length and the length cap are
	// enforced separately by validAPNsToken.
	apnsHexTokenRe = regexp.MustCompile(`^[0-9a-f]+$`)
)

// validAPNsToken reports whether tok is a well-formed APNs device token as the
// gateway validates it: lowercase hex, even length, 1..200 chars.
func validAPNsToken(tok string) bool {
	return apnsHexTokenRe.MatchString(tok) && len(tok)%2 == 0 && len(tok) <= maxAPNsTokenLen
}

// acceptPushToken reports whether an inbound hello.push token is well-formed for
// the active push mode: hex APNs tokens in gateway mode, Expo tokens otherwise.
func (s *Server) acceptPushToken(tok string) bool {
	if s.gatewayEnabled() {
		return validAPNsToken(tok)
	}
	return expoTokenRe.MatchString(tok)
}

// gatewayEnabled reports whether the signed push-gateway path is active. It is
// gated on HOTLINE_PUSH_ENDPOINT being set (which constructs the signer): unset
// ⇒ the legacy Expo path below runs unchanged. In core mode the gateway is
// forced off: HOTLINE_PUSH_ENDPOINT is ignored (SPEC §5), so token acceptance
// validates the app's Expo shape and any B1 legacy fallback uses Expo-direct —
// exactly as if the endpoint were unset.
func (s *Server) gatewayEnabled() bool { return s.pushSigner != nil && !s.coreMode }

// roomRegistered reports whether the given room id was successfully registered
// with the core (ensureRegistered's success state). Only core-registered rooms
// (envelope rooms with a stored secret) can take the wake path; a plaintext /
// unregistered room in core mode would 404 on wake, so maybePush falls back to
// the legacy Expo path for it (B1).
func (s *Server) roomRegistered(room string) bool {
	s.regMu.Lock()
	defer s.regMu.Unlock()
	return s.registered[room]
}

// pushIntent is an internal, enqueue-scoped notification override. It never
// rides the durable frame or mailbox disk; only a fresh successful insertion on
// an away device can hand it to the transport.
type pushIntent struct {
	Title string
	Body  string
}

// boundedNotification trims and rune-safely bounds custom provider-visible
// notification text. Generic message notifications bypass this helper and keep
// their existing behavior byte-for-byte.
func boundedNotification(intent pushIntent) pushIntent {
	return pushIntent{
		Title: truncate(strings.TrimSpace(intent.Title), pushTitleMax),
		Body:  truncate(strings.TrimSpace(intent.Body), pushBodyMax),
	}
}

func (s *Server) maybePush(deviceID string, item MailboxItem) {
	s.routePush(deviceID, item, nil)
}

func (s *Server) maybePushWithIntent(deviceID string, item MailboxItem, intent pushIntent) {
	bounded := boundedNotification(intent)
	s.routePush(deviceID, item, &bounded)
}

// routePush is the shared token/room/transport router for generic item pushes
// and custom FB44 completion pushes. A nil intent is the historical generic
// path; keeping all existing choices in this function prevents transport drift.
func (s *Server) routePush(deviceID string, item MailboxItem, intent *pushIntent) {
	// Read the token, key_id, and the current room in one locked snapshot so a
	// concurrent link/room rotation can never pair a token bound to an old room
	// with a newly rotated room label. The room (== the app's chat id) rides
	// along so the app's foreground handler can tell whether the message belongs
	// to the chat already on screen and suppress the in-app banner only in that
	// case.
	token, keyID, room, ok := s.store.ActivePushTarget(deviceID)
	if !ok {
		return
	}
	// Core mode (core-v1 SPEC §3): the push DECISION already happened at the
	// mailbox's durable insertion boundary. Registered rooms always keep the
	// existing signed wake transport; a custom intent never bypasses core and
	// never adds fields to its wake contract.
	//
	// B1: only core-REGISTERED rooms can be woken. A plaintext / secretless room
	// (including any minted before core mode was flipped on) is never registered,
	// so its wake would 404 in the core. For those rooms we fall through to the
	// legacy Expo path below — with the gateway forced off in core mode, that is
	// Expo-direct, the pre-core behavior.
	if s.coreMode && s.coreClient != nil && s.roomRegistered(room) {
		// Box-side wake pre-gate (S-2): generic and custom wakes share the core's
		// 6/min+10s contract and the box's existing local reservation.
		if s.wakeGate != nil {
			if ok, logNow := s.wakeGate.allow(deviceID); !ok {
				if logNow {
					fmt.Fprintf(os.Stderr, "hotline: wake pre-gated locally (core 6/min+10s) device=%s\n", deviceID)
				}
				return
			}
		}
		// Per-device FB23 preview precedence is unchanged. When clear preview is
		// effective, a custom completion contributes only its bounded body; the
		// registered room's core-owned name remains the notification title. With
		// previews disabled the body remains core's generic message text, and the
		// empty-preview wake stays wire-identical.
		effClear := s.pushPreviewClear
		if d, ok := s.store.Device(deviceID); ok && d.PushPreviewClear != nil {
			effClear = *d.PushPreviewClear
		}
		var preview string
		if effClear {
			if intent != nil {
				preview = intent.Body
			} else {
				preview = previewText(item.Payload)
			}
		}
		go s.sendWakeHint(deviceID, room, preview)
		return
	}
	title := nonBlank(s.getBotName(), "Hotline")
	body := pushPreview(item.Payload)
	if intent != nil {
		title, body = intent.Title, intent.Body
	}
	if s.gatewayEnabled() {
		if !validAPNsToken(token) {
			return
		}
		if keyID == "" {
			// The token has no gateway credential yet: kick registration and drop
			// this push. The next message after complete() carries the key_id.
			s.kickRegister(deviceID, token)
			return
		}
		go s.sendGatewayPush(deviceID, token, keyID, title, body, room)
		return
	}
	// Legacy Expo path (HOTLINE_PUSH_ENDPOINT unset), unchanged for generic
	// pushes and also used by unregistered rooms while core mode is enabled.
	if !expoTokenRe.MatchString(token) {
		return
	}
	go s.sendPush(token, title, body, room)
}

// forwardDeviceToken pushes a device's hello.push token up to the core registry
// in the background (core-v1 SPEC §3.3). Best-effort: a failure is logged and
// retried on the device's next hello.
func (s *Server) forwardDeviceToken(room, deviceID, platform, token string) {
	go func() {
		ctx, cancel := withTimeout(context.Background(), pushTimeout)
		defer cancel()
		if err := s.coreClient.putDevice(ctx, room, deviceID, platform, token); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: core put-device failed device=%s: %v\n", deviceID, err)
		}
	}()
}

// sendWakeHint posts the signed wake hint to the core for an away device
// (core-v1 SPEC §3.4). Throttling and token resolution live in the core; the box
// only reacts to a terminal token_invalid by dropping its local copy so it stops
// forwarding a dead token. Throttled / no_token replies are success, not errors.
func (s *Server) sendWakeHint(deviceID, room, preview string) {
	ctx, cancel := withTimeout(context.Background(), pushTimeout)
	defer cancel()
	res, err := s.coreClient.wake(ctx, room, deviceID, preview)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: core wake failed device=%s: %v\n", deviceID, err)
		return
	}
	if res.TokenInvalid {
		if derr := s.store.DropPushToken(deviceID); derr != nil {
			fmt.Fprintf(os.Stderr, "hotline: core wake drop-token failed device=%s: %v\n", deviceID, derr)
		}
	}
}

// kickRegister launches a background registration for a device whose token has
// no gateway credential yet. It is a no-op if the signer is absent or a
// registration for this device is already in flight.
func (s *Server) kickRegister(deviceID, token string) {
	if s.pushSigner == nil || token == "" {
		return
	}
	go func() {
		if err := s.pushSigner.register(context.Background(), deviceID, token); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: app push registration failed device=%s: %v\n", deviceID, err)
		}
	}()
}

// pushPreview derives the push notification body from a message payload: the
// text, as always. Element-only messages need no special casing because the
// box synthesizes their text from the elements' fallbacks (ERRATA E6/E10).
func pushPreview(payload []byte) string {
	if t := previewText(payload); t != "" {
		return t
	}
	return "New message"
}

// previewText extracts the message's plaintext body from a payload, trimmed and
// rune-safe-truncated to pushBodyMax. Returns "" when there is no text (an
// element-only message's text is synthesized upstream, ERRATA E6/E10). This is
// the exact snippet a clear-preview wake hint carries (SPEC core-v1 §3.4
// preview extension) — the same content the legacy Expo path previews.
func previewText(payload []byte) string {
	var p struct {
		Text string `json:"text"`
	}
	_ = json.Unmarshal(payload, &p)
	if t := strings.TrimSpace(p.Text); t != "" {
		return truncate(t, pushBodyMax)
	}
	return ""
}

func (s *Server) sendPush(to, title, body, room string) {
	pushData := map[string]string{"url": "hotline://chat"}
	if room != "" {
		pushData["room"] = room
	}
	payload := map[string]any{
		"to": to, "title": title, "body": body,
		"data":  pushData,
		"sound": "default", "priority": "high",
		// mutable-content invokes the app's Notification Service Extension
		// (communication notifications, FB56); without it iOS never runs the NSE.
		"mutableContent": true,
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return
	}
	endpoint := s.pushEndpoint
	if endpoint == "" {
		endpoint = expoPushEndpoint
	}
	req, err := http.NewRequest(http.MethodPost, endpoint, bytes.NewReader(data))
	if err != nil {
		return
	}
	req.Header.Set("Content-Type", "application/json")
	if s.pushBearer != "" {
		req.Header.Set("Authorization", "Bearer "+s.pushBearer)
	}
	client := s.pushClient
	if client == nil {
		client = &http.Client{Timeout: pushTimeout}
	}
	resp, err := client.Do(req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app push send failed: %v\n", err)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 256))
		fmt.Fprintf(os.Stderr, "hotline: app push rejected (%d): %s\n", resp.StatusCode, strings.TrimSpace(string(snippet)))
	}
}

// sendGatewayPush delivers one message through the signed push gateway
// (PROTOCOL.md §1). The body keeps the same data.url/data.room the app relies on
// for tap-routing. Terminal token rejection (410 / token_invalid / drop_token)
// permanently drops the token; a 401 unknown_registration re-registers; 429/503
// are transient and simply retried on the next message.
func (s *Server) sendGatewayPush(deviceID, token, keyID, title, body, room string) {
	pushData := map[string]string{"url": "hotline://chat"}
	if room != "" {
		pushData["room"] = room
	}
	payload := map[string]any{
		"platform": "ios",
		"token":    token,
		"title":    title,
		"body":     body,
		"data":     pushData,
	}
	data, err := json.Marshal(payload)
	if err != nil {
		return
	}
	const path = "/v1/push"
	h, err := s.pushSigner.signCanonical(http.MethodPost, path, data)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app push sign failed: %v\n", err)
		return
	}
	req, err := http.NewRequest(http.MethodPost, s.pushSigner.endpoint+path, bytes.NewReader(data))
	if err != nil {
		return
	}
	req.Header.Set("Content-Type", "application/json")
	applySignedHeaders(req, h)
	req.Header.Set("X-Hotline-Key-Id", keyID)
	client := s.pushClient
	if client == nil {
		client = &http.Client{Timeout: pushTimeout}
	}
	resp, err := client.Do(req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app push send failed: %v\n", err)
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusOK {
		return
	}
	snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 256))
	code, disposition := pushRejectFields(snippet)
	switch {
	case resp.StatusCode == http.StatusGone || code == "token_invalid" || disposition == "drop_token":
		// APNs terminally rejected the token: remove it and stop pushing.
		if err := s.store.DropPushToken(deviceID); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: app push drop-token failed device=%s: %v\n", deviceID, err)
		}
	case resp.StatusCode == http.StatusUnauthorized || code == "unknown_registration":
		// The key_id is unknown/revoked: re-register from scratch.
		if err := s.store.DropPushToken(deviceID); err == nil {
			_ = s.store.SetPush(deviceID, token, "ios")
			s.kickRegister(deviceID, token)
		}
	default:
		// 429/503 and anything else: transient, retried on the next message.
		fmt.Fprintf(os.Stderr, "hotline: app push rejected (%d): %s\n", resp.StatusCode, strings.TrimSpace(string(snippet)))
	}
}

// pushRejectFields extracts the gateway's error code and push disposition from
// a rejection body for drop/retry routing (index.ts pushResponse).
func pushRejectFields(snippet []byte) (code, disposition string) {
	var r struct {
		Code        string `json:"code"`
		Disposition string `json:"disposition"`
	}
	_ = json.Unmarshal(snippet, &r)
	return r.Code, r.Disposition
}
