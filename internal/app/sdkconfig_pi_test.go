package app

// pi hot-apply amendment 2026-07-20: the set_sdk_config control on a PI box.
// Same frames, same hot loop, different knob family — these cases pin the
// per-harness half (charset, effort grammar, .env keys) and prove the
// claude-sdk surface did not move.

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// piEnvSandbox is sdkEnvSandbox for the pi knob family: an isolated state dir
// with every HOTLINE_PI_* key cleared out of the real environment (mergedEnv is
// real-env-wins, so a leaked key would shadow the .env under test).
func piEnvSandbox(t *testing.T) string {
	t.Helper()
	dir := sdkEnvSandbox(t)
	for _, k := range []string{"HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING", "HOTLINE_PI_PROVIDER"} {
		t.Setenv(k, "")
		if err := os.Unsetenv(k); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

// TestPiValidateSDKConfig: the pi knob family's acceptance surface, and its
// isolation from the claude-sdk one.
func TestPiValidateSDKConfig(t *testing.T) {
	cases := []struct {
		name    string
		harness string
		req     sdkConfigRequest
		want    string
	}{
		// Model charset: pi ids are provider-scoped, claude ids are not.
		{"pi takes a provider-scoped id", "pi", sdkConfigRequest{RID: "r", Model: sp("openai-codex/gpt-5.6-sol")}, ""},
		{"pi takes a bare id", "pi", sdkConfigRequest{RID: "r", Model: sp("gpt-5.6-sol")}, ""},
		{"pi takes a :level suffix", "pi", sdkConfigRequest{RID: "r", Model: sp("glm-5.2:high")}, ""},
		{"claude-sdk still refuses a slash", "claude-sdk", sdkConfigRequest{RID: "r", Model: sp("openai-codex/gpt-5.6-sol")}, "invalid_model"},
		{"pi refuses a leading slash", "pi", sdkConfigRequest{RID: "r", Model: sp("/gpt-5.6-sol")}, "invalid_model"},
		{"pi refuses shell metacharacters", "pi", sdkConfigRequest{RID: "r", Model: sp("gpt$(id)")}, "invalid_model"},

		// Effort: pi takes a thinking LEVEL, never a token budget.
		{"pi takes the shared five", "pi", sdkConfigRequest{RID: "r", Effort: sp("xhigh")}, ""},
		{"pi takes off (settable from its TUI)", "pi", sdkConfigRequest{RID: "r", Effort: sp("off")}, ""},
		{"pi takes minimal", "pi", sdkConfigRequest{RID: "r", Effort: sp("minimal")}, ""},
		{"pi REFUSES a raw token budget", "pi", sdkConfigRequest{RID: "r", Effort: sp("32000")}, "invalid_effort"},
		{"claude-sdk still takes a raw token budget", "claude-sdk", sdkConfigRequest{RID: "r", Effort: sp("32000")}, ""},
		{"claude-sdk still refuses off", "claude-sdk", sdkConfigRequest{RID: "r", Effort: sp("off")}, "invalid_effort"},

		// Clears and the empty request are harness-agnostic.
		{"pi clear model", "pi", sdkConfigRequest{RID: "r", Model: sp("")}, ""},
		{"pi clear effort", "pi", sdkConfigRequest{RID: "r", Effort: sp("")}, ""},
		{"pi empty request", "pi", sdkConfigRequest{RID: "r"}, "empty_request"},
	}
	for _, tc := range cases {
		knobs := knobsForTest(t, tc.harness)
		if code, _ := validateSDKConfig(tc.req, knobs); code != tc.want {
			t.Errorf("%s: code = %q, want %q", tc.name, code, tc.want)
		}
	}
}

// TestKnobsForCoverage: exactly the harnesses that bind the hot forwarder take
// remote knobs. If these two lists ever drift, a box either silently refuses a
// harness that can apply, or forwards to one that cannot answer.
func TestKnobsForCoverage(t *testing.T) {
	for _, h := range []string{"claude-sdk", "pi"} {
		if _, ok := knobsFor(h, ""); !ok {
			t.Errorf("harness %q binds the sdk_apply forwarder but has no knob family", h)
		}
	}
	for _, h := range []string{"claude", "opencode", "", "future-thing"} {
		if _, ok := knobsFor(h, ""); ok {
			t.Errorf("harness %q must not take remote model/effort", h)
		}
	}
}

// TestPiHotApplyLoop: the full pi hot loop through the real inbound dispatch.
// The request carries a BARE id and a level the model can't do; the harness
// answers with the canonical id and the CLAMPED level, and both of those — not
// the request — are what get persisted into the pi env family and echoed to the
// app. That echo is what lets the client's identity confirmation land.
func TestPiHotApplyLoop(t *testing.T) {
	envRoot := piEnvSandbox(t)
	// Pre-seed a box that was pinned the old way: an explicit provider line the
	// canonical model write must retire.
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("# box knobs\nHOTLINE_PI_PROVIDER=zai\nHOTLINE_PI_MODEL=glm-5.2\nHOTLINE_PI_THINKING=low\nHOTLINE_MC_CONTEXT_CAP=300000\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "pi", Model: "zai/glm-5.2", Effort: "low"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKC0PI000001"
	ride := sdkConfigRide(t, "cid-pi-hot0000001", map[string]any{"rid": rid, "model": "gpt-5.6-sol", "effort": "max"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("pi hot ride: bad=%v fatal=%v, want silent consume", bad, fatal)
	}

	// Forwarded and deferred — nothing answered, nothing persisted.
	if results := drainSDKResults(t, sub, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("pi hot ride answered %d results before the harness confirmed, want 0", len(results))
	}
	calls := fwd.snapshot()
	if len(calls) != 1 || calls[0].RID != rid || deref(calls[0].Model) != "gpt-5.6-sol" || deref(calls[0].Effort) != "max" {
		t.Fatalf("forward calls = %+v (model=%q effort=%q)", calls, deref(calls[0].Model), deref(calls[0].Effort))
	}

	// The harness confirms with what actually LANDED: the canonical provider/id
	// and the level pi clamped to.
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{
		RID: rid, OK: true, Model: "openai-codex/gpt-5.6-sol", Effort: "high",
	})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("post-confirm results = %d, want 1", len(results))
	}
	r := results[0]
	if r["rid"] != rid || r["ok"] != true || r["restart"] != false || r["hot"] != true {
		t.Fatalf("pi hot result = %v", r)
	}
	if r["model"] != "openai-codex/gpt-5.6-sol" {
		t.Errorf("result echoed %v, want the harness's canonical id — the client confirms against this", r["model"])
	}
	if r["effort"] != "high" {
		t.Errorf("result echoed %v, want the CLAMPED level; echoing the request would hang the client's confirm", r["effort"])
	}

	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	if !strings.Contains(env, "HOTLINE_PI_MODEL=openai-codex/gpt-5.6-sol") {
		t.Errorf("pi model not persisted canonically:\n%s", env)
	}
	if !strings.Contains(env, "HOTLINE_PI_THINKING=high") {
		t.Errorf("pi thinking not persisted as the clamped value:\n%s", env)
	}
	if strings.Contains(env, "HOTLINE_PI_PROVIDER") {
		t.Errorf("stale HOTLINE_PI_PROVIDER survived a model write; it would fight the canonical prefix at respawn:\n%s", env)
	}
	if strings.Contains(env, "HOTLINE_SDK_MODEL") || strings.Contains(env, "HOTLINE_SDK_EFFORT") {
		t.Errorf("a pi apply wrote into the claude-sdk env family:\n%s", env)
	}
	if !strings.Contains(env, "HOTLINE_MC_CONTEXT_CAP=300000") {
		t.Errorf("unrelated key lost:\n%s", env)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("pi hot apply filed a restart request (err=%v) — the wire must never drop", err)
	}
}

// TestPiHotNoOpAgainstLiveIdentity: the no-op check compares against the LIVE
// AgentInfo, which on a pi box is stamped by the extension's harness_info (the
// canonical id + the live thinking level). A request already effective must be
// answered ok without forwarding; a flip-back must NOT be mistaken for a no-op.
func TestPiHotNoOpAgainstLiveIdentity(t *testing.T) {
	envRoot := piEnvSandbox(t)
	// The BOOT config still says what `up` launched with. The live identity has
	// since moved (a TUI Ctrl+P, or an earlier hot apply). LoadPiModel must not
	// be what the no-op is judged against.
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_PI_MODEL=zai/glm-5.2\nHOTLINE_PI_THINKING=low\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	// harness_info from the pi extension: the operator switched in the TUI.
	srv.SetAgentInfo(AgentInfo{Harness: "pi", Model: "openai-codex/gpt-5.6-sol", Effort: "high"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	// (a) Asking for what is already live: answered ok, never forwarded.
	ride := sdkConfigRide(t, "cid-pi-noop000001", map[string]any{
		"rid": "01J2ZKC1PINOOP01", "model": "openai-codex/gpt-5.6-sol", "effort": "high",
	})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("pi no-op ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("pi no-op results = %d, want 1", len(results))
	}
	if r := results[0]; r["ok"] != true || r["restart"] != false || r["hot"] != nil {
		t.Fatalf("pi no-op result = %v, want a plain ok (no bounce, no hot wait)", r)
	}
	if calls := fwd.snapshot(); len(calls) != 0 {
		t.Fatalf("pi no-op forwarded %d applies, want 0", len(calls))
	}
	after, err := os.Stat(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !after.ModTime().Equal(before.ModTime()) || after.Size() != before.Size() {
		t.Error("pi no-op rewrote the .env")
	}

	// (b) The flip-back regression: asking for the BOOT value, which the live
	// identity has moved away from, is a real change and must forward.
	ride = sdkConfigRide(t, "cid-pi-flipback01", map[string]any{
		"rid": "01J2ZKC2PIFLIP01", "model": "zai/glm-5.2",
	})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("pi flip-back ride: bad=%v fatal=%v", bad, fatal)
	}
	if calls := fwd.snapshot(); len(calls) != 1 || deref(calls[0].Model) != "zai/glm-5.2" {
		t.Fatalf("pi flip-back forward calls = %+v; a flip back to the boot value must not read as a no-op", calls)
	}
}

// TestPiRestartFallbackUsesPiEnvFamily: a pi box whose wiring bound no
// forwarder (telegram-only, or a pre-amendment binary) keeps the persist →
// bounce path — and must persist into HOTLINE_PI_*, never the claude-sdk keys.
func TestPiRestartFallbackUsesPiEnvFamily(t *testing.T) {
	envRoot := piEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "pi", Model: "zai/glm-5.2", Effort: "low"})
	// No fwd.bind: sdkApplyForward stays nil.

	ride := sdkConfigRide(t, "cid-pi-restart001", map[string]any{
		"rid": "01J2ZKC3PIREST01", "model": "openai-codex/gpt-5.6-sol", "effort": "xhigh",
	})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("pi restart ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("pi restart results = %d, want 1", len(results))
	}
	if r := results[0]; r["ok"] != true || r["restart"] != true {
		t.Fatalf("pi restart result = %v, want ok+restart:true", r)
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	if !strings.Contains(env, "HOTLINE_PI_MODEL=openai-codex/gpt-5.6-sol") || !strings.Contains(env, "HOTLINE_PI_THINKING=xhigh") {
		t.Errorf("pi restart path persisted into the wrong keys:\n%s", env)
	}
	if strings.Contains(env, "HOTLINE_SDK_") {
		t.Errorf("pi restart path wrote claude-sdk keys:\n%s", env)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); err != nil {
		t.Errorf("pi restart path did not file a restart request: %v", err)
	}
}

// TestPiApplyFailureCodesReachTheApp: the pi-only no_api_key code must survive
// the box verbatim so the client can print copy the operator can act on.
func TestPiApplyFailureCodesReachTheApp(t *testing.T) {
	piEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "pi", Model: "zai/glm-5.2", Effort: "low"})
	fwd := &forwardRecorder{}
	fwd.bind(srv)

	const rid = "01J2ZKC4PIKEY001"
	ride := sdkConfigRide(t, "cid-pi-nokey00001", map[string]any{"rid": rid, "model": "openai-codex/gpt-5.6-sol"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("pi no-key ride: bad=%v fatal=%v", bad, fatal)
	}
	srv.handleSDKApplyResult(mcpchan.SDKApplyResultParams{
		RID: rid, OK: false, Code: "no_api_key", Detail: "no API key configured for openai-codex",
	})
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != false || r["code"] != "no_api_key" {
		t.Fatalf("result = %v, want the harness's no_api_key verbatim", r)
	}
	if r["detail"] != "no API key configured for openai-codex" {
		t.Errorf("detail = %v, want the harness line", r["detail"])
	}
}
