package app

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
)

func TestJobRegistryReloadPersistsRecordsAndSequence(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cards", "registry.json")
	reg, err := newPersistentJobRegistry(path)
	if err != nil {
		t.Fatal(err)
	}
	progress := 0.375
	reg.rehydrate(&jobRecord{
		jobID: "job-7", elementID: "el-job-7", messageID: "a-41", chatID: "app",
		title: "Persist me", detail: "working", progress: &progress, startedAt: 1234, stale: true,
	})

	reloaded, err := newPersistentJobRegistry(path)
	if err != nil {
		t.Fatal(err)
	}
	if reloaded.seq != 7 {
		t.Fatalf("sequence=%d, want 7", reloaded.seq)
	}
	rec := reloaded.jobs["job-7"]
	if rec == nil || rec.elementID != "el-job-7" || rec.messageID != "a-41" || rec.chatID != "app" ||
		rec.title != "Persist me" || rec.detail != "working" || rec.startedAt != 1234 || !rec.stale ||
		rec.progress == nil || *rec.progress != progress {
		t.Fatalf("reloaded record=%+v", rec)
	}

	reloaded.mu.Lock()
	reloaded.finishLocked("job-7", "ok", false)
	err = reloaded.persistLocked()
	reloaded.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	empty, err := newPersistentJobRegistry(path)
	if err != nil {
		t.Fatal(err)
	}
	if empty.seq != 7 || len(empty.jobs) != 0 {
		t.Fatalf("terminal reload: sequence=%d jobs=%d, want 7/0", empty.seq, len(empty.jobs))
	}

	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(raw) == 0 || raw[len(raw)-1] != '\n' || !strings.Contains(string(raw), "\n  \"sequence\"") {
		t.Fatalf("registry is not indented newline-terminated JSON: %q", raw)
	}
	if info, err := os.Stat(filepath.Dir(path)); err != nil || info.Mode().Perm() != 0o700 {
		t.Fatalf("cards dir mode=%v err=%v, want 0700", infoMode(info), err)
	}
	if info, err := os.Stat(path); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("registry mode=%v err=%v, want 0600", infoMode(info), err)
	}
}

func TestJobCardRestartSweepDoneAndSequence(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{
		ChatID: dev, Action: "start", Title: "Restartable", Detail: "starting",
	}); isErr {
		t.Fatalf("start: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 {
		t.Fatalf("start items=%+v", items)
	}
	original := *srv.jobs.jobs["job-1"]
	progress := 0.6
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{
		ChatID: dev, Action: "update", JobID: "job-1", Detail: "halfway", Progress: &progress,
	}); isErr {
		t.Fatalf("update: %s", msg)
	}
	drainItems(t, sub)
	before := len(srv.outbox.framesAfter(0))
	srv.stopAgentStateEmitter()
	srv.outbox.close()

	restarted := NewServer(srv.cfg, transcript.New(srv.cfg.TranscriptFile))
	if restarted.initErr != nil {
		t.Fatal(restarted.initErr)
	}
	frames := restarted.outbox.framesAfter(0)
	if len(frames) != before+1 {
		t.Fatalf("durable frames after restart=%d, want %d", len(frames), before+1)
	}
	stale := decodeFrame(t, frames[len(frames)-1].data)
	assertJobEdit(t, stale, original.messageID, original.elementID, "cancelled", boxRestartedDetail)
	if runs := restarted.jobs.activeRuns(maxAgentRuns); len(runs) != 0 {
		t.Fatalf("restart-stale activeRuns=%+v, want empty", runs)
	}
	if rec := restarted.jobs.jobs["job-1"]; rec == nil || !rec.stale || rec.detail != boxRestartedDetail {
		t.Fatalf("restart-stale record=%+v", rec)
	}
	restarted.elIndex.mu.Lock()
	identity := restarted.elIndex.byMsg[original.messageID][original.elementID]
	restarted.elIndex.mu.Unlock()
	if !identity {
		t.Fatal("original message/element identity was not restored")
	}

	// A second boot sees the durable stale marker and emits no second edit.
	restarted.outbox.close()
	restarted2 := NewServer(srv.cfg, transcript.New(srv.cfg.TranscriptFile))
	if restarted2.initErr != nil {
		t.Fatal(restarted2.initErr)
	}
	t.Cleanup(restarted2.outbox.close)
	if got := len(restarted2.outbox.framesAfter(0)); got != len(frames) {
		t.Fatalf("second restart durable frames=%d, want unchanged %d", got, len(frames))
	}

	// A true completion after restart targets the original card and removes the
	// stale row while retaining the monotonic sequence.
	tools2 := NewTools(restarted2, srv.cfg, transcript.New(srv.cfg.TranscriptFile))
	if msg, isErr := tools2.Job(ctx, mcpchan.JobInput{
		ChatID: dev, Action: "done", JobID: "job-1", State: "ok", Detail: "finished after restart",
	}); isErr {
		t.Fatalf("done after restart: %s", msg)
	}
	frames = restarted2.outbox.framesAfter(0)
	final := decodeFrame(t, frames[len(frames)-1].data)
	assertJobEdit(t, final, original.messageID, original.elementID, "ok", "finished after restart")

	disk := readJobRegistryDisk(t, jobRegistryStoragePath(srv.cfg.StateDir))
	if disk.Sequence != 1 || len(disk.Active) != 0 {
		t.Fatalf("registry after done: sequence=%d active=%+v", disk.Sequence, disk.Active)
	}
	if msg, isErr := tools2.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "Next"}); isErr {
		t.Fatalf("next start: %s", msg)
	} else if !strings.Contains(msg, "job_id: job-2") {
		t.Fatalf("next start=%q, want job-2", msg)
	}
}

func TestStaleJobUpdateReactivatesAndPersists(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	ctx := context.Background()
	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{ChatID: dev, Action: "start", Title: "Resume me"}); isErr {
		t.Fatalf("start: %s", msg)
	}
	drainItems(t, sub)
	srv.restoreJobCards()
	drainItems(t, sub)
	if rec := srv.jobs.jobs["job-1"]; rec == nil || !rec.stale {
		t.Fatalf("swept record=%+v", rec)
	}

	if msg, isErr := tools.Job(ctx, mcpchan.JobInput{
		ChatID: dev, Action: "update", JobID: "job-1", Detail: "running again",
	}); isErr {
		t.Fatalf("reactivate: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 {
		t.Fatalf("reactivation items=%+v", items)
	}
	el := items[0]["elements"].([]any)[0].(map[string]any)
	if el["state"] != "running" || el["detail"] != "running again" {
		t.Fatalf("reactivation element=%+v", el)
	}
	if runs := srv.jobs.activeRuns(maxAgentRuns); len(runs) != 1 || runs[0].ID != "job-1" {
		t.Fatalf("reactivated activeRuns=%+v", runs)
	}
	disk := readJobRegistryDisk(t, jobRegistryStoragePath(srv.cfg.StateDir))
	if len(disk.Active) != 1 || disk.Active[0].Stale || disk.Active[0].Detail != "running again" {
		t.Fatalf("reactivated disk record=%+v", disk.Active)
	}
}

func TestRestartSweepTakesAndEndsLiveActivity(t *testing.T) {
	srv, tools, dev, sub := activeHarness(t)
	if msg, isErr := tools.Job(context.Background(), mcpchan.JobInput{
		ChatID: dev, Action: "start", Title: "Island", Detail: "working",
	}); isErr {
		t.Fatalf("start: %s", msg)
	}
	drainItems(t, sub)
	if err := srv.store.SetLiveActivity(dev, "job-1", "aabb"); err != nil {
		t.Fatal(err)
	}
	fake := &recordingLiveActivitySender{}
	srv.liveActivitySender = fake

	srv.restoreJobCards()
	reqs := fake.snapshot()
	if len(reqs) != 1 {
		t.Fatalf("restart APNs requests=%+v, want one end", reqs)
	}
	req := reqs[0]
	if req.DeviceID != dev || req.JobID != "job-1" || req.Token != "aabb" ||
		req.Event != liveActivityEventEnd || req.Content.State != "cancelled" || req.Content.Detail != boxRestartedDetail {
		t.Fatalf("restart APNs end=%+v", req)
	}
	device, _ := srv.store.Device(dev)
	if len(device.LiveActivities) != 0 {
		t.Fatalf("restart retained activity tokens=%+v", device.LiveActivities)
	}

	// The stale marker suppresses a second card edit, and the atomic take leaves
	// no token to end again.
	drainItems(t, sub)
	srv.restoreJobCards()
	if got := len(fake.snapshot()); got != 1 {
		t.Fatalf("second sweep APNs requests=%d, want unchanged 1", got)
	}
	if items := drainItems(t, sub); len(items) != 0 {
		t.Fatalf("second sweep emitted duplicate edit: %+v", items)
	}
}

func assertJobEdit(t *testing.T, frame map[string]any, messageID, elementID, state, detail string) {
	t.Helper()
	if frame["t"] != "edit" || frame["id"] != messageID {
		t.Fatalf("job edit target=%+v, want message %s", frame, messageID)
	}
	els, ok := frame["elements"].([]any)
	if !ok || len(els) != 1 {
		t.Fatalf("job edit elements=%+v", frame["elements"])
	}
	el := els[0].(map[string]any)
	if el["id"] != elementID || el["state"] != state || el["detail"] != detail {
		t.Fatalf("job edit element=%+v, want id=%s state=%s detail=%q", el, elementID, state, detail)
	}
}

func readJobRegistryDisk(t *testing.T, path string) jobRegistryDisk {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var disk jobRegistryDisk
	if err := json.Unmarshal(raw, &disk); err != nil {
		t.Fatal(err)
	}
	return disk
}

func infoMode(info os.FileInfo) os.FileMode {
	if info == nil {
		return 0
	}
	return info.Mode().Perm()
}
