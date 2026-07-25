package app

import (
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

// sdkConfigFixture mirrors the protocol/v2/fixtures/sdk-config.json document.
type sdkConfigFixture struct {
	Frames []struct {
		Dir   string          `json:"dir"`
		Frame json.RawMessage `json:"frame"`
	} `json:"frames"`
}

func loadSDKConfigFixture(t *testing.T) sdkConfigFixture {
	t.Helper()
	raw, err := os.ReadFile("../../protocol/v2/fixtures/sdk-config.json")
	if err != nil {
		t.Fatalf("reading fixture: %v", err)
	}
	var fx sdkConfigFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parsing fixture: %v", err)
	}
	if len(fx.Frames) != 14 {
		t.Fatalf("fixture has %d frames, want 14 (ride, ok, invalid_effort, unsupported_harness, welcome, hot ride, hot ok, agent_state confirm, unknown_model, effort-hot ride, effort-hot ok, effort agent_state confirm, unverified ride, unverified ok)", len(fx.Frames))
	}
	return fx
}

// asMap round-trips bytes through a generic map so shapes compare
// independent of key order and formatting.
func asMap(t *testing.T, raw []byte) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	return m
}

// TestSDKConfigFixtureShapes proves this implementation and the frozen
// fixture agree byte-shape for byte-shape: the device_send ride parses to the
// exact request, each result frame the box builds equals the fixture's, and
// the post-restart welcome carrying the new identity matches welcomeFrame's
// output. The client lane builds against the same file.
func TestSDKConfigFixtureShapes(t *testing.T) {
	fx := loadSDKConfigFixture(t)

	// Frame 0 (app->cli): the nested control ride parses into the exact
	// normalized request the handler receives.
	req, isControl := setSDKConfigFromDeviceSend(fx.Frames[0].Frame)
	if !isControl {
		t.Fatal("fixture ride not recognized as a set_sdk_config control")
	}
	if req.RID != "01J2ZKA0M3QW8XY7" || req.Model == nil || *req.Model != "claude-sonnet-4-6" || req.Effort == nil || *req.Effort != "high" {
		t.Fatalf("parsed req = %+v (model=%v effort=%v)", req, req.Model, req.Effort)
	}
	if code, _ := validateSDKConfig(req, sdkKnobs(t)); code != "" {
		t.Fatalf("fixture ride fails validation: %s", code)
	}

	// Frame 1 (cli->app): the ok+restart result the box builds for that ride.
	got := asMap(t, sdkConfigResultFrame(813, req.RID, true, req.Model, req.Effort, true, false, false, "", ""))
	if want := asMap(t, fx.Frames[1].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("ok result mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 2 (cli->app): invalid_effort — code AND detail must be the
	// production strings validateSDKConfig emits.
	badReq := sdkConfigRequest{RID: "01J2ZKA1B7C9D2E4", Effort: sp("xtreme")}
	code, detail := validateSDKConfig(badReq, sdkKnobs(t))
	got = asMap(t, sdkConfigResultFrame(814, badReq.RID, false, nil, nil, false, false, false, code, detail))
	if want := asMap(t, fx.Frames[2].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("invalid_effort result mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 3 (cli->app): unsupported_harness with the pinned TUI detail
	// (the connector-level test proves the handler emits this string).
	got = asMap(t, sdkConfigResultFrame(815, "01J2ZKA2F8G1H3J5", false, nil, nil, false, false, false,
		"unsupported_harness", "this box runs the claude TUI; model/effort can't be set remotely"))
	if want := asMap(t, fx.Frames[3].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("unsupported_harness result mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 4 (cli->app): the post-restart welcome carrying the newly
	// configured identity — the apply confirmation (SC5).
	got = asMap(t, welcomeFrame(
		RoomRecord{ID: "Ab3dEf6hIj8lMn0pQr2tUv", Name: "Umibozu"},
		"dev-af31fd290542", "9007199254740993", "9007199254741007",
		AgentInfo{Harness: "claude-sdk", Model: "claude-sonnet-4-6", Effort: "high"}))
	if want := asMap(t, fx.Frames[4].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("post-restart welcome mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 5 (app->cli): the SC6 hot ride — a model-only set_sdk_config
	// parses into the exact normalized request the hot path forwards.
	hotReq, isHotControl := setSDKConfigFromDeviceSend(fx.Frames[5].Frame)
	if !isHotControl {
		t.Fatal("fixture hot ride not recognized as a set_sdk_config control")
	}
	if hotReq.RID != "01J2ZKB5H2XW9YV3" || hotReq.Model == nil || *hotReq.Model != "claude-opus-4-8" || hotReq.Effort != nil {
		t.Fatalf("parsed hot req = %+v (model=%v effort=%v)", hotReq, hotReq.Model, hotReq.Effort)
	}
	if code, _ := validateSDKConfig(hotReq, sdkKnobs(t)); code != "" {
		t.Fatalf("fixture hot ride fails validation: %s", code)
	}

	// Frame 6 (cli->app): the hot success (SC6/SC7) — the frame
	// handleSDKApplyResult builds after the harness confirmed and the persist
	// landed: restart:false, hot:true, requested post-trim model echoed.
	got = asMap(t, sdkConfigResultFrame(816, hotReq.RID, true, hotReq.Model, nil, false, true, false, "", ""))
	if want := asMap(t, fx.Frames[6].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("hot success mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 7 (cli->app): the hot-apply confirmation (SC6) — the agent_state
	// snapshot the harness_info restamp broadcasts, carrying the new model
	// with no welcome involved.
	got = asMap(t, agentStateFrame(817, agentStateSnapshot{
		Name: "Umibozu", Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "high",
		Runs: []agentRun{}, Schedules: []agentSchedule{}, Loops: []agentLoop{},
	}))
	if want := asMap(t, fx.Frames[7].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("agent_state confirmation mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 8 (cli->app): the harness guard's unknown_model refusal (SC8),
	// forwarded verbatim — the detail is the harness's pinned string.
	got = asMap(t, sdkConfigResultFrame(818, "01J2ZKB6K4PQ7RS9", false, nil, nil, false, false, false,
		"unknown_model", "model not in the CLI's supported list"))
	if want := asMap(t, fx.Frames[8].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("unknown_model mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 9 (app->cli): the SC10 effort-hot ride — an effort-only
	// set_sdk_config parses into a request with model omitted (nil) and effort
	// set, which the hot gate routes to the hot path.
	effReq, isEffControl := setSDKConfigFromDeviceSend(fx.Frames[9].Frame)
	if !isEffControl {
		t.Fatal("fixture effort-hot ride not recognized as a set_sdk_config control")
	}
	if effReq.RID != "01J2ZKC1H2XW9YV3" || effReq.Model != nil || effReq.Effort == nil || *effReq.Effort != "xhigh" {
		t.Fatalf("parsed effort req = %+v (model=%v effort=%v)", effReq, effReq.Model, effReq.Effort)
	}
	if code, _ := validateSDKConfig(effReq, sdkKnobs(t)); code != "" {
		t.Fatalf("fixture effort-hot ride fails validation: %s", code)
	}

	// Frame 10 (cli->app): the effort hot success — model omitted, effort
	// echoed, restart:false, hot:true (handleSDKApplyResult after confirm+persist).
	got = asMap(t, sdkConfigResultFrame(819, effReq.RID, true, nil, effReq.Effort, false, true, false, "", ""))
	if want := asMap(t, fx.Frames[10].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("effort hot success mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 11 (cli->app): the effort hot-apply confirmation — the agent_state
	// restamp carrying the new effort, model unchanged from the prior hot apply.
	got = asMap(t, agentStateFrame(820, agentStateSnapshot{
		Name: "Umibozu", Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "xhigh",
		Runs: []agentRun{}, Schedules: []agentSchedule{}, Loops: []agentLoop{},
	}))
	if want := asMap(t, fx.Frames[11].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("effort agent_state confirmation mismatch:\n got %v\nwant %v", got, want)
	}

	// Frame 12 (app->cli): the SC11 unverified ride — a model-only
	// set_sdk_config for a well-formed id absent from the catalog. It is
	// syntactically valid (passes validateSDKConfig) and routes to the hot path
	// exactly like any model-only request; the box can't tell it's catalog-absent.
	unvReq, isUnvControl := setSDKConfigFromDeviceSend(fx.Frames[12].Frame)
	if !isUnvControl {
		t.Fatal("fixture unverified ride not recognized as a set_sdk_config control")
	}
	if unvReq.RID != "01J2ZKD1H2XW9YV3" || unvReq.Model == nil || *unvReq.Model != "claude-opus-4-6" || unvReq.Effort != nil {
		t.Fatalf("parsed unverified req = %+v (model=%v effort=%v)", unvReq, unvReq.Model, unvReq.Effort)
	}
	if code, _ := validateSDKConfig(unvReq, sdkKnobs(t)); code != "" {
		t.Fatalf("fixture unverified ride fails validation: %s", code)
	}

	// Frame 13 (cli->app): the SC11 Tier-2 success — handleSDKApplyResult
	// builds this after the harness applied the catalog-absent id and answered
	// unverified:true (threaded from SDKApplyResultParams.Unverified). Same
	// hot-success shape plus unverified:true.
	got = asMap(t, sdkConfigResultFrame(821, unvReq.RID, true, unvReq.Model, nil, false, true, true, "", ""))
	if want := asMap(t, fx.Frames[13].Frame); !reflect.DeepEqual(got, want) {
		t.Errorf("unverified hot success mismatch:\n got %v\nwant %v", got, want)
	}
}
