package app

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// sdkKnobs / piKnobs fetch a harness knob family for the validation cases. The
// pre-pi tests all assert the claude-sdk surface, so they take sdkKnobs.
func sdkKnobs(t *testing.T) harnessKnobs { return knobsForTest(t, "claude-sdk") }
func piKnobs(t *testing.T) harnessKnobs  { return knobsForTest(t, "pi") }

func knobsForTest(t *testing.T, harness string) harnessKnobs {
	t.Helper()
	k, ok := knobsFor(harness, "")
	if !ok {
		t.Fatalf("no knob family for harness %q", harness)
	}
	return k
}

// sdkConfigRide builds the set_sdk_config control frame the way the app sends
// it: a device_send whose "send" text payload is the serialized control line.
// fields go into the inner object next to t=set_sdk_config.
func sdkConfigRide(t *testing.T, cid string, fields map[string]any) []byte {
	t.Helper()
	m := map[string]any{"t": "set_sdk_config"}
	for k, v := range fields {
		m[k] = v
	}
	inner, err := json.Marshal(m)
	if err != nil {
		t.Fatalf("marshal set_sdk_config: %v", err)
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

// sdkEnvSandbox points the shared state dir (the .env home LoadSDK and
// UpdateSDKEnv resolve) at a temp root and truly unsets every SDK knob so
// mergedEnv's real-env-wins can't leak the developer's environment into the
// test (mirrors internal/config's sdkTestState).
func sdkEnvSandbox(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	for _, k := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_CLAUDE_SDK_MODEL"} {
		t.Setenv(k, "")
		if err := os.Unsetenv(k); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func sp(s string) *string { return &s }

// drainSDKResults filters a transient drain down to sdk_config_result frames.
func drainSDKResults(t *testing.T, sub *mailboxSubscriber, wait time.Duration) []map[string]any {
	t.Helper()
	var out []map[string]any
	for _, m := range drainTransients(t, sub, wait) {
		if m["t"] == "sdk_config_result" {
			out = append(out, m)
		}
	}
	return out
}

// TestParseSetSDKConfigRecognition: the parser's consume-vs-flow-on boundary
// (SC1) and its normalization (post-trim model, lowercased effort).
func TestParseSetSDKConfigRecognition(t *testing.T) {
	// Not controls: flow on to the harness untouched.
	for _, text := range []string{
		"hello there",
		"{\"t\":\"set_name\",\"name\":\"x\"}",
		"{\"t\":\"send\",\"text\":\"hi\"}",
		"  not json {",
		"",
	} {
		if _, isControl := parseSetSDKConfig(text); isControl {
			t.Errorf("parseSetSDKConfig(%q) claimed control", text)
		}
	}

	// A recognized control with malformed fields is STILL consumed (never
	// leaks to the harness); the zero RID makes the handler drop it.
	req, isControl := parseSetSDKConfig(`{"t":"set_sdk_config","rid":7,"model":"x"}`)
	if !isControl {
		t.Fatal("malformed rid: control not consumed")
	}
	if req.RID != "" {
		t.Errorf("malformed control rid = %q, want zero", req.RID)
	}

	// Well-formed: pointer presence + normalization.
	req, isControl = parseSetSDKConfig(`{"t":"set_sdk_config","rid":"01J2ZKA0M3QW8XY7","model":" claude-sonnet-4-6 ","effort":" HIGH "}`)
	if !isControl {
		t.Fatal("well-formed control not recognized")
	}
	if req.RID != "01J2ZKA0M3QW8XY7" || req.Model == nil || *req.Model != "claude-sonnet-4-6" || req.Effort == nil || *req.Effort != "high" {
		t.Errorf("req = %+v (model=%v effort=%v)", req, req.Model, req.Effort)
	}

	// Absent vs explicit-clear survive parsing.
	req, _ = parseSetSDKConfig(`{"t":"set_sdk_config","rid":"r1","model":""}`)
	if req.Model == nil || *req.Model != "" || req.Effort != nil {
		t.Errorf("clear-model req = %+v", req)
	}
}

// TestSetSDKConfigFromDeviceSendBoundary: only a device_send whose "send"
// text is the control matches; other frames and other controls flow on.
func TestSetSDKConfigFromDeviceSendBoundary(t *testing.T) {
	if _, isControl := setSDKConfigFromDeviceSend(sdkConfigRide(t, "cid-sdk-ride00001", map[string]any{"rid": "r1", "model": "m"})); !isControl {
		t.Error("real ride not recognized")
	}
	if _, isControl := setSDKConfigFromDeviceSend(setNameRide(t, "cid-name-ride0001", "Wendigo")); isControl {
		t.Error("set_name ride misclaimed as sdk config")
	}
	plain, _ := json.Marshal(map[string]any{"t": "device_send", "cid": "c", "payload": map[string]any{"t": "send", "text": "plain message"}})
	if _, isControl := setSDKConfigFromDeviceSend(plain); isControl {
		t.Error("plain send misclaimed")
	}
	if _, isControl := setSDKConfigFromDeviceSend([]byte("not json")); isControl {
		t.Error("garbage misclaimed")
	}
}

// TestValidateSDKConfig: every SC2 code plus the boundary lengths, numeric
// efforts, and the caps shared with LoadSDK.
func TestValidateSDKConfig(t *testing.T) {
	cases := []struct {
		name string
		req  sdkConfigRequest
		want string
	}{
		{"empty request", sdkConfigRequest{RID: "r"}, "empty_request"},
		{"model ok", sdkConfigRequest{RID: "r", Model: sp("claude-sonnet-4-6")}, ""},
		{"model clear ok", sdkConfigRequest{RID: "r", Model: sp("")}, ""},
		{"model 64 ok", sdkConfigRequest{RID: "r", Model: sp("a" + strings.Repeat("b", 63))}, ""},
		// A tapped catalog id is ModelInfo.value verbatim, which carries the
		// context-window suffix in square brackets ("claude-opus-4-8[1m]"). The
		// box must accept its own advertised menu, so brackets are in the charset.
		{"model catalog bracket suffix ok", sdkConfigRequest{RID: "r", Model: sp("claude-opus-4-8[1m]")}, ""},
		{"model 65 too long", sdkConfigRequest{RID: "r", Model: sp("a" + strings.Repeat("b", 64))}, "invalid_model"},
		{"model bad lead", sdkConfigRequest{RID: "r", Model: sp("-claude")}, "invalid_model"},
		{"model bad chars", sdkConfigRequest{RID: "r", Model: sp("not a model!")}, "invalid_model"},
		{"model env hostile", sdkConfigRequest{RID: "r", Model: sp("a\nb")}, "invalid_model"},
		{"effort name ok", sdkConfigRequest{RID: "r", Effort: sp("xhigh")}, ""},
		{"effort clear ok", sdkConfigRequest{RID: "r", Effort: sp("")}, ""},
		{"effort numeric ok", sdkConfigRequest{RID: "r", Effort: sp("12000")}, ""},
		{"effort bad name", sdkConfigRequest{RID: "r", Effort: sp("xtreme")}, "invalid_effort"},
		{"effort zero", sdkConfigRequest{RID: "r", Effort: sp("0")}, "invalid_effort"},
		{"effort negative", sdkConfigRequest{RID: "r", Effort: sp("-5")}, "invalid_effort"},
		{"effort 16 ok", sdkConfigRequest{RID: "r", Effort: sp("1234567890123456")}, ""},
		{"effort 17 too long", sdkConfigRequest{RID: "r", Effort: sp("12345678901234567")}, "invalid_effort"},
		{"both ok", sdkConfigRequest{RID: "r", Model: sp("claude-opus-4-8"), Effort: sp("max")}, ""},
		{"model checked first", sdkConfigRequest{RID: "r", Model: sp("bad!"), Effort: sp("bad")}, "invalid_model"},
	}
	for _, tc := range cases {
		if code, _ := validateSDKConfig(tc.req, sdkKnobs(t)); code != tc.want {
			t.Errorf("%s: code = %q, want %q", tc.name, code, tc.want)
		}
	}
}

// TestSDKConfigResultFrameGoldens: the frame builder's byte shapes — ok with
// echo, cleared-echo "" (must serialize, not be omitted), ok no-op, and both
// error forms.
func TestSDKConfigResultFrameGoldens(t *testing.T) {
	cases := []struct {
		name string
		got  []byte
		want string
	}{
		{
			"ok with echo",
			sdkConfigResultFrame(813, "r1", true, sp("claude-sonnet-4-6"), sp("high"), true, false, false, "", ""),
			`{"effort":"high","model":"claude-sonnet-4-6","ok":true,"restart":true,"rid":"r1","seq":813,"t":"sdk_config_result"}`,
		},
		{
			"cleared echo serializes empty strings",
			sdkConfigResultFrame(814, "r2", true, sp(""), nil, true, false, false, "", ""),
			`{"model":"","ok":true,"restart":true,"rid":"r2","seq":814,"t":"sdk_config_result"}`,
		},
		{
			"no-op",
			sdkConfigResultFrame(815, "r3", true, sp("claude-opus-4-8"), nil, false, false, false, "", ""),
			`{"model":"claude-opus-4-8","ok":true,"restart":false,"rid":"r3","seq":815,"t":"sdk_config_result"}`,
		},
		{
			"error with detail",
			sdkConfigResultFrame(816, "r4", false, nil, nil, false, false, false, "invalid_model", "model must be 1-64 chars: alphanumeric lead, then [A-Za-z0-9._:[]-]"),
			`{"code":"invalid_model","detail":"model must be 1-64 chars: alphanumeric lead, then [A-Za-z0-9._:[]-]","ok":false,"rid":"r4","seq":816,"t":"sdk_config_result"}`,
		},
		{
			"error without detail",
			sdkConfigResultFrame(817, "r5", false, nil, nil, false, false, false, "empty_request", ""),
			`{"code":"empty_request","ok":false,"rid":"r5","seq":817,"t":"sdk_config_result"}`,
		},
		{
			"hot success carries hot:true with restart:false",
			sdkConfigResultFrame(818, "r6", true, sp("claude-sonnet-4-6"), nil, false, true, false, "", ""),
			`{"hot":true,"model":"claude-sonnet-4-6","ok":true,"restart":false,"rid":"r6","seq":818,"t":"sdk_config_result"}`,
		},
		{
			"hot never serializes on failure",
			sdkConfigResultFrame(819, "r7", false, nil, nil, false, true, false, "unknown_model", "model not in the CLI's supported list"),
			`{"code":"unknown_model","detail":"model not in the CLI's supported list","ok":false,"rid":"r7","seq":819,"t":"sdk_config_result"}`,
		},
		{
			"unverified hot success carries unverified:true alongside hot:true",
			sdkConfigResultFrame(820, "r8", true, sp("claude-opus-4-6"), nil, false, true, true, "", ""),
			`{"hot":true,"model":"claude-opus-4-6","ok":true,"restart":false,"rid":"r8","seq":820,"t":"sdk_config_result","unverified":true}`,
		},
		{
			"unverified never serializes on failure",
			sdkConfigResultFrame(821, "r9", false, nil, nil, false, true, true, "apply_failed", "boom"),
			`{"code":"apply_failed","detail":"boom","ok":false,"rid":"r9","seq":821,"t":"sdk_config_result"}`,
		},
	}
	// encoding/json sorts map keys, so the golden strings are deterministic.
	for _, tc := range cases {
		if string(tc.got) != tc.want {
			t.Errorf("%s:\n got %s\nwant %s", tc.name, tc.got, tc.want)
		}
	}
}

// TestSetSDKConfigApplyLoop drives the full accepted path through the real
// inbound dispatch (setname_test style): the ride is consumed silently (no
// error frame, no journal event, never reaches the harness), the knobs
// persist into the shared .env (legacy HOTLINE_CLAUDE_SDK_MODEL dropped), a
// restart.request lands in the supervisor dir carrying the rid, and the
// requesting device — and ONLY the requesting device — receives the
// rid-correlated ok+restart:true result with post-trim echoes.
func TestSetSDKConfigApplyLoop(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	// Pre-seed the .env the way Umibozu's looks: legacy model line + comment.
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("# box knobs\nHOTLINE_CLAUDE_SDK_MODEL=claude-legacy\nHOTLINE_SDK_MAX_TURNS=40\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-legacy", Effort: "xhigh"})

	// Second active device: the result must NOT reach it.
	const dev2 = "dev-testdevice02"
	srv.store.st.Devices[dev2] = DeviceRecord{ID: dev2, Room: "room1", SecretHash: "x", State: DeviceActive}
	srv.store.mu.Lock()
	_ = srv.store.saveLocked()
	srv.store.mu.Unlock()
	srv.mailbox.disk.Devices[dev2] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	_, _, _, sub2, err := srv.mailbox.stateAndSubscribe(dev2)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.mailbox.unsubscribe(dev2, sub2) })

	ctx := context.Background()
	var writes [][]byte
	write := func(b []byte) error { writes = append(writes, b); return nil }

	// Model + effort in one ride; effort arrives denormalized to prove the
	// accepted post-trim echo.
	ride := sdkConfigRide(t, "cid-sdk-apply0001", map[string]any{"rid": "01J2ZKA0M3QW8XY7", "model": "claude-sonnet-4-6", "effort": " HIGH "})
	if bad, fatal := srv.handleSessionInput(ctx, dev, sub, ride, write); bad || fatal {
		t.Fatalf("apply ride: bad=%v fatal=%v, want silent consume", bad, fatal)
	}
	if len(writes) != 0 {
		t.Fatalf("apply ride wrote %d frames on the session path, want 0 (results ride transient)", len(writes))
	}
	if items := drainItems(t, sub); len(items) != 0 {
		t.Fatalf("apply ride journaled %d durable items, want 0 (never a transcript event)", len(items))
	}

	// Result: rid-correlated ok+restart:true with post-trim echoes, to the
	// requesting device only.
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("requesting device got %d sdk_config_result frames, want 1", len(results))
	}
	r := results[0]
	if r["rid"] != "01J2ZKA0M3QW8XY7" || r["ok"] != true || r["restart"] != true || r["model"] != "claude-sonnet-4-6" || r["effort"] != "high" {
		t.Fatalf("result = %v", r)
	}
	if _, hasSeq := r["seq"]; !hasSeq {
		t.Fatalf("result missing seq (transient family frames are seq'd): %v", r)
	}
	if other := drainSDKResults(t, sub2, 300*time.Millisecond); len(other) != 0 {
		t.Fatalf("non-requesting device received %d sdk_config_result frames, want 0", len(other))
	}

	// Persistence: canonical keys written, legacy line dropped, comment and
	// unrelated knob preserved.
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	env := string(raw)
	for _, want := range []string{"HOTLINE_SDK_MODEL=claude-sonnet-4-6", "HOTLINE_SDK_EFFORT=high", "# box knobs", "HOTLINE_SDK_MAX_TURNS=40"} {
		if !strings.Contains(env, want) {
			t.Errorf(".env missing %q:\n%s", want, env)
		}
	}
	if strings.Contains(env, "HOTLINE_CLAUDE_SDK_MODEL") {
		t.Errorf("legacy model line survived the model set:\n%s", env)
	}

	// Restart control: the supervisor poll file exists and names the rid.
	reqRaw, err := os.ReadFile(filepath.Join(srv.supervisorDir, "restart.request"))
	if err != nil {
		t.Fatalf("restart.request not written: %v", err)
	}
	if !strings.Contains(string(reqRaw), "sdk config change from app (rid 01J2ZKA0M3QW8XY7)") {
		t.Errorf("restart reason = %q", reqRaw)
	}

	// Clear ride: model "" removes the line and echoes "" explicitly.
	if err := os.Remove(filepath.Join(srv.supervisorDir, "restart.request")); err != nil {
		t.Fatal(err)
	}
	ride = sdkConfigRide(t, "cid-sdk-clear0001", map[string]any{"rid": "01J2ZKA1CLEAR001", "model": ""})
	if bad, fatal := srv.handleSessionInput(ctx, dev, sub, ride, write); bad || fatal {
		t.Fatalf("clear ride: bad=%v fatal=%v", bad, fatal)
	}
	results = drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("clear ride results = %d, want 1", len(results))
	}
	r = results[0]
	model, present := r["model"]
	if r["ok"] != true || r["restart"] != true || !present || model != "" {
		t.Fatalf("clear result = %v, want ok with explicit empty model echo", r)
	}
	if _, present := r["effort"]; present {
		t.Fatalf("clear result echoed effort the request did not carry: %v", r)
	}
	raw, err = os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "HOTLINE_SDK_MODEL") {
		t.Errorf("model line survived the clear:\n%s", raw)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_EFFORT=high") {
		t.Errorf("effort line lost on an unrelated clear:\n%s", raw)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); err != nil {
		t.Fatalf("clear did not request a restart: %v", err)
	}
}

// TestSetSDKConfigNoOp: requesting the values already in effect writes
// nothing, bounces nothing, and answers ok+restart:false.
func TestSetSDKConfigNoOp(t *testing.T) {
	envRoot := sdkEnvSandbox(t)
	if err := os.WriteFile(filepath.Join(envRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}

	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk"})

	ride := sdkConfigRide(t, "cid-sdk-noop00001", map[string]any{"rid": "01J2ZKA2NOOP0001", "model": "claude-opus-4-8", "effort": "xhigh"})
	if bad, fatal := srv.handleSessionInput(context.Background(), dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("no-op ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != true || r["restart"] != false || r["model"] != "claude-opus-4-8" || r["effort"] != "xhigh" {
		t.Fatalf("no-op result = %v", r)
	}
	after, err := os.Stat(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !after.ModTime().Equal(before.ModTime()) || after.Size() != before.Size() {
		t.Error("no-op rewrote the .env")
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("no-op filed a restart request (err=%v)", err)
	}
}

// TestSetSDKConfigRefusalsAndErrors: non-sdk harness refusal (disk untouched),
// unknown/empty harness refusal, validation errors, bad-rid silent drop, and
// the no-supervisor restart_failed downgrade (persisted, not bounced).
func TestSetSDKConfigRefusalsAndErrors(t *testing.T) {
	ctx := context.Background()

	// (a) TUI harness: refuse with the pinned detail; .env never created.
	envRoot := sdkEnvSandbox(t)
	srv, _, dev, sub := activeHarness(t)
	srv.supervisorDir = t.TempDir()
	srv.SetAgentInfo(AgentInfo{Harness: "claude"})
	ride := sdkConfigRide(t, "cid-sdk-tui000001", map[string]any{"rid": "01J2ZKA3TUI00001", "model": "claude-sonnet-4-6"})
	if bad, fatal := srv.handleSessionInput(ctx, dev, sub, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("tui ride: bad=%v fatal=%v", bad, fatal)
	}
	results := drainSDKResults(t, sub, 300*time.Millisecond)
	if len(results) != 1 {
		t.Fatalf("tui results = %d, want 1", len(results))
	}
	r := results[0]
	if r["ok"] != false || r["code"] != "unsupported_harness" {
		t.Fatalf("tui result = %v", r)
	}
	if r["detail"] != "this box runs the claude TUI; model/effort can't be set remotely" {
		t.Errorf("tui detail = %q (fixture sdk-config.json pins this string)", r["detail"])
	}
	if _, err := os.Stat(filepath.Join(envRoot, ".env")); !os.IsNotExist(err) {
		t.Errorf("refusal touched the .env (err=%v)", err)
	}
	if _, err := os.Stat(filepath.Join(srv.supervisorDir, "restart.request")); !os.IsNotExist(err) {
		t.Errorf("refusal filed a restart request (err=%v)", err)
	}

	// (b) Unseeded harness identity: honest refusal too.
	srv2, _, dev2, sub2 := activeHarness(t)
	ride = sdkConfigRide(t, "cid-sdk-unk000001", map[string]any{"rid": "01J2ZKA4UNK00001", "effort": "high"})
	if bad, fatal := srv2.handleSessionInput(ctx, dev2, sub2, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("unknown-harness ride: bad=%v fatal=%v", bad, fatal)
	}
	results = drainSDKResults(t, sub2, 300*time.Millisecond)
	if len(results) != 1 || results[0]["code"] != "unsupported_harness" {
		t.Fatalf("unknown-harness results = %v", results)
	}

	// (c) Validation errors on an sdk box: invalid model, invalid effort,
	// empty request — .env stays untouched.
	srv3, _, dev3, sub3 := activeHarness(t)
	srv3.supervisorDir = t.TempDir()
	srv3.SetAgentInfo(AgentInfo{Harness: "claude-sdk"})
	for _, tc := range []struct {
		fields map[string]any
		code   string
	}{
		{map[string]any{"rid": "01J2ZKA5VAL00001", "model": "not a model!"}, "invalid_model"},
		{map[string]any{"rid": "01J2ZKA5VAL00002", "effort": "xtreme"}, "invalid_effort"},
		{map[string]any{"rid": "01J2ZKA5VAL00003"}, "empty_request"},
	} {
		ride = sdkConfigRide(t, "cid-sdk-val", tc.fields)
		if bad, fatal := srv3.handleSessionInput(ctx, dev3, sub3, ride, func([]byte) error { return nil }); bad || fatal {
			t.Fatalf("%s ride: bad=%v fatal=%v", tc.code, bad, fatal)
		}
		results = drainSDKResults(t, sub3, 300*time.Millisecond)
		if len(results) != 1 || results[0]["ok"] != false || results[0]["code"] != tc.code {
			t.Fatalf("want %s, results = %v", tc.code, results)
		}
	}
	if _, err := os.Stat(filepath.Join(envRoot, ".env")); !os.IsNotExist(err) {
		t.Errorf("validation errors touched the .env (err=%v)", err)
	}

	// (d) Malformed rid: consumed, no result routed, nothing persisted.
	ride = sdkConfigRide(t, "cid-sdk-badrid001", map[string]any{"rid": "has space", "model": "claude-opus-4-8"})
	if bad, fatal := srv3.handleSessionInput(ctx, dev3, sub3, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("bad-rid ride: bad=%v fatal=%v, want silent consume", bad, fatal)
	}
	if results = drainSDKResults(t, sub3, 300*time.Millisecond); len(results) != 0 {
		t.Fatalf("bad rid routed %d results, want 0", len(results))
	}

	// (e) No supervisor: persisted but downgraded to restart_failed.
	srv3.supervisorDir = ""
	ride = sdkConfigRide(t, "cid-sdk-nosup0001", map[string]any{"rid": "01J2ZKA6NOSUP001", "model": "claude-sonnet-4-6"})
	if bad, fatal := srv3.handleSessionInput(ctx, dev3, sub3, ride, func([]byte) error { return nil }); bad || fatal {
		t.Fatalf("no-supervisor ride: bad=%v fatal=%v", bad, fatal)
	}
	results = drainSDKResults(t, sub3, 300*time.Millisecond)
	if len(results) != 1 || results[0]["code"] != "restart_failed" {
		t.Fatalf("no-supervisor results = %v", results)
	}
	if results[0]["detail"] != "settings saved; restart the box to apply" {
		t.Errorf("restart_failed detail = %q", results[0]["detail"])
	}
	raw, err := os.ReadFile(filepath.Join(envRoot, ".env"))
	if err != nil {
		t.Fatalf("restart_failed must still have persisted: %v", err)
	}
	if !strings.Contains(string(raw), "HOTLINE_SDK_MODEL=claude-sonnet-4-6") {
		t.Errorf("restart_failed .env = %q", raw)
	}
}
