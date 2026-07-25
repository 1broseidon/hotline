package app

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

type recordingLiveActivitySender struct {
	mu   sync.Mutex
	reqs []LiveActivityRequest
}

func (f *recordingLiveActivitySender) Enqueue(req LiveActivityRequest) {
	f.mu.Lock()
	f.reqs = append(f.reqs, cloneLiveActivityRequest(req))
	f.mu.Unlock()
}

func (f *recordingLiveActivitySender) snapshot() []LiveActivityRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := make([]LiveActivityRequest, len(f.reqs))
	for i := range f.reqs {
		out[i] = cloneLiveActivityRequest(f.reqs[i])
	}
	return out
}

func TestLiveActivityTokenValidationUnknownAndHydration(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	fake := &recordingLiveActivitySender{}
	srv.liveActivitySender = fake
	if msg, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: dev, Action: "start", Title: "Header pass", Detail: "Compiling"}); isErr {
		t.Fatalf("start: %s", msg)
	}
	drainItems(t, sub)

	var writes [][]byte
	write := func(raw []byte) error {
		writes = append(writes, append([]byte(nil), raw...))
		return nil
	}
	feed := func(raw string) (bad, fatal bool) {
		return srv.handleSessionInput(context.Background(), dev, sub, []byte(raw), write)
	}

	// A valid unknown id is intentionally silent and does not persist a token.
	if bad, fatal := feed(`{"t":"live_activity_token","job_id":"job-404","token":"aabb"}`); bad || fatal {
		t.Fatalf("unknown job: bad=%v fatal=%v", bad, fatal)
	}
	if len(fake.snapshot()) != 0 || len(writes) != 0 {
		t.Fatalf("unknown job produced side effects: reqs=%+v writes=%q", fake.snapshot(), writes)
	}

	// Registering a running job persists and synchronously enqueues its current
	// state as hydration before this handler returns.
	if bad, fatal := feed(`{"t":"live_activity_token","job_id":"job-1","token":"aabb"}`); bad || fatal {
		t.Fatalf("valid registration: bad=%v fatal=%v", bad, fatal)
	}
	reqs := fake.snapshot()
	if len(reqs) != 1 {
		t.Fatalf("hydration requests=%d, want 1", len(reqs))
	}
	got := reqs[0]
	if got.DeviceID != dev || got.JobID != "job-1" || got.Token != "aabb" || got.Event != liveActivityEventUpdate ||
		got.Content.Title != "Header pass" || got.Content.State != "running" || got.Content.Detail != "Compiling" || got.Content.Progress != nil || got.Timestamp <= 0 {
		t.Fatalf("hydration request=%+v", got)
	}
	device, _ := srv.store.Device(dev)
	if reg := device.LiveActivities["job-1"]; reg.Token != "aabb" || reg.RegisteredAt == "" {
		t.Fatalf("persisted registration=%+v", reg)
	}

	// Empty token is an idempotent unregister and emits no APNs request.
	for i := 0; i < 2; i++ {
		if bad, fatal := feed(`{"t":"live_activity_token","job_id":"job-1","token":""}`); bad || fatal {
			t.Fatalf("unregister %d: bad=%v fatal=%v", i, bad, fatal)
		}
	}
	device, _ = srv.store.Device(dev)
	if len(device.LiveActivities) != 0 || len(fake.snapshot()) != 1 {
		t.Fatalf("unregister state=%+v reqs=%+v", device.LiveActivities, fake.snapshot())
	}

	badFrames := []string{
		`{"t":"live_activity_token","job_id":"job-1"}`,
		`{"t":"live_activity_token","token":"aabb"}`,
		`{"t":"live_activity_token","JOB_ID":"job-1","token":"aabb"}`,
		`{"t":"live_activity_token","job_id":"job-1","TOKEN":"aabb"}`,
		`{"t":"live_activity_token","job_id":"job-0","token":"aabb"}`,
		`{"t":"live_activity_token","job_id":"job-01","token":"aabb"}`,
		`{"t":"live_activity_token","job_id":"job-1","token":"ABCDEF"}`,
		`{"t":"live_activity_token","job_id":"job-1","token":"abc"}`,
		`{"t":"live_activity_token","job_id":"job-1","token":"` + strings.Repeat("aa", 101) + `"}`,
	}
	for _, raw := range badFrames {
		before := len(writes)
		bad, fatal := feed(raw)
		if !bad || fatal {
			t.Fatalf("malformed frame %s: bad=%v fatal=%v", raw, bad, fatal)
		}
		if len(writes) != before+1 {
			t.Fatalf("malformed frame did not write one error: %s", raw)
		}
		var frame struct {
			T    string `json:"t"`
			Code string `json:"code"`
		}
		if err := json.Unmarshal(writes[len(writes)-1], &frame); err != nil || frame.T != "error" || frame.Code != "bad_frame" {
			t.Fatalf("malformed response=%s err=%v", writes[len(writes)-1], err)
		}
	}
}

func TestLiveActivityJobUpdateEndOrderingAndCleanup(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	fake := &recordingLiveActivitySender{}
	srv.liveActivitySender = fake
	ctx := context.Background()
	progress := 0.25
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "Ship", Detail: "Build", Progress: &progress}); isErr {
		t.Fatalf("start: %s", msg)
	}
	drainItems(t, sub)
	if bad, fatal := srv.handleSessionInput(ctx, dev, sub, []byte(`{"t":"live_activity_token","job_id":"job-1","token":"aabb"}`), func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("register: bad=%v fatal=%v", bad, fatal)
	}

	updatedProgress := 0.75
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "update", JobID: "job-1", Detail: "Test", Progress: &updatedProgress}); isErr {
		t.Fatalf("update: %s", msg)
	}
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "done", JobID: "job-1", State: "ok", Detail: "Shipped"}); isErr {
		t.Fatalf("done: %s", msg)
	}

	reqs := fake.snapshot()
	if len(reqs) != 3 {
		t.Fatalf("requests=%d, want hydration/update/end: %+v", len(reqs), reqs)
	}
	if reqs[0].Event != liveActivityEventUpdate || reqs[0].Content.State != "running" || reqs[0].Content.Detail != "Build" || reqs[0].Content.Progress == nil || *reqs[0].Content.Progress != 0.25 {
		t.Fatalf("hydration=%+v", reqs[0])
	}
	if reqs[1].Event != liveActivityEventUpdate || reqs[1].Content.State != "running" || reqs[1].Content.Detail != "Test" || reqs[1].Content.Progress == nil || *reqs[1].Content.Progress != 0.75 {
		t.Fatalf("update=%+v", reqs[1])
	}
	if reqs[2].Event != liveActivityEventEnd || reqs[2].Content.State != "ok" || reqs[2].Content.Detail != "Shipped" || reqs[2].Content.Progress == nil || *reqs[2].Content.Progress != 0.75 {
		t.Fatalf("end=%+v", reqs[2])
	}
	for i, req := range reqs {
		if req.DeviceID != dev || req.JobID != "job-1" || req.Token != "aabb" {
			t.Fatalf("request %d target=%+v", i, req)
		}
	}
	device, _ := srv.store.Device(dev)
	if len(device.LiveActivities) != 0 {
		t.Fatalf("terminal job retained registration: %+v", device.LiveActivities)
	}

	// A valid token for a remembered terminal id is also a silent no-op.
	before := len(fake.snapshot())
	if bad, fatal := srv.handleSessionInput(ctx, dev, sub, []byte(`{"t":"live_activity_token","job_id":"job-1","token":"ccdd"}`), func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("finished job token: bad=%v fatal=%v", bad, fatal)
	}
	if len(fake.snapshot()) != before {
		t.Fatal("finished job registration was dispatched")
	}
}

func TestLiveActivityNewStartClearsReusedJobID(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	if err := srv.store.SetLiveActivity(dev, "job-1", "aabb"); err != nil {
		t.Fatal(err)
	}
	if msg, isErr := tools.Job(context.Background(), mcpchan.JobInput{ChatID: dev, Action: "start", Title: "Reused"}); isErr {
		t.Fatalf("start: %s", msg)
	}
	drainItems(t, sub)
	device, _ := srv.store.Device(dev)
	if len(device.LiveActivities) != 0 {
		t.Fatalf("new job lifecycle inherited stale registration: %+v", device.LiveActivities)
	}
}
