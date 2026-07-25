package app

// Control-rail hardening (sol review #1 CRITICAL, #2 MAJOR).
//
// A set_sdk_config rides a device_send but is NOT a message: the box consumes
// it silently and answers on the transient, rid-correlated sdk_config_result,
// never with a mailbox `sent` echo. Every app build before the control-rail fix
// therefore parked that device_send in a pending outbox that could never settle
// — so it re-flushed on EVERY reconnect. The app side now writes controls
// straight to the socket, but the box cannot patch clients already in the
// field, so it must be idempotent under replay on its own.
//
// These tests fail without the settled-rid cache and the atomic pending
// registration:
//
//   - Replay: a second delivery of an already-answered rid must be re-answered
//     from cache, NOT forwarded to the harness and NOT re-persisted.
//   - Duplicate rid in flight: the second delivery must not replace the pending
//     entry, or the harness's answer to request A settles request B.
//   - Concurrency: two overlapping applies must not interleave through .env.

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// TestSetSDKConfigReplayIsIdempotent is the release-blocker regression. An
// outbox-replaying client re-sends a settled model change on every reconnect;
// without the settled-rid cache the box re-forwards it, re-applies it to the
// live session, and re-writes .env — so a model change from an hour ago
// silently takes effect again.
func TestSetSDKConfigReplayIsIdempotent(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "r-replay-0000000000000001"
	ride := sdkConfigRide(t, "cid-sdk-replay001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})

	// First delivery: forwarded, confirmed, persisted, answered.
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("first ride: bad=%v fatal=%v", bad, fatal)
	}
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	first := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(first) != 1 || first[0]["ok"] != true || first[0]["model"] != "claude-sonnet-4-6" {
		t.Fatalf("first result = %v, want one ok", first)
	}
	if got := len(fwd.snapshot()); got != 1 {
		t.Fatalf("first delivery forwarded %d times, want 1", got)
	}

	// The operator now moves the box on by hand (or a later apply lands): .env
	// and the live identity both say opus again. This is what makes a replay
	// observable — a re-apply would drag the box back to sonnet.
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})

	// REPLAY: the exact same frame, exactly as a reconnecting outbox re-flushes it.
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("replayed ride: bad=%v fatal=%v", bad, fatal)
	}

	if got := len(fwd.snapshot()); got != 1 {
		t.Fatalf("replay forwarded to the harness %d times, want 1 — an old model change re-applied to the live session", got)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-opus-4-8") {
		t.Fatalf("replay rewrote .env — the box silently reverted to a stale model:\n%s", raw)
	}
	if pendingCount(srv) != 0 {
		t.Fatalf("replay registered a pending apply: %d", pendingCount(srv))
	}

	// The client still gets an answer (it is waiting on one), and it is the
	// SAME outcome — replay is idempotent, not silent.
	replayed := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(replayed) != 1 {
		t.Fatalf("replay answered %d frames, want 1 cached answer", len(replayed))
	}
	r := replayed[0]
	if r["rid"] != rid || r["ok"] != true || r["hot"] != true || r["model"] != "claude-sonnet-4-6" {
		t.Fatalf("replayed answer = %v, want the cached original", r)
	}
}

// TestSetSDKConfigReplayOfRefusalDoesNotReapply covers the restart path, which
// has the louder side effect: a replayed control there would persist AND file a
// second supervisor bounce.
func TestSetSDKConfigReplayDoesNotRebounce(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	// No forwarder bound: the forwarder-unbound fallback (old binaries, SC9)
	// takes the persist → bounce → restart:true path.

	const rid = "r-replay-0000000000000002"
	ride := sdkConfigRide(t, "cid-sdk-replay002", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("first ride: bad=%v fatal=%v", bad, fatal)
	}
	first := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(first) != 1 || first[0]["restart"] != true {
		t.Fatalf("first result = %v, want restart:true", first)
	}
	restartReq := filepath.Join(srv.supervisorDir, "restart.request")
	if _, err := os.Stat(restartReq); err != nil {
		t.Fatalf("restart not requested: %v", err)
	}
	if err := os.Remove(restartReq); err != nil {
		t.Fatal(err)
	}

	// REPLAY.
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("replayed ride: bad=%v fatal=%v", bad, fatal)
	}
	if _, err := os.Stat(restartReq); !os.IsNotExist(err) {
		t.Fatalf("replay filed a SECOND supervisor bounce (err=%v)", err)
	}
	replayed := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(replayed) != 1 || replayed[0]["restart"] != true || replayed[0]["rid"] != rid {
		t.Fatalf("replayed answer = %v, want the cached original", replayed)
	}
}

// TestSetSDKConfigDuplicateRidDoesNotSupersede is sol #3's core case. Two
// requests sharing a rid used to REPLACE each other in the pending map: the
// first device was orphaned and the harness's answer — computed for request A —
// was persisted and reported as the answer to request B.
func TestSetSDKConfigDuplicateRidDoesNotSupersede(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "r-dup-00000000000000000001"
	// Request A: a MODEL change.
	rideA := sdkConfigRide(t, "cid-sdk-dup000001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, rideA, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("ride A: bad=%v fatal=%v", bad, fatal)
	}
	// Request B: an EFFORT change reusing the same rid (a colliding
	// timestamp+counter cid, or a second device).
	rideB := sdkConfigRide(t, "cid-sdk-dup000002", map[string]any{"rid": rid, "effort": "low"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, rideB, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("ride B: bad=%v fatal=%v", bad, fatal)
	}

	// B must not have displaced A: exactly one forward, exactly one pending.
	calls := fwd.snapshot()
	if len(calls) != 1 || deref(calls[0].Model) != "claude-sonnet-4-6" {
		t.Fatalf("forward calls = %+v, want only request A", calls)
	}
	if pendingCount(srv) != 1 {
		t.Fatalf("pending = %d, want 1", pendingCount(srv))
	}

	// The harness answers A. It must settle A — model — and never be read as
	// the answer to B's effort request.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["model"] != "claude-sonnet-4-6" {
		t.Fatalf("result = %v, want request A's model", r)
	}
	if _, present := r["effort"]; present {
		t.Fatalf("request A's result carried request B's effort field: %v", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "HOTLINE_SDK_EFFORT") {
		t.Fatalf("request B's effort was persisted by request A's confirmation:\n%s", raw)
	}
}

// TestSetSDKConfigSerializesApplies: a second, DIFFERENT apply while one is in
// flight is refused with an explicit code rather than racing it through the
// live session and a concurrent .env read-modify-write.
func TestSetSDKConfigSerializesApplies(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const ridA = "r-ser-00000000000000000001"
	const ridB = "r-ser-00000000000000000002"
	rideA := sdkConfigRide(t, "cid-sdk-ser000001", map[string]any{"rid": ridA, "model": "claude-sonnet-4-6"})
	rideB := sdkConfigRide(t, "cid-sdk-ser000002", map[string]any{"rid": ridB, "effort": "low"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, rideA, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("ride A: bad=%v fatal=%v", bad, fatal)
	}
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, rideB, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("ride B: bad=%v fatal=%v", bad, fatal)
	}

	if got := len(fwd.snapshot()); got != 1 {
		t.Fatalf("forwarded %d applies concurrently, want 1", got)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1 (B's refusal)", len(results))
	}
	if results[0]["rid"] != ridB || results[0]["ok"] != false || results[0]["code"] != "apply_in_progress" {
		t.Fatalf("B's answer = %v, want an apply_in_progress refusal", results[0])
	}

	// A still settles normally afterwards.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: ridA, OK: true, Model: "claude-sonnet-4-6"})
	after := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(after) != 1 || after[0]["rid"] != ridA || after[0]["ok"] != true {
		t.Fatalf("A's answer = %v", after)
	}
}

// TestHotClearMakesReselectApply is sol #4. Clearing a model deletes the env
// value, but the harness used to report the clear by OMITTING the field — and
// the box merged only non-empty values, so the old identity stuck. Two things
// went wrong from there: the app kept displaying a model the session was no
// longer running, and the box's no-op check kept believing that model was live,
// so re-selecting it was answered "already effective" and never applied.
func TestHotClearMakesReselectApply(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	// 1. Clear to the SDK default.
	const clearRid = "r-clear-000000000000000001"
	clear := sdkConfigRide(t, "cid-sdk-clear0001", map[string]any{"rid": clearRid, "model": ""})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, clear, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("clear ride: bad=%v fatal=%v", bad, fatal)
	}
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: clearRid, OK: true})
	if got := drainSDKResults(t, sub, 300*time.Millisecond); len(got) != 1 || got[0]["ok"] != true {
		t.Fatalf("clear result = %v", got)
	}

	// 2. The harness reports the clear the way it now does: PRESENCE, not
	//    omission. Model is explicitly "".
	empty := ""
	srv.MergeAgentInfo("claude-sdk", &empty, nil)

	if info := srv.currentAgentInfo(); info.Model != "" {
		t.Fatalf("the box still advertises %q after an explicit clear", info.Model)
	}
	if info := srv.currentAgentInfo(); !info.ModelKnown {
		t.Fatal("the cleared model is not marked as reported, so the config fallback will resurrect it")
	}

	// 3. Re-select the SAME model that was cleared. This must APPLY, not be
	//    dismissed as a no-op. The real env still carries the boot value, so a
	//    config fallback here would answer "already effective" and never
	//    forward — which is exactly the bug.
	t.Setenv("HOTLINE_SDK_MODEL", "claude-opus-4-8")
	const reRid = "r-clear-000000000000000002"
	reselect := sdkConfigRide(t, "cid-sdk-clear0002", map[string]any{"rid": reRid, "model": "claude-opus-4-8"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, reselect, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("reselect ride: bad=%v fatal=%v", bad, fatal)
	}

	calls := fwd.snapshot()
	if len(calls) != 2 {
		t.Fatalf("forwards = %d, want 2 — re-selecting a just-cleared model was mis-answered as a no-op", len(calls))
	}
	if deref(calls[1].Model) != "claude-opus-4-8" {
		t.Fatalf("second forward = %+v", calls[1])
	}
}

// TestMergeAgentInfoPresenceSemantics pins the three states directly.
func TestMergeAgentInfoPresenceSemantics(t *testing.T) {
	srv, _, _, _ := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "xhigh"})

	// nil = leave alone.
	srv.MergeAgentInfo("claude-sdk", nil, nil)
	if info := srv.currentAgentInfo(); info.Model != "claude-opus-4-8" || info.Effort != "xhigh" {
		t.Fatalf("a nil report changed the identity: %+v", info)
	}

	// pointer to "" = clear, and only the named field.
	empty := ""
	srv.MergeAgentInfo("claude-sdk", &empty, nil)
	info := srv.currentAgentInfo()
	if info.Model != "" || !info.ModelKnown {
		t.Fatalf("explicit clear did not land: %+v", info)
	}
	if info.Effort != "xhigh" {
		t.Fatalf("clearing model disturbed effort: %+v", info)
	}

	// a value = the value.
	sonnet := "claude-sonnet-4-6"
	srv.MergeAgentInfo("claude-sdk", &sonnet, nil)
	if got := srv.currentAgentInfo().Model; got != "claude-sonnet-4-6" {
		t.Fatalf("model = %q", got)
	}
}

// TestModelNoopIsDelimiterAware (sol review #2, box half). The no-op check's
// prefix tolerance exists for aliases — a bare "claude-opus-4-8" IS the live
// "claude-opus-4-8[1m]". Bare HasPrefix went much further: a request of "c"
// against a live "claude-sonnet-4-6" was answered "already effective", so the
// apply never ran and the app waited for an identity that would never change.
func TestModelNoopIsDelimiterAware(t *testing.T) {
	ptr := func(s string) *string { return &s }
	for _, tc := range []struct {
		req  string
		live string
		noop bool
		why  string
	}{
		{"claude-opus-4-8", "claude-opus-4-8", true, "exact"},
		{"claude-opus-4-8", "claude-opus-4-8[1m]", true, "context-tagged alias"},
		{"claude-haiku-4-5", "claude-haiku-4-5-20251001", true, "dated alias"},
		{"c", "claude-sonnet-4-6", false, "a one-character request is not that model"},
		{"claude-o", "claude-opus-4-8", false, "mid-token"},
		{"claude-opus-4-8", "claude-sonnet-4-6", false, "a different model"},
		{"openai-codex/gpt-5.6-sol", "openai-codex/gpt-5.6-sol", true, "pi canonical exact"},
	} {
		if got := sdkModelNoop(ptr(tc.req), tc.live); got != tc.noop {
			t.Errorf("sdkModelNoop(%q, live=%q) = %v, want %v — %s", tc.req, tc.live, got, tc.noop, tc.why)
		}
	}
	// The pointer semantics are unchanged.
	if !sdkModelNoop(nil, "anything") {
		t.Error("an omitted model is still a no-op")
	}
	if sdkModelNoop(ptr(""), "claude-opus-4-8") {
		t.Error("a clear against a set model is not a no-op")
	}
	if !sdkModelNoop(ptr(""), "") {
		t.Error("a clear against nothing configured is a no-op")
	}
}
