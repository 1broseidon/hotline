package app

// SDK hot-model amendment 2026-07-19 (fixture SC6-SC9): the model-only hot
// path — routing, the live-identity no-op check (incl. the flip-back
// regression), persist-after-ok ordering, pending timeout, forward failure,
// and effort keeping the restart path.

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// forwardRecorder is a bindable sdkApplyForward that records calls and can
// fail on demand.
type forwardRecorder struct {
	mu    sync.Mutex
	calls []mcpchan.SDKApplyParams
	err   error
}

func (f *forwardRecorder) bind(srv *Server) {
	srv.sdkApplyForward = func(_ context.Context, rid string, model, effort *string) error {
		f.mu.Lock()
		defer f.mu.Unlock()
		if f.err != nil {
			return f.err
		}
		f.calls = append(f.calls, mcpchan.SDKApplyParams{RID: rid, Model: model, Effort: effort})
		return nil
	}
}

// deref renders an optional forwarded field for assertions: "<nil>" = the field
// was omitted (unchanged), "" = an explicit clear.
func deref(p *string) string {
	if p == nil {
		return "<nil>"
	}
	return *p
}

func (f *forwardRecorder) snapshot() []mcpchan.SDKApplyParams {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]mcpchan.SDKApplyParams(nil), f.calls...)
}

func pendingCount(srv *Server) int {
	srv.sdkMu.Lock()
	defer srv.sdkMu.Unlock()
	return len(srv.sdkPending)
}

// TestSetSDKConfigHotApplyLoop drives the full hot loop through the real
// inbound dispatch: the model-only ride is consumed silently and answered by
// NOTHING until the harness confirms (persist-after-ok, SC7) — no supervisor
// bounce ever happens (SC6). The confirmation resolves the pending apply:
// .env is written only then, and the requesting device gets the deferred
// ok+restart:false+hot:true result. A duplicate/late result is dropped.
func TestSetSDKConfigHotApplyLoop(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "xhigh"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB0HOT00001"
	ride := sdkConfigRide(t, "cid-sdk-hot000001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("hot ride: bad=%v fatal=%v, want silent consume", bad, fatal)
	}

	// Forwarded, deferred: no result yet, nothing persisted, nothing bounced.
	if results := drainSDKResults(t, sub, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("hot ride answered %d results before the harness confirmed, want 0", len(results))
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || calls[0].RID != rid || deref(calls[0].Model) != "claude-sonnet-4-6" || calls[0].Effort != nil {
		t.Fatalf("forward calls = %+v (model=%q effort=%q)", calls, deref(calls[0].Model), deref(calls[0].Effort))
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-opus-4-8") {
		t.Fatalf(".env moved before the harness confirmed (SC7):\n%s", raw)
	}
	if pendingCount(srv) != 1 {
		t.Fatalf("pending = %d, want 1", pendingCount(srv))
	}

	// The harness confirms: persist NOW, then the deferred hot result.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("post-confirm results = %d, want 1", len(results))
	}
	r := results[0]
	if r["rid"] != rid || r["ok"] != true || r["restart"] != false || r["hot"] != true || r["model"] != "claude-sonnet-4-6" {
		t.Fatalf("hot result = %v", r)
	}
	if _, present := r["effort"]; present {
		t.Fatalf("hot result echoed effort the request did not carry: %v", r)
	}
	raw, err = os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	if !strings.Contains(env, "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Errorf(".env not persisted after confirm:\n%s", env)
	}
	if !strings.Contains(env, "HOTLINE_SDK_EFFORT=xhigh") {
		t.Errorf("hot model apply disturbed the effort knob:\n%s", env)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("hot apply filed a restart request (err=%v) — the wire must never drop", err)
	}
	if pendingCount(srv) != 0 {
		t.Fatalf("pending not cleared after confirm: %d", pendingCount(srv))
	}

	// Duplicate/late result for the consumed rid: dropped, no second frame.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	if results := drainSDKResults(t, sub, 200*time.Millisecond); len(results) != 0 {
		t.Fatalf("duplicate result answered %d frames, want 0", len(results))
	}
}

// TestSetSDKConfigHotClear: a clear ("") is never a live-identity no-op — it
// forwards, and the confirmed persist removes the .env line while the result
// echoes "" explicitly.
func TestSetSDKConfigHotClear(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB2CLR00001"
	ride := sdkConfigRide(t, "cid-sdk-hotclear1", map[string]any{"rid": rid, "model": ""})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("clear ride: bad=%v fatal=%v", bad, fatal)
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || deref(calls[0].Model) != "" {
		t.Fatalf("forward calls = %+v, want one clear", calls)
	}

	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	model, present := r["model"]
	if r["ok"] != true || r["hot"] != true || !present || model != "" {
		t.Fatalf("clear result = %v, want hot ok with explicit empty model echo", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "HOTLINE_SDK_MODEL") {
		t.Errorf("model line survived the confirmed clear:\n%s", raw)
	}
}

// TestSetSDKConfigHotNoOp: the no-op check reads the LIVE identity — exact
// match, the client's resolved-prefix tolerance, and the LoadSDK fallback
// when no model was ever reported. A no-op answers ok+restart:false with NO
// hot marker and never forwards.
func TestSetSDKConfigHotNoOp(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cases := []struct {
		name  string
		live  string // AgentInfo model ("" = unreported, LoadSDK fallback)
		model string
	}{
		{"exact live match", "claude-sonnet-4-6", "claude-sonnet-4-6"},
		{"resolved-prefix tolerance", "claude-sonnet-4-6-20250929", "claude-sonnet-4-6"},
		{"LoadSDK fallback when identity has no model", "", "claude-opus-4-8"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			srv, _, dev, sub := activeHarness(t)
			srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: tc.live})
			fwd := &forwardRecorder{}
			fwd.bind(srv)

			ride := sdkConfigRide(t, "cid-sdk-hotnoop01", map[string]any{"rid": "01J2ZKB3NOP00001", "model": tc.model})
			if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
				t.Fatalf("no-op ride: bad=%v fatal=%v", bad, fatal)
			}
			results := drainSDKResults(t, sub, 300*time.Millisecond)
			if len(results) != 1 {
				t.Fatalf("results = %d, want 1", len(results))
			}
			r := results[0]
			if r["ok"] != true || r["restart"] != false || r["model"] != tc.model {
				t.Fatalf("no-op result = %v", r)
			}
			if _, present := r["hot"]; present {
				t.Fatalf("no-op result carries hot: %v (unchanged shape required)", r)
			}
			if calls := fwd.snapshot(); len(calls) != 0 {
				t.Fatalf("no-op forwarded %d applies, want 0", len(calls))
			}
		})
	}
}

// TestSetSDKConfigHotFlipBackRegression: after a hot apply the .env (and the
// child's real env) still carry the BOOT model, so a LoadSDK-based no-op
// check would mis-answer a flip-back as a no-op. The live-identity check must
// forward it.
func TestSetSDKConfigHotFlipBackRegression(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	// Boot model: opus (what `up` exported and .env still says post-hot-apply).
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	// Live identity: sonnet (harness_info restamp after an earlier hot apply).
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-sonnet-4-6"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	// Flip back to the boot model: LoadSDK says it's current — the live
	// session disagrees, so this MUST forward, not no-op.
	ride := sdkConfigRide(t, "cid-sdk-hotflip01", map[string]any{"rid": "01J2ZKB4FLP00001", "model": "claude-opus-4-8"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("flip-back ride: bad=%v fatal=%v", bad, fatal)
	}
	if results := drainSDKResults(t, sub, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("flip-back mis-answered as an immediate result: %v", results)
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || deref(calls[0].Model) != "claude-opus-4-8" {
		t.Fatalf("flip-back forward calls = %+v, want the opus apply", calls)
	}
}

// TestSetSDKConfigHotEffort (effort hot amendment): an effort-only request on
// a hot-capable box hot-applies to the LIVE session exactly like model —
// forwarded (model nil, effort set), deferred, persisted AFTER the harness
// confirms, answered ok+restart:false+hot:true with only the effort echoed, no
// bounce, and the model knob left untouched in .env.
func TestSetSDKConfigHotEffort(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=high\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "high"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB5EFF00001"
	ride := sdkConfigRide(t, "cid-sdk-hoteff001", map[string]any{"rid": rid, "effort": "xhigh"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("effort ride: bad=%v fatal=%v", bad, fatal)
	}
	// Deferred: no result, nothing persisted, forwarded with model nil.
	if results := drainSDKResults(t, sub, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("effort ride answered %d results before confirm, want 0", len(results))
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || calls[0].Model != nil || deref(calls[0].Effort) != "xhigh" {
		t.Fatalf("forward calls = %+v (model=%q effort=%q)", calls, deref(calls[0].Model), deref(calls[0].Effort))
	}

	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Effort: "xhigh"})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("post-confirm results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != true || r["restart"] != false || r["hot"] != true || r["effort"] != "xhigh" {
		t.Fatalf("effort hot result = %v", r)
	}
	if _, present := r["model"]; present {
		t.Fatalf("effort-only result echoed model: %v", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	if !strings.Contains(env, "HOTLINE_SDK_EFFORT=xhigh") {
		t.Errorf("effort not persisted after confirm:\n%s", env)
	}
	if !strings.Contains(env, "HOTLINE_SDK_MODEL=claude-opus-4-8") {
		t.Errorf("hot effort apply disturbed the model knob:\n%s", env)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("hot effort apply filed a restart request (err=%v)", err)
	}
}

// TestSetSDKConfigHotCombined: a combined model+effort request forwards both,
// and one confirmation persists both and answers a single hot result echoing
// both accepted values.
func TestSetSDKConfigHotCombined(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=high\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "high"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB5CMB00001"
	ride := sdkConfigRide(t, "cid-sdk-hotcmb001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6", "effort": "xhigh"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("combined ride: bad=%v fatal=%v", bad, fatal)
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || deref(calls[0].Model) != "claude-sonnet-4-6" || deref(calls[0].Effort) != "xhigh" {
		t.Fatalf("combined forward = %+v", calls)
	}

	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6", Effort: "xhigh"})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != true || r["hot"] != true || r["restart"] != false || r["model"] != "claude-sonnet-4-6" || r["effort"] != "xhigh" {
		t.Fatalf("combined result = %v", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	if !strings.Contains(env, "HOTLINE_SDK_MODEL=claude-sonnet-4-6") || !strings.Contains(env, "HOTLINE_SDK_EFFORT=xhigh") {
		t.Errorf("combined not fully persisted:\n%s", env)
	}
}

// TestSetSDKConfigHotEffortClear: an effort clear ("") forwards a
// pointer-to-"" and, on confirm, removes the effort line while echoing "".
func TestSetSDKConfigHotEffortClear(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_EFFORT=high\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "high"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB5ECL00001"
	ride := sdkConfigRide(t, "cid-sdk-hotecl001", map[string]any{"rid": rid, "effort": ""})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("effort-clear ride: bad=%v fatal=%v", bad, fatal)
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || calls[0].Effort == nil || deref(calls[0].Effort) != "" || calls[0].Model != nil {
		t.Fatalf("effort-clear forward = %+v", calls)
	}

	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Effort: ""})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	effort, present := r["effort"]
	if r["ok"] != true || r["hot"] != true || !present || effort != "" {
		t.Fatalf("effort-clear result = %v, want hot ok with explicit empty effort echo", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "HOTLINE_SDK_EFFORT") {
		t.Errorf("effort line survived the confirmed clear:\n%s", raw)
	}
}

// TestSetSDKConfigEffortWithoutForwarder: a claude-sdk box whose wiring never
// bound the forwarder keeps the restart path for an effort request too (SC9) —
// persisted immediately, bounce requested, restart:true, no hot marker.
func TestSetSDKConfigEffortWithoutForwarder(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "high"})
	// No forwarder bound.

	ride := sdkConfigRide(t, "cid-sdk-nofweff01", map[string]any{"rid": "01J2ZKBANFE00001", "effort": "xhigh"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("no-forwarder effort ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != true || r["restart"] != true || r["effort"] != "xhigh" {
		t.Fatalf("no-forwarder effort result = %v", r)
	}
	if _, present := r["hot"]; present {
		t.Fatalf("restart-path result carries hot: %v", r)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); err != nil {
		t.Fatalf("restart path did not bounce: %v", err)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_EFFORT=xhigh") {
		t.Errorf("restart path did not persist effort:\n%s", raw)
	}
}

// TestSetSDKConfigHotForwardError: an unreachable harness (forward error)
// answers harness_unreachable immediately, unregisters the pending apply,
// and leaves .env untouched — explicit error, never a silent restart
// fallback.
func TestSetSDKConfigHotForwardError(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{err: os.ErrClosed}
	fwd.bind(srv)

	ride := sdkConfigRide(t, "cid-sdk-hoterr001", map[string]any{"rid": "01J2ZKB6ERR00001", "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("forward-error ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != false || r["code"] != "harness_unreachable" || r["detail"] != "couldn't reach the agent; try again" {
		t.Fatalf("forward-error result = %v", r)
	}
	if pendingCount(srv) != 0 {
		t.Fatalf("pending not unwound after forward error: %d", pendingCount(srv))
	}
	if _, err := os.Stat(filepath.Join(envRoot, ".env")); !os.IsNotExist(err) {
		t.Errorf("forward error touched the .env (err=%v)", err)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("forward error degraded to a restart (err=%v)", err)
	}
}

// TestSetSDKConfigHotTimeout: a harness that never confirms trips the pending
// timer (harness_unreachable), and a LATE ok after expiry is dropped without
// persisting — .env only ever moves on an answered confirmation.
func TestSetSDKConfigHotTimeout(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	srv.sdkPendingTimeout = 50 * time.Millisecond
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB7TMO00001"
	ride := sdkConfigRide(t, "cid-sdk-hottmo001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("timeout ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, time.Second)
	if len(results) != 1 {
		t.Fatalf("timeout results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != false || r["code"] != "harness_unreachable" || r["detail"] != "the agent didn't confirm the change" {
		t.Fatalf("timeout result = %v", r)
	}
	if pendingCount(srv) != 0 {
		t.Fatalf("pending survived expiry: %d", pendingCount(srv))
	}

	// Late ok: dropped, nothing persisted, no second frame.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	if late := drainSDKResults(t, sub, 200*time.Millisecond); len(late) != 0 {
		t.Fatalf("late result answered %d frames, want 0", len(late))
	}
	if _, err := os.Stat(filepath.Join(envRoot, ".env")); !os.IsNotExist(err) {
		t.Errorf("late-ok persisted after expiry (err=%v)", err)
	}
}

// TestSetSDKConfigHotHarnessFailures: the harness's failure codes
// (unknown_model / apply_failed / no_session, plus a defensive empty-code
// default) forward straight into the result frame; nothing persists.
func TestSetSDKConfigHotHarnessFailures(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	cases := []struct {
		rid        string
		code       string // harness-sent
		detail     string
		wantCode   string
		wantDetail string
	}{
		{"01J2ZKB8FLA00001", "unknown_model", "model not in the CLI's supported list", "unknown_model", "model not in the CLI's supported list"},
		{"01J2ZKB8FLA00002", "apply_failed", "control request rejected", "apply_failed", "control request rejected"},
		{"01J2ZKB8FLA00003", "no_session", "SDK session not running", "no_session", "SDK session not running"},
		{"01J2ZKB8FLA00004", "", "who knows", "apply_failed", "who knows"},
	}
	for _, tc := range cases {
		ride := sdkConfigRide(t, "cid-sdk-hotfla", map[string]any{"rid": tc.rid, "model": "claude-sonnet-4-6"})
		if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
			t.Fatalf("%s ride: bad=%v fatal=%v", tc.rid, bad, fatal)
		}
		srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: tc.rid, OK: false, Code: tc.code, Detail: tc.detail})
		results := drainSDKResults(t, sub, 300*time.Millisecond)
		if len(results) != 1 {
			t.Fatalf("%s results = %d, want 1", tc.rid, len(results))
		}
		r := results[0]
		if r["ok"] != false || r["code"] != tc.wantCode || r["detail"] != tc.wantDetail {
			t.Fatalf("%s result = %v (want code=%s detail=%q)", tc.rid, r, tc.wantCode, tc.wantDetail)
		}
	}
	if _, err := os.Stat(filepath.Join(envRoot, ".env")); !os.IsNotExist(err) {
		t.Errorf("harness failures touched the .env (err=%v)", err)
	}
}

// TestSetSDKConfigHotPersistFailed: the split state — applied to the live
// session but the .env write fails — surfaces the honest persist_failed
// detail instead of a fake success.
func TestSetSDKConfigHotPersistFailed(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKB9PSF00001"
	ride := sdkConfigRide(t, "cid-sdk-hotpsf001", map[string]any{"rid": rid, "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("persist-fail ride: bad=%v fatal=%v", bad, fatal)
	}

	// Make the state root unwritable so UpdateSDKEnv's write fails; readable
	// so the earlier LoadSDK fallback path stays healthy elsewhere.
	if err := os.Chmod(envRoot, 0o500); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(envRoot, 0o700) })

	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{RID: rid, OK: true, Model: "claude-sonnet-4-6"})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != false || r["code"] != "persist_failed" {
		t.Fatalf("persist-fail result = %v", r)
	}
	if r["detail"] != "applied to the live session but not saved; a restart will revert it" {
		t.Errorf("persist-fail detail = %q", r["detail"])
	}
}

// TestSetSDKConfigModelOnlyWithoutForwarder: a claude-sdk box whose wiring
// never bound the forwarder (telegram-only, older binary behavior) keeps the
// f459366 restart path for model-only requests (SC9).
func TestSetSDKConfigModelOnlyWithoutForwarder(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	// No forwarder bound.

	ride := sdkConfigRide(t, "cid-sdk-nofwd0001", map[string]any{"rid": "01J2ZKBANFW00001", "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("no-forwarder ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != true || r["restart"] != true || r["model"] != "claude-sonnet-4-6" {
		t.Fatalf("no-forwarder result = %v", r)
	}
	if _, present := r["hot"]; present {
		t.Fatalf("restart-path result carries hot: %v", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Errorf("restart path did not persist:\n%s", raw)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); err != nil {
		t.Fatalf("restart path did not bounce: %v", err)
	}
}

// TestStopSDKPendingSweeps: Run-exit cleanup stops and clears every pending
// timer so a stopped server never answers again.
func TestStopSDKPendingSweeps(t *testing.T) {
	sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	srv.sdkPendingTimeout = 100 * time.Millisecond
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	ride := sdkConfigRide(t, "cid-sdk-hotswp001", map[string]any{"rid": "01J2ZKBBSWP00001", "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("sweep ride: bad=%v fatal=%v", bad, fatal)
	}
	if pendingCount(srv) != 1 {
		t.Fatalf("pending = %d, want 1", pendingCount(srv))
	}
	srv.stopSDKPending()
	if pendingCount(srv) != 0 {
		t.Fatalf("pending survived the sweep: %d", pendingCount(srv))
	}
	// The stopped timer never fires a result.
	if results := drainSDKResults(t, sub, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("swept pending still answered %d frames", len(results))
	}
}
