package app

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
)

func writeAPNsTestKey(t *testing.T) (string, *ecdsa.PrivateKey) {
	t.Helper()
	priv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalPKCS8PrivateKey(priv)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "AuthKey_TEST.p8")
	if err := os.WriteFile(path, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}), 0o600); err != nil {
		t.Fatal(err)
	}
	return path, priv
}

func directAPNSSender(priv *ecdsa.PrivateKey, endpoint string, client *http.Client, store *RelayStore) *apnsLiveActivitySender {
	return &apnsLiveActivitySender{
		priv: priv, keyID: "KEYID12345", teamID: "TEAMID1234", topic: "dev.hotline.app",
		endpoint: endpoint, client: client, store: store, clock: time.Now,
		lanes: make(map[string]*liveActivityLane),
	}
}

func captureNewServerStderr(t *testing.T, cfg *config.Config) (*Server, string) {
	t.Helper()
	read, write, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	old := os.Stderr
	os.Stderr = write
	srv := NewServer(cfg, nil)
	_ = write.Close()
	os.Stderr = old
	output, err := io.ReadAll(read)
	_ = read.Close()
	if err != nil {
		t.Fatal(err)
	}
	if srv.outbox != nil {
		t.Cleanup(srv.outbox.close)
	}
	return srv, string(output)
}

func TestLiveActivityConfigAbsentPartialAndEnvironment(t *testing.T) {
	store, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if sender, err := newLiveActivitySender(&config.Config{}, store); err != nil || sender != nil {
		t.Fatalf("absent config: sender=%T err=%v", sender, err)
	}
	if sender, err := newLiveActivitySender(&config.Config{APNsKeyID: "KEYID12345"}, store); err == nil || sender != nil {
		t.Fatalf("partial config: sender=%T err=%v", sender, err)
	}

	absentServer, absentLog := captureNewServerStderr(t, testConfig(t))
	if absentServer.initErr != nil || absentServer.liveActivitySender != nil || absentLog != "" {
		t.Fatalf("absent server config: initErr=%v sender=%T log=%q", absentServer.initErr, absentServer.liveActivitySender, absentLog)
	}
	partialCfg := testConfig(t)
	partialCfg.APNsKeyID = "KEYID12345"
	partialServer, partialLog := captureNewServerStderr(t, partialCfg)
	if partialServer.initErr != nil || partialServer.jobs == nil || partialServer.liveActivitySender != nil ||
		strings.Count(partialLog, "live activities disabled") != 1 || strings.Count(strings.TrimSpace(partialLog), "\n") != 0 {
		t.Fatalf("partial server config: initErr=%v sender=%T log=%q", partialServer.initErr, partialServer.liveActivitySender, partialLog)
	}

	keyFile, _ := writeAPNsTestKey(t)
	base := config.Config{
		APNsKeyFile: keyFile, APNsKeyID: "KEYID12345", APNsTeamID: "TEAMID1234", APNsTopic: "dev.hotline.app",
	}
	production, err := newLiveActivitySender(&base, store)
	if err != nil {
		t.Fatal(err)
	}
	prod := production.(*apnsLiveActivitySender)
	if prod.endpoint != apnsProductionEndpoint || prod.client.Timeout != liveActivityHTTPTimeout {
		t.Fatalf("production sender endpoint=%q timeout=%v", prod.endpoint, prod.client.Timeout)
	}
	base.APNsEnvironment = "sandbox"
	sandbox, err := newLiveActivitySender(&base, store)
	if err != nil {
		t.Fatal(err)
	}
	if got := sandbox.(*apnsLiveActivitySender).endpoint; got != apnsSandboxEndpoint {
		t.Fatalf("sandbox endpoint=%q", got)
	}
	base.APNsEnvironment = "staging"
	if sender, err := newLiveActivitySender(&base, store); err == nil || sender != nil {
		t.Fatalf("invalid environment: sender=%T err=%v", sender, err)
	}
}

func TestLiveActivityAPNsPayloadHeadersAndJWT(t *testing.T) {
	_, priv := writeAPNsTestKey(t)
	type captured struct {
		path    string
		headers http.Header
		body    string
	}
	got := make(chan captured, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		got <- captured{path: r.URL.Path, headers: r.Header.Clone(), body: string(body)}
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()
	sender := directAPNSSender(priv, server.URL, server.Client(), nil)
	sender.clock = func() time.Time { return time.Unix(1_800_000_000, 0) }

	progress := 0.5
	requests := []LiveActivityRequest{
		{
			DeviceID: "dev-aaaaaa111111", JobID: "job-1", Token: "aabb", Event: liveActivityEventUpdate, Timestamp: 1_800_000_001,
			Content: LiveActivityContent{Title: "Header pass", State: "running", Detail: "Compiling", Progress: nil, StartedAt: 1_799_999_900},
		},
		{
			DeviceID: "dev-aaaaaa111111", JobID: "job-1", Token: "aabb", Event: liveActivityEventEnd, Timestamp: 1_800_000_002,
			Content: LiveActivityContent{Title: "Header pass", State: "ok", Detail: "Shipped", Progress: &progress, StartedAt: 1_799_999_900},
		},
	}
	for _, req := range requests {
		if err := sender.send(req); err != nil {
			t.Fatal(err)
		}
	}
	captures := []captured{<-got, <-got}
	wantBodies := []string{
		`{"aps":{"timestamp":1800000001,"event":"update","content-state":{"title":"Header pass","state":"running","detail":"Compiling","progress":null,"startedAt":1799999900}}}`,
		`{"aps":{"timestamp":1800000002,"event":"end","content-state":{"title":"Header pass","state":"ok","detail":"Shipped","progress":0.5,"startedAt":1799999900}}}`,
	}
	for i, capture := range captures {
		if capture.path != "/3/device/aabb" || capture.body != wantBodies[i] {
			t.Fatalf("request %d path=%q body=%s", i, capture.path, capture.body)
		}
		for header, want := range map[string]string{
			"apns-topic": "dev.hotline.app.push-type.liveactivity", "apns-push-type": "liveactivity",
			"apns-priority": "10", "apns-expiration": "0", "Content-Type": "application/json",
		} {
			if got := capture.headers.Get(header); got != want {
				t.Errorf("request %d %s=%q, want %q", i, header, got, want)
			}
		}
		if !strings.HasPrefix(capture.headers.Get("Authorization"), "bearer ") {
			t.Fatalf("request %d authorization missing bearer token", i)
		}
	}

	jwt := strings.TrimPrefix(captures[0].headers.Get("Authorization"), "bearer ")
	parts := strings.Split(jwt, ".")
	if len(parts) != 3 {
		t.Fatalf("JWT has %d parts", len(parts))
	}
	var header map[string]any
	headerJSON, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil || json.Unmarshal(headerJSON, &header) != nil || header["alg"] != "ES256" || header["kid"] != "KEYID12345" {
		t.Fatalf("JWT header=%s err=%v", headerJSON, err)
	}
	var claims map[string]any
	claimsJSON, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || json.Unmarshal(claimsJSON, &claims) != nil || claims["iss"] != "TEAMID1234" || claims["iat"] != float64(1_800_000_000) {
		t.Fatalf("JWT claims=%s err=%v", claimsJSON, err)
	}
	signature, err := base64.RawURLEncoding.DecodeString(parts[2])
	if err != nil || len(signature) != 64 {
		t.Fatalf("JWT signature len=%d err=%v", len(signature), err)
	}
	digest := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
	if !ecdsa.Verify(&priv.PublicKey, digest[:], new(big.Int).SetBytes(signature[:32]), new(big.Int).SetBytes(signature[32:])) {
		t.Fatal("JWT JOSE signature did not verify")
	}
	if captures[0].headers.Get("Authorization") != captures[1].headers.Get("Authorization") {
		t.Fatal("provider JWT was not reused inside the 50 minute cache")
	}
}

func TestLiveActivityAPNsExpiredProviderTokenRetriesOnce(t *testing.T) {
	_, priv := writeAPNsTestKey(t)
	var mu sync.Mutex
	var auth []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		auth = append(auth, r.Header.Get("Authorization"))
		attempt := len(auth)
		mu.Unlock()
		if attempt == 1 {
			w.WriteHeader(http.StatusForbidden)
			_, _ = w.Write([]byte(`{"reason":"ExpiredProviderToken"}`))
			return
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()
	sender := directAPNSSender(priv, server.URL, server.Client(), nil)
	req := LiveActivityRequest{Token: "aabb", Event: liveActivityEventUpdate, Timestamp: 1, Content: LiveActivityContent{State: "running"}}
	if err := sender.send(req); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(auth) != 2 || auth[0] == auth[1] {
		t.Fatalf("authorization attempts=%v", auth)
	}
}

func TestLiveActivityAPNsInvalidTokenConditionalCleanup(t *testing.T) {
	_, priv := writeAPNsTestKey(t)
	for _, tc := range []struct {
		name   string
		status int
		reason string
	}{
		{name: "gone", status: http.StatusGone, reason: "Unregistered"},
		{name: "bad device token", status: http.StatusBadRequest, reason: "BadDeviceToken"},
		{name: "wrong topic", status: http.StatusBadRequest, reason: "DeviceTokenNotForTopic"},
	} {
		t.Run(tc.name+" drops matching token", func(t *testing.T) {
			store, _ := liveActivityStoreDevice(t, t.TempDir(), "dev-aaaaaa111111")
			if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
				t.Fatal(err)
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(tc.status)
				_, _ = w.Write([]byte(`{"reason":"` + tc.reason + `"}`))
			}))
			defer server.Close()
			sender := directAPNSSender(priv, server.URL, server.Client(), store)
			err := sender.send(LiveActivityRequest{DeviceID: "dev-aaaaaa111111", JobID: "job-1", Token: "aabb", Event: liveActivityEventUpdate})
			if err == nil {
				t.Fatal("invalid token response unexpectedly succeeded")
			}
			if targets := store.ActiveLiveActivityTargets("job-1"); len(targets) != 0 {
				t.Fatalf("invalid token was not dropped: %+v", targets)
			}
		})
	}

	t.Run("replacement survives stale rejection", func(t *testing.T) {
		store, _ := liveActivityStoreDevice(t, t.TempDir(), "dev-aaaaaa111111")
		if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "aabb"); err != nil {
			t.Fatal(err)
		}
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if err := store.SetLiveActivity("dev-aaaaaa111111", "job-1", "ccdd"); err != nil {
				t.Errorf("replace token: %v", err)
			}
			w.WriteHeader(http.StatusGone)
			_, _ = w.Write([]byte(`{"reason":"Unregistered"}`))
		}))
		defer server.Close()
		sender := directAPNSSender(priv, server.URL, server.Client(), store)
		_ = sender.send(LiveActivityRequest{DeviceID: "dev-aaaaaa111111", JobID: "job-1", Token: "aabb", Event: liveActivityEventUpdate})
		targets := store.ActiveLiveActivityTargets("job-1")
		if len(targets) != 1 || targets[0].Token != "ccdd" {
			t.Fatalf("replacement registration was dropped: %+v", targets)
		}
	})
}

func TestLiveActivitySenderLanesOrderAndConcurrency(t *testing.T) {
	_, priv := writeAPNsTestKey(t)
	firstA := make(chan struct{})
	otherB := make(chan struct{})
	secondA := make(chan struct{})
	releaseA := make(chan struct{})
	var mu sync.Mutex
	callsA := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/3/device/aabb":
			mu.Lock()
			callsA++
			call := callsA
			mu.Unlock()
			if call == 1 {
				close(firstA)
				<-releaseA
			} else {
				close(secondA)
			}
		case "/3/device/ccdd":
			close(otherB)
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()
	sender := directAPNSSender(priv, server.URL, server.Client(), nil)
	sender.Enqueue(LiveActivityRequest{DeviceID: "dev-a", JobID: "job-1", Token: "aabb", Event: liveActivityEventUpdate, Timestamp: 1})
	<-firstA
	sender.Enqueue(LiveActivityRequest{DeviceID: "dev-a", JobID: "job-1", Token: "aabb", Event: liveActivityEventEnd, Timestamp: 2})
	sender.Enqueue(LiveActivityRequest{DeviceID: "dev-b", JobID: "job-1", Token: "ccdd", Event: liveActivityEventUpdate, Timestamp: 3})
	select {
	case <-otherB:
	case <-time.After(2 * time.Second):
		t.Fatal("independent device/job lane did not run concurrently")
	}
	select {
	case <-secondA:
		t.Fatal("second same-lane request overtook the blocked first request")
	default:
	}
	close(releaseA)
	select {
	case <-secondA:
	case <-time.After(2 * time.Second):
		t.Fatal("same-lane second request did not run after release")
	}
}
