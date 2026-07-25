package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// setJobCompletionPushRide builds the FB44 nested text control at the existing
// device_send boundary. enabled is any so tests can forge malformed values; nil
// omits the key entirely.
func setJobCompletionPushRide(t *testing.T, cid string, enabled any) []byte {
	t.Helper()
	control := map[string]any{"t": "set_job_completion_push"}
	if enabled != nil {
		control["enabled"] = enabled
	}
	inner, err := json.Marshal(control)
	if err != nil {
		t.Fatalf("marshal set_job_completion_push: %v", err)
	}
	payload, err := json.Marshal(map[string]any{"t": "send", "text": string(inner)})
	if err != nil {
		t.Fatalf("marshal payload: %v", err)
	}
	raw, err := json.Marshal(map[string]any{"t": "device_send", "cid": cid, "payload": json.RawMessage(payload)})
	if err != nil {
		t.Fatalf("marshal device_send: %v", err)
	}
	return raw
}

func TestSetJobCompletionPushParsePersistSilent(t *testing.T) {
	srv, _, dev, sub := activeHarness(t)
	sink := newFakeSink()
	srv.bindSink(sink)
	var writes [][]byte
	write := func(b []byte) error {
		writes = append(writes, append([]byte(nil), b...))
		return nil
	}
	ride := func(cid string, enabled any) {
		t.Helper()
		if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, setJobCompletionPushRide(t, cid, enabled), write); bad || fatal {
			t.Fatalf("control %s: bad=%v fatal=%v, want silent consume", cid, bad, fatal)
		}
	}
	pref := func(store *RelayStore) *bool {
		d, ok := store.Device(dev)
		if !ok {
			t.Fatalf("device %s vanished", dev)
		}
		return d.JobCompletionPush
	}

	if p := pref(srv.store); p != nil {
		t.Fatalf("fresh device preference = %v, want nil/default-enabled", *p)
	}
	ride("cid-jcp-false00001", false)
	if p := pref(srv.store); p == nil || *p {
		t.Fatalf("enabled=false not persisted: %v", p)
	}
	ride("cid-jcp-true000001", true)
	if p := pref(srv.store); p == nil || !*p {
		t.Fatalf("enabled=true not persisted: %v", p)
	}

	// Malformed and missing enabled values are controls, but leave the prior
	// explicit value untouched and produce no protocol or harness output.
	ride("cid-jcp-bad0000001", "yes")
	ride("cid-jcp-missing000", nil)
	if p := pref(srv.store); p == nil || !*p {
		t.Fatalf("malformed/missing enabled changed preference: %v", p)
	}
	if len(writes) != 0 {
		t.Fatalf("control emitted %d error/echo frames, want 0", len(writes))
	}
	select {
	case got := <-sink.ch:
		t.Fatalf("control leaked to harness: %+v", got)
	default:
	}
	if data, err := os.ReadFile(srv.cfg.TranscriptFile); err == nil && strings.TrimSpace(string(data)) != "" {
		t.Fatalf("control leaked to transcript: %s", data)
	} else if err != nil && !os.IsNotExist(err) {
		t.Fatalf("read transcript: %v", err)
	}

	// FB23-style durability: another store process sees the explicit value.
	reloaded, err := OpenRelayStore(filepath.Dir(srv.store.path))
	if err != nil {
		t.Fatalf("reload store: %v", err)
	}
	if p := pref(reloaded); p == nil || !*p {
		t.Fatalf("reloaded preference = %v, want true", p)
	}
}

func TestJobCompletionIntentForegroundAndTerminalStates(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	intents := make(chan pushIntent, 8)
	srv.mailbox.onCustomPushIntent = func(_ string, _ MailboxItem, intent pushIntent) {
		intents <- intent
	}
	ctx := context.Background()

	// A foreground bare success still lands its terminal edit but never reaches
	// the custom push callback.
	if _, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "foreground job", Detail: "done here"}); isErr {
		t.Fatal("foreground start failed")
	}
	if _, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok"}); isErr {
		t.Fatal("foreground done failed")
	}

	// Move away, then drive err/cancelled completions. Neither may attach a
	// completion intent. The final whitespace-notify success is a scheduling
	// sentinel: its callback proves the seam is live and must be the first one.
	srv.mailbox.unsubscribe(dev, sub)
	for i, state := range []string{"err", "cancelled"} {
		title := state + " job"
		if _, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: title}); isErr {
			t.Fatalf("%s start failed", state)
		}
		if _, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-" + string(rune('2'+i)), State: state}); isErr {
			t.Fatalf("%s done failed", state)
		}
	}
	if _, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "whitespace notify", Detail: "first detail"}); isErr {
		t.Fatal("whitespace start failed")
	}
	msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-4", State: "ok", Detail: "final detail", Notify: " \t\n "})
	if isErr {
		t.Fatalf("whitespace done failed: %s", msg)
	}
	if strings.Contains(msg, "notify_id") {
		t.Fatalf("whitespace notify created a fresh message: %s", msg)
	}
	select {
	case got := <-intents:
		if got.Title != "whitespace notify" || got.Body != "final detail" {
			t.Fatalf("first completion intent = %+v, want whitespace job final record", got)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("whitespace notify was not normalized to a bare completion")
	}
	select {
	case extra := <-intents:
		t.Fatalf("foreground/err/cancelled produced an extra completion intent: %+v", extra)
	case <-time.After(50 * time.Millisecond):
	}

	// The away sentinel also proves the earlier foreground success did not push.
	mb := srv.mailbox.disk.Devices[dev]
	if got := len(mb.Items); got != 8 { // four starts + four terminal edits; no notify msg
		t.Fatalf("mailbox items = %d, want 8 (whitespace notify must not add a msg)", got)
	}
}

func TestJobCompletionPreferenceMixedDevices(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 3)
	for i := range ids {
		srv.mailbox.unsubscribe(ids[i], subs[i])
	}
	// ids[0] remains nil/default-enabled; ids[1] is explicit true; ids[2] opts out.
	if err := srv.store.SetDeviceJobCompletionPush(ids[1], true); err != nil {
		t.Fatal(err)
	}
	if err := srv.store.SetDeviceJobCompletionPush(ids[2], false); err != nil {
		t.Fatal(err)
	}
	got := make(chan string, 4)
	srv.mailbox.onCustomPushIntent = func(deviceID string, _ MailboxItem, _ pushIntent) {
		got <- deviceID
	}
	tools := NewTools(srv, srv.cfg, srv.log)
	if _, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: unifiedChatID, Action: "start", Title: "fanout job"}); isErr {
		t.Fatal("fanout start failed")
	}
	if _, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: unifiedChatID, Action: "done", JobID: "job-1", State: "ok", Detail: "all devices updated"}); isErr {
		t.Fatal("fanout done failed")
	}

	var pushed []string
	for len(pushed) < 2 {
		select {
		case id := <-got:
			pushed = append(pushed, id)
		case <-time.After(2 * time.Second):
			t.Fatalf("completion callbacks = %v, want nil+true devices", pushed)
		}
	}
	sort.Strings(pushed)
	want := append([]string(nil), ids[:2]...)
	sort.Strings(want)
	if strings.Join(pushed, ",") != strings.Join(want, ",") {
		t.Fatalf("completion devices = %v, want %v", pushed, want)
	}
	select {
	case extra := <-got:
		t.Fatalf("opted-out device received completion intent: %s", extra)
	case <-time.After(50 * time.Millisecond):
	}

	// Preference selection changes only the internal intent. Every device still
	// receives the same start + terminal edit fanout.
	for _, id := range ids {
		if n := len(srv.mailbox.disk.Devices[id].Items); n != 2 {
			t.Fatalf("device %s mailbox items = %d, want 2", id, n)
		}
	}
}

func TestJobCompletionCustomTransportRouting(t *testing.T) {
	t.Run("unregistered core room keeps Expo fallback", func(t *testing.T) {
		coreHit := make(chan string, 1)
		core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			coreHit <- r.URL.Path
			w.WriteHeader(http.StatusOK)
		}))
		defer core.Close()
		expoBody := make(chan map[string]any, 1)
		expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			expoBody <- body
			w.WriteHeader(http.StatusOK)
		}))
		defer expo.Close()
		srv, deviceID, roomID := coreModeServer(t, core.URL, expo.URL)

		srv.maybePushWithIntent(deviceID, pushItem(), pushIntent{Title: "deploy complete", Body: "all checks passed"})
		select {
		case body := <-expoBody:
			if body["title"] != "deploy complete" || body["body"] != "all checks passed" {
				t.Fatalf("Expo custom notification = %+v", body)
			}
			data, _ := body["data"].(map[string]any)
			if data["url"] != "hotline://chat" || data["room"] != roomID {
				t.Fatalf("Expo tap routing = %+v", data)
			}
		case <-time.After(2 * time.Second):
			t.Fatal("unregistered core room did not use Expo fallback")
		}
		select {
		case path := <-coreHit:
			t.Fatalf("unregistered room unexpectedly called core: %s", path)
		default:
		}
	})

	t.Run("non-core signed gateway keeps custom content and routing", func(t *testing.T) {
		requests := make(chan struct {
			Path  string
			KeyID string
			Body  map[string]any
		}, 1)
		gateway := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			requests <- struct {
				Path  string
				KeyID string
				Body  map[string]any
			}{r.URL.Path, r.Header.Get("X-Hotline-Key-Id"), body}
			w.WriteHeader(http.StatusOK)
		}))
		defer gateway.Close()
		cfg := testConfig(t)
		cfg.AppPushEndpoint = gateway.URL
		srv, _ := newTools(cfg)
		if srv.initErr != nil {
			t.Fatal(srv.initErr)
		}
		t.Cleanup(func() { srv.outbox.close() })
		const deviceID = "dev-gateway000001"
		const roomID = "gateway-room"
		srv.store.st.Rooms[roomID] = RoomRecord{ID: roomID}
		srv.store.st.Devices[deviceID] = DeviceRecord{ID: deviceID, Room: roomID, State: DeviceActive}
		if err := srv.store.SetPush(deviceID, "aabbccddeeff0011", "ios"); err != nil {
			t.Fatal(err)
		}
		if err := srv.store.SetPushKeyID(deviceID, "key-fb44"); err != nil {
			t.Fatal(err)
		}

		srv.maybePushWithIntent(deviceID, pushItem(), pushIntent{Title: "gateway done", Body: "signed detail"})
		select {
		case req := <-requests:
			if req.Path != "/v1/push" || req.KeyID != "key-fb44" {
				t.Fatalf("gateway request path/key = %q/%q", req.Path, req.KeyID)
			}
			if req.Body["title"] != "gateway done" || req.Body["body"] != "signed detail" {
				t.Fatalf("gateway custom notification = %+v", req.Body)
			}
			data, _ := req.Body["data"].(map[string]any)
			if data["url"] != "hotline://chat" || data["room"] != roomID {
				t.Fatalf("gateway tap routing = %+v", data)
			}
		case <-time.After(2 * time.Second):
			t.Fatal("signed gateway push not received")
		}
	})
}

func TestCustomPushIntentFiresOnlyAtFreshAwayDurabilityBoundary(t *testing.T) {
	path := filepath.Join(t.TempDir(), "mailboxes.json")
	mailbox, err := newLocalMailbox(path)
	if err != nil {
		t.Fatal(err)
	}
	const deviceID = "dev-boundary00001"
	mailbox.disk.Devices[deviceID] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0", JHead: "0", CHead: "0"}
	custom := make(chan MailboxItem, 4)
	generic := make(chan MailboxItem, 2)
	mailbox.onCustomPushIntent = func(_ string, item MailboxItem, _ pushIntent) { custom <- item }
	mailbox.onPushIntent = func(_ string, item MailboxItem) { generic <- item }
	intent := &pushIntent{Title: "done", Body: "detail"}
	payload := []byte(`{"t":"edit","seq":1,"id":"a-1","text":"job: detail","elements":[{"el":"job","id":"el-1","fallback":"job: detail"}]}`)

	// Fresh + durable + away: custom callback fires even though the terminal
	// element-only edit is generically push-ineligible.
	if _, err := mailbox.enqueueWithPushIntent(deviceID, "1", payload, intent); err != nil {
		t.Fatal(err)
	}
	select {
	case item := <-custom:
		if item.J != "1" {
			t.Fatalf("first custom callback j=%s, want 1", item.J)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("fresh away custom intent did not fire")
	}

	// Dedup of the same durable envelope never re-fires the custom callback.
	if _, err := mailbox.enqueueWithPushIntent(deviceID, "1", payload, intent); err != nil {
		t.Fatal(err)
	}
	// Existing wrapper callers retain generic behavior on a fresh eligible item,
	// and cannot carry a custom callback (including reconciliation/backfill).
	if _, err := mailbox.enqueue(deviceID, "2", []byte(`{"t":"msg","seq":2,"id":"a-2","text":"generic"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case item := <-generic:
		if item.J != "2" {
			t.Fatalf("generic callback j=%s, want 2", item.J)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("generic enqueue wrapper stopped firing onPushIntent")
	}

	// Foreground snapshot suppresses the custom callback.
	_, _, _, sub, err := mailbox.stateAndSubscribe(deviceID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := mailbox.enqueueWithPushIntent(deviceID, "3", payload, intent); err != nil {
		t.Fatal(err)
	}
	mailbox.unsubscribe(deviceID, sub)

	// A failed durable save rolls back and never fires. Recovering the path and
	// retrying the same j must be a genuinely fresh insert that does fire.
	blocker := filepath.Join(t.TempDir(), "not-a-directory")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	mailbox.path = filepath.Join(blocker, "mailboxes.json")
	if _, err := mailbox.enqueueWithPushIntent(deviceID, "4", payload, intent); err == nil {
		t.Fatal("custom enqueue should fail when mailbox persistence fails")
	}
	mailbox.path = path
	if _, err := mailbox.enqueueWithPushIntent(deviceID, "4", payload, intent); err != nil {
		t.Fatalf("retry after persistence recovery: %v", err)
	}
	select {
	case item := <-custom:
		if item.J != "4" {
			t.Fatalf("unexpected custom callback j=%s; dedup/foreground/failure fired", item.J)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("recovered fresh custom insert did not fire")
	}
	select {
	case item := <-custom:
		t.Fatalf("extra custom callback from dedup/foreground/failure: j=%s", item.J)
	case <-time.After(50 * time.Millisecond):
	}
}

func TestJobCompletionCoreRegisteredUsesExistingWake(t *testing.T) {
	set := func(v bool) *bool { return &v }
	cases := []struct {
		name        string
		envClear    bool
		deviceClear *bool
		wantPreview bool
	}{
		{name: "device allows detail", envClear: false, deviceClear: set(true), wantPreview: true},
		{name: "device hides detail", envClear: true, deviceClear: set(false), wantPreview: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cap := newWakeCapture()
			srv, deviceID, roomID := previewServer(t, tc.envClear, cap)
			if err := srv.store.SetDevicePushPreview(deviceID, *tc.deviceClear); err != nil {
				t.Fatal(err)
			}
			expoHit := make(chan struct{}, 1)
			expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				expoHit <- struct{}{}
				w.WriteHeader(http.StatusOK)
			}))
			defer expo.Close()
			srv.pushEndpoint = expo.URL

			srv.maybePushWithIntent(deviceID, pushItem(), pushIntent{Title: "must not ride wake", Body: "registered detail"})
			cap.wait(t)
			body := cap.decode(t)
			if cap.path != "/v1/rooms/"+roomID+"/wake" || cap.signature == "" {
				t.Fatalf("registered completion did not use signed wake: path=%q signature=%q", cap.path, cap.signature)
			}
			if _, exists := body["title"]; exists {
				t.Fatalf("wake invented a title field: %s", cap.body)
			}
			preview, present := body["preview"]
			if present != tc.wantPreview {
				t.Fatalf("preview present=%v, want %v: %s", present, tc.wantPreview, cap.body)
			}
			if present && preview != "registered detail" {
				t.Fatalf("completion preview = %v, want final detail", preview)
			}
			if !tc.wantPreview {
				want := `{"device_id":"dev-af31fd290542","kind":"message","preview_c":null}`
				if string(cap.body) != want {
					t.Fatalf("no-preview wake changed wire body:\n got %s\nwant %s", cap.body, want)
				}
			}
			select {
			case <-expoHit:
				t.Fatal("registered completion bypassed core for Expo")
			default:
			}
		})
	}
}
