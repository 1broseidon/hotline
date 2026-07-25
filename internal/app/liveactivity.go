package app

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/1broseidon/hotline/internal/config"
)

const (
	liveActivityEventUpdate = "update"
	liveActivityEventEnd    = "end"

	liveActivityJWTLifetime = 50 * time.Minute
	liveActivityHTTPTimeout = 10 * time.Second

	apnsProductionEndpoint = "https://api.push.apple.com"
	apnsSandboxEndpoint    = "https://api.sandbox.push.apple.com"
	apnsLiveActivitySuffix = ".push-type.liveactivity"
	maxLiveActivityJobID   = 64
)

var (
	liveActivityJobIDRE = regexp.MustCompile(`^job-[1-9][0-9]*$`)
	apnsIdentifierRE    = regexp.MustCompile(`^[A-Za-z0-9]{1,64}$`)
	apnsTopicRE         = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9.-]{0,253}[A-Za-z0-9]$`)
	apnsReasonRE        = regexp.MustCompile(`^[A-Za-z]{1,64}$`)
)

// liveActivitySender is optional on Server. Enqueue must synchronously retain
// an immutable request and return without performing network I/O.
type liveActivitySender interface {
	Enqueue(LiveActivityRequest)
}

// LiveActivityRequest is one immutable ActivityKit lifecycle event. Tokens and
// Content are transport-private and must never be logged.
type LiveActivityRequest struct {
	DeviceID  string
	JobID     string
	Token     string
	Event     string
	Timestamp int64
	Content   LiveActivityContent
}

// LiveActivityContent is the exact content-state object sent to ActivityKit.
type LiveActivityContent struct {
	Title     string
	State     string
	Detail    string
	Progress  *float64
	StartedAt int64
}

type liveActivityLane struct {
	queue   []LiveActivityRequest
	running bool
}

// apnsLiveActivitySender owns the APNs provider token cache and keyed lanes.
// A lane serializes one device/job while independent lanes drain concurrently.
type apnsLiveActivitySender struct {
	priv     *ecdsa.PrivateKey
	keyID    string
	teamID   string
	topic    string
	endpoint string
	client   *http.Client
	store    *RelayStore
	clock    func() time.Time

	jwtMu       sync.Mutex
	cachedJWT   string
	jwtIssuedAt time.Time

	lanesMu sync.Mutex
	lanes   map[string]*liveActivityLane
}

// newLiveActivitySender validates the all-or-nothing APNs config. A completely
// absent credential set is a silent disabled state; partial or invalid config
// returns one startup error while leaving every other server path operational.
func newLiveActivitySender(cfg *config.Config, store *RelayStore) (liveActivitySender, error) {
	keyFile := strings.TrimSpace(cfg.APNsKeyFile)
	keyID := strings.TrimSpace(cfg.APNsKeyID)
	teamID := strings.TrimSpace(cfg.APNsTeamID)
	topic := strings.TrimSpace(cfg.APNsTopic)
	configured := 0
	for _, value := range []string{keyFile, keyID, teamID, topic} {
		if value != "" {
			configured++
		}
	}
	if configured == 0 {
		return nil, nil
	}
	if configured != 4 {
		return nil, errors.New("HOTLINE_APNS_KEY_FILE, HOTLINE_APNS_KEY_ID, HOTLINE_APNS_TEAM_ID, and HOTLINE_APNS_TOPIC must be set together")
	}
	if !apnsIdentifierRE.MatchString(keyID) {
		return nil, errors.New("HOTLINE_APNS_KEY_ID is invalid")
	}
	if !apnsIdentifierRE.MatchString(teamID) {
		return nil, errors.New("HOTLINE_APNS_TEAM_ID is invalid")
	}
	if !validAPNsTopic(topic) {
		return nil, errors.New("HOTLINE_APNS_TOPIC must be a base app bundle id")
	}

	environment := strings.TrimSpace(cfg.APNsEnvironment)
	if environment == "" {
		environment = config.DefaultAPNsEnvironment
	}
	endpoint := ""
	switch environment {
	case "production":
		endpoint = apnsProductionEndpoint
	case "sandbox":
		endpoint = apnsSandboxEndpoint
	default:
		return nil, errors.New("HOTLINE_APNS_ENVIRONMENT must be production or sandbox")
	}

	keyData, err := os.ReadFile(keyFile)
	if err != nil {
		return nil, fmt.Errorf("read HOTLINE_APNS_KEY_FILE: %w", err)
	}
	priv, err := parseAPNsPrivateKey(keyData)
	if err != nil {
		return nil, err
	}
	return &apnsLiveActivitySender{
		priv: priv, keyID: keyID, teamID: teamID, topic: topic,
		endpoint: endpoint,
		client:   &http.Client{Timeout: liveActivityHTTPTimeout},
		store:    store,
		clock:    time.Now,
		lanes:    make(map[string]*liveActivityLane),
	}, nil
}

func validAPNsTopic(topic string) bool {
	return len(topic) <= 255 && apnsTopicRE.MatchString(topic) &&
		!strings.Contains(topic, "..") &&
		!strings.HasSuffix(topic, apnsLiveActivitySuffix)
}

// parseAPNsPrivateKey parses Apple's PEM .p8 form: PKCS#8 containing a P-256
// ECDSA private key. Other PEM blocks, key formats, and curves are rejected.
func parseAPNsPrivateKey(data []byte) (*ecdsa.PrivateKey, error) {
	block, rest := pem.Decode(data)
	if block == nil || block.Type != "PRIVATE KEY" || len(bytes.TrimSpace(rest)) != 0 {
		return nil, errors.New("HOTLINE_APNS_KEY_FILE is not a PKCS#8 private key")
	}
	parsed, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, errors.New("HOTLINE_APNS_KEY_FILE is not a valid PKCS#8 private key")
	}
	priv, ok := parsed.(*ecdsa.PrivateKey)
	if !ok || priv.Curve == nil || priv.Curve.Params().Name != elliptic.P256().Params().Name {
		return nil, errors.New("HOTLINE_APNS_KEY_FILE must contain a P-256 EC private key")
	}
	return priv, nil
}

func cloneLiveActivityRequest(req LiveActivityRequest) LiveActivityRequest {
	if req.Content.Progress != nil {
		progress := *req.Content.Progress
		req.Content.Progress = &progress
	}
	return req
}

// Enqueue appends to a device/job lane and starts its asynchronous drain. No
// APNs request runs on the caller's goroutine.
func (s *apnsLiveActivitySender) Enqueue(req LiveActivityRequest) {
	req = cloneLiveActivityRequest(req)
	key := req.DeviceID + "\x00" + req.JobID
	s.lanesMu.Lock()
	if s.lanes == nil {
		s.lanes = make(map[string]*liveActivityLane)
	}
	lane := s.lanes[key]
	if lane == nil {
		lane = &liveActivityLane{}
		s.lanes[key] = lane
	}
	lane.queue = append(lane.queue, req)
	start := !lane.running
	if start {
		lane.running = true
	}
	s.lanesMu.Unlock()
	if start {
		go s.drainLane(key, lane)
	}
}

func (s *apnsLiveActivitySender) drainLane(key string, lane *liveActivityLane) {
	for {
		s.lanesMu.Lock()
		if len(lane.queue) == 0 {
			lane.running = false
			if s.lanes[key] == lane {
				delete(s.lanes, key)
			}
			s.lanesMu.Unlock()
			return
		}
		req := lane.queue[0]
		lane.queue[0] = LiveActivityRequest{}
		lane.queue = lane.queue[1:]
		s.lanesMu.Unlock()

		if err := s.send(req); err != nil {
			// Deliberately omit token, JWT, title, detail, progress, and payload.
			fmt.Fprintf(os.Stderr, "hotline: live activity send failed device=%s job=%s event=%s: %v\n", req.DeviceID, req.JobID, req.Event, err)
		}
	}
}

type liveActivityPayload struct {
	APS liveActivityAPS `json:"aps"`
}

type liveActivityAPS struct {
	Timestamp    int64                    `json:"timestamp"`
	Event        string                   `json:"event"`
	ContentState liveActivityContentState `json:"content-state"`
}

type liveActivityContentState struct {
	Title     string   `json:"title"`
	State     string   `json:"state"`
	Detail    string   `json:"detail"`
	Progress  *float64 `json:"progress"`
	StartedAt int64    `json:"startedAt"`
}

func (s *apnsLiveActivitySender) send(req LiveActivityRequest) error {
	payload := liveActivityPayload{APS: liveActivityAPS{
		Timestamp: req.Timestamp,
		Event:     req.Event,
		ContentState: liveActivityContentState{
			Title: req.Content.Title, State: req.Content.State, Detail: req.Content.Detail,
			Progress: req.Content.Progress, StartedAt: req.Content.StartedAt,
		},
	}}
	body, err := json.Marshal(payload)
	if err != nil {
		return errors.New("encode APNs payload")
	}

	status, reason, err := s.post(req.Token, body, false)
	if err != nil {
		return err
	}
	if status == http.StatusForbidden && reason == "ExpiredProviderToken" {
		status, reason, err = s.post(req.Token, body, true)
		if err != nil {
			return err
		}
	}
	if status == http.StatusOK {
		return nil
	}

	invalidToken := req.Event == liveActivityEventUpdate && (status == http.StatusGone ||
		(status == http.StatusBadRequest && (reason == "BadDeviceToken" || reason == "DeviceTokenNotForTopic")))
	if invalidToken && s.store != nil {
		if _, dropErr := s.store.DropLiveActivityIfToken(req.DeviceID, req.JobID, req.Token); dropErr != nil {
			return errors.New("APNs rejected token and registration cleanup failed")
		}
	}
	return fmt.Errorf("APNs rejected request status=%d reason=%s", status, reason)
}

func (s *apnsLiveActivitySender) post(token string, body []byte, refreshJWT bool) (int, string, error) {
	providerToken, err := s.providerJWT(refreshJWT)
	if err != nil {
		return 0, "", err
	}
	ctx, cancel := context.WithTimeout(context.Background(), liveActivityHTTPTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, s.endpoint+"/3/device/"+url.PathEscape(token), bytes.NewReader(body))
	if err != nil {
		return 0, "", errors.New("build APNs request")
	}
	req.Header.Set("Authorization", "bearer "+providerToken)
	req.Header.Set("apns-topic", s.topic+apnsLiveActivitySuffix)
	req.Header.Set("apns-push-type", "liveactivity")
	req.Header.Set("apns-priority", "10")
	req.Header.Set("apns-expiration", "0")
	req.Header.Set("Content-Type", "application/json")
	client := s.client
	if client == nil {
		client = &http.Client{Timeout: liveActivityHTTPTimeout}
	}
	resp, err := client.Do(req)
	if err != nil {
		// A net/http error includes the request URL (and therefore the token).
		// Return a fixed message so callers can safely log it.
		return 0, "", errors.New("APNs request failed")
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
	return resp.StatusCode, parseAPNsReason(raw), nil
}

func parseAPNsReason(raw []byte) string {
	var body struct {
		Reason string `json:"reason"`
	}
	if json.Unmarshal(raw, &body) == nil && apnsReasonRE.MatchString(body.Reason) {
		return body.Reason
	}
	return "unknown"
}

func (s *apnsLiveActivitySender) providerJWT(force bool) (string, error) {
	s.jwtMu.Lock()
	defer s.jwtMu.Unlock()
	clock := s.clock
	if clock == nil {
		clock = time.Now
	}
	now := clock().UTC()
	if !force && s.cachedJWT != "" && now.Sub(s.jwtIssuedAt) < liveActivityJWTLifetime {
		return s.cachedJWT, nil
	}
	header, _ := json.Marshal(struct {
		Alg string `json:"alg"`
		Kid string `json:"kid"`
	}{Alg: "ES256", Kid: s.keyID})
	claims, _ := json.Marshal(struct {
		Iss string `json:"iss"`
		Iat int64  `json:"iat"`
	}{Iss: s.teamID, Iat: now.Unix()})
	unsigned := base64.RawURLEncoding.EncodeToString(header) + "." + base64.RawURLEncoding.EncodeToString(claims)
	digest := sha256.Sum256([]byte(unsigned))
	r, sigS, err := ecdsa.Sign(rand.Reader, s.priv, digest[:])
	if err != nil {
		return "", errors.New("sign APNs provider token")
	}
	signature := make([]byte, 64)
	r.FillBytes(signature[:32])
	sigS.FillBytes(signature[32:])
	token := unsigned + "." + base64.RawURLEncoding.EncodeToString(signature)
	s.cachedJWT = token
	s.jwtIssuedAt = now
	return token, nil
}

func liveActivityRequest(target LiveActivityTarget, rec *jobRecord, event, state string) LiveActivityRequest {
	var progress *float64
	if rec.progress != nil {
		value := *rec.progress
		progress = &value
	}
	return LiveActivityRequest{
		DeviceID:  target.DeviceID,
		JobID:     target.JobID,
		Token:     target.Token,
		Event:     event,
		Timestamp: time.Now().Unix(),
		Content: LiveActivityContent{
			Title: rec.title, State: state, Detail: rec.detail,
			Progress: progress, StartedAt: rec.startedAt,
		},
	}
}

func (s *Server) enqueueLiveActivities(targets []LiveActivityTarget, rec *jobRecord, event, state string) {
	if s.liveActivitySender == nil {
		return
	}
	for _, target := range targets {
		s.liveActivitySender.Enqueue(liveActivityRequest(target, rec, event, state))
	}
}

// applyLiveActivityToken validates and applies one authenticated post-hello
// control frame. It returns false only for a malformed frame; valid references
// to unknown or terminal jobs are intentionally ignored.
func (s *Server) applyLiveActivityToken(deviceID string, raw []byte) bool {
	var env rawEnvelope
	if json.Unmarshal(raw, &env) != nil {
		return false
	}
	jobRaw, hasJob := env["job_id"]
	tokenRaw, hasToken := env["token"]
	var frame liveActivityTokenFrame
	if !hasJob || !hasToken || json.Unmarshal(jobRaw, &frame.JobID) != nil || json.Unmarshal(tokenRaw, &frame.Token) != nil ||
		len(frame.JobID) > maxLiveActivityJobID || !liveActivityJobIDRE.MatchString(frame.JobID) ||
		(frame.Token != "" && !validAPNsToken(frame.Token)) {
		return false
	}

	reg := s.jobs
	reg.mu.Lock()
	defer reg.mu.Unlock()
	rec, active := reg.jobs[frame.JobID]
	if !active {
		return true
	}
	if frame.Token == "" {
		if err := s.store.RemoveLiveActivity(deviceID, frame.JobID); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: live activity unregister persist failed device=%s job=%s: %v\n", deviceID, frame.JobID, err)
		}
		return true
	}
	if rec.stale {
		return true
	}
	if err := s.store.SetLiveActivity(deviceID, frame.JobID, frame.Token); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: live activity register persist failed device=%s job=%s: %v\n", deviceID, frame.JobID, err)
		return true
	}
	s.enqueueLiveActivities([]LiveActivityTarget{{DeviceID: deviceID, JobID: frame.JobID, Token: frame.Token}}, rec, liveActivityEventUpdate, "running")
	return true
}
