package app

import (
	"encoding/json"
	"testing"
	"time"
)

// TestWelcomeFrameMetadata pins the §3.2 wire contract: the optional
// harness/model/effort identity rides welcome when known, and an empty
// AgentInfo leaves the frame byte-shape identical to the pre-metadata one
// (old boxes / unknown identity emit no new keys).
func TestWelcomeFrameMetadata(t *testing.T) {
	room := RoomRecord{ID: "Ab3dEf6hIj8lMn0pQr2tUv", Name: "Umibozu"}

	withMeta := welcomeFrame(room, "dev-af31fd290542", "9007199254740993", "9007199254741007",
		AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "xhigh"})
	var got map[string]any
	if err := json.Unmarshal(withMeta, &got); err != nil {
		t.Fatal(err)
	}
	want := map[string]any{
		"t": "welcome", "v": float64(2), "room": "Ab3dEf6hIj8lMn0pQr2tUv", "name": "Umibozu",
		"device_id": "dev-af31fd290542", "floor": "9007199254740993", "head": "9007199254741007",
		"harness": "claude-sdk", "model": "claude-opus-4-8", "effort": "xhigh",
	}
	if len(got) != len(want) {
		t.Fatalf("welcome keys = %v, want exactly %v", got, want)
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("welcome[%q] = %v, want %v", k, got[k], v)
		}
	}

	// Unknown identity: no new keys at all (compat with pre-metadata frames).
	plain := welcomeFrame(room, "dev-af31fd290542", "1", "2", AgentInfo{})
	var gotPlain map[string]any
	if err := json.Unmarshal(plain, &gotPlain); err != nil {
		t.Fatal(err)
	}
	for _, k := range []string{"harness", "model", "effort"} {
		if _, present := gotPlain[k]; present {
			t.Errorf("empty AgentInfo leaked key %q into welcome: %s", k, plain)
		}
	}

	// Partial identity (pi with a model knob, no effort): only known fields.
	partial := welcomeFrame(room, "dev-af31fd290542", "1", "2", AgentInfo{Harness: "pi", Model: "gpt-5.4"})
	var gotPartial map[string]any
	if err := json.Unmarshal(partial, &gotPartial); err != nil {
		t.Fatal(err)
	}
	if gotPartial["harness"] != "pi" || gotPartial["model"] != "gpt-5.4" {
		t.Errorf("partial identity mangled: %s", partial)
	}
	if _, present := gotPartial["effort"]; present {
		t.Errorf("absent effort must be omitted: %s", partial)
	}
}

// TestBuildAgentStateStampsIdentity: the snapshot builder stamps the current
// AgentInfo next to Name (§1.2), and empty() treats a harness-bearing
// snapshot as announceable — an idle box still says who it is.
func TestBuildAgentStateStampsIdentity(t *testing.T) {
	t.Setenv("HOTLINE_YOLO", "0")
	t.Setenv("HOTLINE_HARNESS", "claude")
	srv, _, _, _ := activeHarness(t)

	// No identity: unchanged legacy behavior.
	if snap := srv.buildAgentState(); snap.Harness != "" || !snap.empty() {
		t.Fatalf("pre-identity snapshot = %+v, want empty/announce-suppressed", snap)
	}

	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8", Effort: "xhigh"})
	snap := srv.buildAgentState()
	if snap.Harness != "claude-sdk" || snap.Model != "claude-opus-4-8" || snap.Effort != "xhigh" {
		t.Fatalf("snapshot identity = %+v", snap)
	}
	if snap.empty() {
		t.Fatal("a harness-bearing snapshot must not be suppressed as empty (idle box announces identity)")
	}

	// The JSON shape carries the three fields next to name with omitempty.
	body, err := json.Marshal(snap)
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(body, &m); err != nil {
		t.Fatal(err)
	}
	if m["harness"] != "claude-sdk" || m["model"] != "claude-opus-4-8" || m["effort"] != "xhigh" {
		t.Errorf("snapshot JSON = %s", body)
	}
}

// TestSetAgentInfoMergeAndLiveEmit: non-empty fields merge (a model-only
// refinement never clears harness/effort), a change triggers a live
// agent_state emit to attached devices, and a no-op set stays silent.
func TestSetAgentInfoMergeAndLiveEmit(t *testing.T) {
	t.Setenv("HOTLINE_YOLO", "0")
	t.Setenv("HOTLINE_HARNESS", "claude")
	srv, _, _, sub := activeHarness(t)
	srv.asThrottle = time.Millisecond // fast trailing sends for the test

	// Construction seed (as NewProvider does it).
	srv.agentInfo = AgentInfo{Harness: "claude-sdk", Effort: "xhigh"}
	drainTransients(t, sub, 50*time.Millisecond)

	// harness_info refinement: model resolves after attach.
	srv.SetAgentInfo(AgentInfo{Harness: "claude-sdk", Model: "claude-opus-4-8"})
	frames := drainTransients(t, sub, 300*time.Millisecond)
	var seen map[string]any
	for _, f := range frames {
		if f["t"] == "agent_state" {
			seen = f["state"].(map[string]any)
		}
	}
	if seen == nil {
		t.Fatal("model refinement did not emit an agent_state")
	}
	if seen["harness"] != "claude-sdk" || seen["model"] != "claude-opus-4-8" || seen["effort"] != "xhigh" {
		t.Fatalf("emitted state = %v (effort must survive a model-only merge)", seen)
	}

	// Identical set: no change, no emit.
	srv.SetAgentInfo(AgentInfo{Model: "claude-opus-4-8"})
	for _, f := range drainTransients(t, sub, 150*time.Millisecond) {
		if f["t"] == "agent_state" {
			t.Fatalf("no-op SetAgentInfo emitted: %v", f)
		}
	}

	if info := srv.currentAgentInfo(); info.Harness != "claude-sdk" || info.Model != "claude-opus-4-8" || info.Effort != "xhigh" {
		t.Fatalf("merged info = %+v", info)
	}
}
