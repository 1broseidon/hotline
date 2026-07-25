package app

// Model catalog amendment 2026-07-20 — the box's half of the wire.

import (
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// TestAgentCatalogFrameShape pins the wire contract: the optional metadata is
// omitted when unset (so a minimal catalog stays minimal), models is always an
// array, and `available` is ALWAYS serialized — an omitted false would be
// indistinguishable from a frame that never carried the field.
func TestAgentCatalogFrameShape(t *testing.T) {
	cat := AgentCatalog{
		Harness: "pi",
		Source:  "models",
		Models: []catalogModel{
			{ID: "openai-codex/gpt-5.6-sol", Label: "Sol", Available: true},
			{ID: "google/gemini-3-pro", Label: "Gemini 3 Pro", Available: false},
		},
	}
	got := asMap(t, agentCatalogFrame(7, cat))
	if got["t"] != "agent_catalog" || got["seq"] != float64(7) {
		t.Fatalf("envelope = %v", got)
	}
	if got["harness"] != "pi" || got["source"] != "models" {
		t.Errorf("metadata = %v", got)
	}
	if _, present := got["truncated"]; present {
		t.Errorf("an uncut catalog must not carry truncated: %v", got)
	}
	models, ok := got["models"].([]any)
	if !ok || len(models) != 2 {
		t.Fatalf("models = %v", got["models"])
	}
	first := models[0].(map[string]any)
	if first["id"] != "openai-codex/gpt-5.6-sol" || first["label"] != "Sol" || first["available"] != true {
		t.Errorf("entry = %v", first)
	}
	second := models[1].(map[string]any)
	if v, present := second["available"]; !present || v != false {
		t.Errorf("available:false MUST be serialized, got present=%v value=%v", present, v)
	}

	// Truncation is announced; a nil model slice still serializes as [].
	trunc := asMap(t, agentCatalogFrame(8, AgentCatalog{Truncated: true}))
	if trunc["truncated"] != true {
		t.Errorf("truncated not carried: %v", trunc)
	}
	if arr, ok := trunc["models"].([]any); !ok || len(arr) != 0 {
		t.Errorf("models must be [] not null: %v", trunc["models"])
	}
}

// TestSanitizeAgentCatalog: everything the harness reports is ultimately read
// out of a third-party registry (models.json, plus whatever extensions register
// at runtime), so it is bounded and de-duplicated before the box ever holds it.
func TestSanitizeAgentCatalog(t *testing.T) {
	long := strings.Repeat("x", maxCatalogModelID+5)
	in := []catalogModel{
		{ID: "  zai/glm-5.2  ", Label: "  GLM  5.2 ", Available: true},
		{ID: "zai/glm-5.2", Label: "dupe", Available: false}, // de-duped, first wins
		{ID: "", Label: "no id"},                             // dropped: a tap could only fail
		{ID: long, Label: "capped id"},
		{ID: "ok/model", Label: strings.Repeat("L", maxCatalogLabel+10)},
	}
	got := sanitizeAgentCatalog("pi", "models", in, false)
	if len(got.Models) != 3 {
		t.Fatalf("models = %+v, want 3 (dupe and id-less dropped)", got.Models)
	}
	if got.Models[0].ID != "zai/glm-5.2" || got.Models[0].Label != "GLM 5.2" {
		t.Errorf("entry not trimmed/flattened: %+v", got.Models[0])
	}
	if !got.Models[0].Available {
		t.Errorf("the FIRST occurrence wins on a duplicate: %+v", got.Models[0])
	}
	if len([]rune(got.Models[1].ID)) != maxCatalogModelID {
		t.Errorf("id not capped: %d runes", len([]rune(got.Models[1].ID)))
	}
	if len([]rune(got.Models[2].Label)) != maxCatalogLabel {
		t.Errorf("label not capped: %d runes", len([]rune(got.Models[2].Label)))
	}
}

// TestSanitizeAgentCatalogCap: a harness that over-reports is cut at the cap
// and the cut is announced, so the app can say the menu is partial rather than
// implying the box can run nothing else.
func TestSanitizeAgentCatalogCap(t *testing.T) {
	var in []catalogModel
	for i := 0; i < maxCatalogModels+5; i++ {
		in = append(in, catalogModel{ID: string(rune('a'+i%26)) + "/" + strings.Repeat("z", i%7+1) + string(rune('0'+i%10)), Available: true})
	}
	got := sanitizeAgentCatalog("pi", "available", in, false)
	if len(got.Models) > maxCatalogModels {
		t.Fatalf("catalog not capped: %d entries", len(got.Models))
	}
	if !got.Truncated {
		t.Error("a cut catalog must report truncated")
	}
}

// TestHarnessCatalogSink: the harness notification lands as a sanitized,
// bounded catalog on the server — the one path a catalog can enter by.
func TestHarnessCatalogSink(t *testing.T) {
	// A real Server: SetAgentCatalog broadcasts on change, which needs the
	// outbox/store a bare struct literal doesn't have.
	srv, _ := newTools(testConfig(t))
	p := &Provider{srv: srv}
	p.HarnessCatalogSink()(mcpchan.HarnessCatalogParams{
		Harness: "pi",
		Source:  "models",
		Models: []mcpchan.HarnessCatalogModel{
			{ID: "openai-codex/gpt-5.6-sol", Label: "Sol", Available: true},
			{ID: "  ", Label: "blank"},
		},
	})
	got := srv.currentAgentCatalog()
	if got.Harness != "pi" || got.Source != "models" {
		t.Errorf("metadata = %+v", got)
	}
	if len(got.Models) != 1 || got.Models[0].ID != "openai-codex/gpt-5.6-sol" || !got.Models[0].Available {
		t.Errorf("models = %+v", got.Models)
	}
}

// TestAgentCatalogChangeGate: an identical re-report (every harness respawn
// re-enumerates) must not re-broadcast to every attached device; a real change
// must. The gate is what keeps a restart from costing device traffic.
func TestAgentCatalogChangeGate(t *testing.T) {
	a := sanitizeAgentCatalog("pi", "models", []catalogModel{{ID: "a/b", Label: "AB", Available: true}}, false)
	b := sanitizeAgentCatalog("pi", "models", []catalogModel{{ID: "a/b", Label: "AB", Available: true}}, false)
	if !agentCatalogEqual(a, b) {
		t.Error("an identical re-report must compare equal")
	}
	for name, other := range map[string]AgentCatalog{
		"different source":      sanitizeAgentCatalog("pi", "available", a.Models, false),
		"different label":       sanitizeAgentCatalog("pi", "models", []catalogModel{{ID: "a/b", Label: "XY", Available: true}}, false),
		"availability flipped":  sanitizeAgentCatalog("pi", "models", []catalogModel{{ID: "a/b", Label: "AB"}}, false),
		"an extra model":        sanitizeAgentCatalog("pi", "models", append(append([]catalogModel{}, a.Models...), catalogModel{ID: "c/d", Available: true}), false),
		"truncation flipped on": sanitizeAgentCatalog("pi", "models", a.Models, true),
	} {
		if agentCatalogEqual(a, other) {
			t.Errorf("%s must count as a change", name)
		}
	}
}

// TestAgentCatalogEmptyIsNotSent: absence and emptiness are different claims.
// A box with no catalog must stay silent so the app keeps its curated fallback;
// an empty frame would assert "this box has a menu and it is empty", leaving the
// operator with nothing but free text.
func TestAgentCatalogEmptyIsNotSent(t *testing.T) {
	if !(AgentCatalog{}).empty() {
		t.Error("a zero catalog must be empty")
	}
	if !(AgentCatalog{Harness: "pi", Source: "models"}).empty() {
		t.Error("metadata without models is still nothing to offer")
	}
	if (AgentCatalog{Models: []catalogModel{{ID: "a/b"}}}).empty() {
		t.Error("one model is a catalog")
	}

	// snapshotAgentCatalogTo must not even reach the emit path when empty; a
	// nil-device Server would panic in emitTransientTo if it did.
	srv := &Server{}
	srv.snapshotAgentCatalogTo("dev-nobody")
}

// --- fixture parity ---------------------------------------------------------

type agentCatalogFixture struct {
	Frames []struct {
		Dir   string          `json:"dir"`
		Frame json.RawMessage `json:"frame"`
	} `json:"frames"`
}

// TestAgentCatalogFixtureShapes proves this implementation and the frozen
// fixture agree shape for shape: the frames the box builds equal the fixture's,
// and the set_sdk_config rides parse to the exact requests. The client lane
// builds against the same file (byte-identical copy in the app repo).
func TestAgentCatalogFixtureShapes(t *testing.T) {
	raw, err := os.ReadFile("../../protocol/v2/fixtures/agent-catalog.json")
	if err != nil {
		t.Fatalf("reading fixture: %v", err)
	}
	var fx agentCatalogFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parsing fixture: %v", err)
	}
	if len(fx.Frames) != 8 {
		t.Fatalf("fixture has %d frames, want 8 (scoped catalog, unscoped+truncated catalog, row ride, row ok, off-catalog ride, off-catalog ok, unkeyed ride, no_api_key)", len(fx.Frames))
	}

	// Frame 0: the scoped catalog pushed on subscribe.
	want0 := asMap(t, fx.Frames[0].Frame)
	got0 := asMap(t, agentCatalogFrame(902, AgentCatalog{
		Harness: "pi", Source: "models",
		Models: []catalogModel{
			{ID: "openai-codex/gpt-5.6-luna", Label: "Luna", Available: true},
			{ID: "openai-codex/gpt-5.6-sol", Label: "Sol", Available: true},
			{ID: "anthropic/claude-opus-4-8", Label: "Opus 4.8", Available: true},
		},
	}))
	if !jsonEqual(t, got0, want0) {
		t.Errorf("scoped catalog frame\n got %v\nwant %v", got0, want0)
	}

	// Frame 1: the unscoped, truncated catalog carrying an unavailable model.
	want1 := asMap(t, fx.Frames[1].Frame)
	got1 := asMap(t, agentCatalogFrame(903, AgentCatalog{
		Harness: "pi", Source: "available", Truncated: true,
		Models: []catalogModel{
			{ID: "anthropic/claude-opus-4-8", Label: "Opus 4.8", Available: true},
			{ID: "zai/glm-5.2", Label: "GLM 5.2", Available: true},
			{ID: "openai/gpt-5.5", Label: "GPT-5.5", Available: false},
		},
	}))
	if !jsonEqual(t, got1, want1) {
		t.Errorf("unscoped catalog frame\n got %v\nwant %v", got1, want1)
	}

	// Frames 2/4/6: the rides parse to the exact model requests. Frame 4 is the
	// point of the whole amendment — a model absent from frame 0's catalog.
	for _, tc := range []struct {
		idx        int
		rid, model string
	}{
		{2, "01J2ZM50N4RX9YZ8", "openai-codex/gpt-5.6-sol"},
		{4, "01J2ZM6C1P5TA0B2", "zai/glm-5.2"},
		{6, "01J2ZM7E2Q6UB1C3", "google/gemini-3-pro"},
	} {
		req, isControl := setSDKConfigFromDeviceSend(fx.Frames[tc.idx].Frame)
		if !isControl {
			t.Fatalf("frame %d not recognized as a set_sdk_config control", tc.idx)
		}
		if req.RID != tc.rid || req.Model == nil || *req.Model != tc.model {
			t.Errorf("frame %d parsed to rid=%q model=%v, want %q/%q", tc.idx, req.RID, req.Model, tc.rid, tc.model)
		}
		// The box must ACCEPT every one of these under the pi knobs: the
		// catalog changed which rows are offered, not what the box validates.
		knobs, _ := knobsFor("pi", "")
		if code, detail := validateSDKConfig(req, knobs); code != "" {
			t.Errorf("frame %d refused by the box: %s (%s)", tc.idx, code, detail)
		}
	}

	// Frames 3/5: ordinary hot results. Frame 5 proves the free-text tier is
	// genuinely wider than the rows.
	for _, tc := range []struct {
		idx        int
		seq        uint64
		rid, model string
	}{
		{3, 904, "01J2ZM50N4RX9YZ8", "openai-codex/gpt-5.6-sol"},
		{5, 905, "01J2ZM6C1P5TA0B2", "zai/glm-5.2"},
	} {
		model := tc.model
		got := asMap(t, sdkConfigResultFrame(tc.seq, tc.rid, true, &model, nil, false, true, false, "", ""))
		if !jsonEqual(t, got, asMap(t, fx.Frames[tc.idx].Frame)) {
			t.Errorf("frame %d\n got %v\nwant %v", tc.idx, got, asMap(t, fx.Frames[tc.idx].Frame))
		}
	}

	// Frame 7: no_api_key — resolved, but no credential. Distinct from
	// unknown_model because the fix is a login, not a typo.
	got7 := asMap(t, sdkConfigResultFrame(906, "01J2ZM7E2Q6UB1C3", false, nil, nil, false, false, false,
		"no_api_key", "no API key configured for google"))
	if !jsonEqual(t, got7, asMap(t, fx.Frames[7].Frame)) {
		t.Errorf("no_api_key frame\n got %v\nwant %v", got7, asMap(t, fx.Frames[7].Frame))
	}
}

// jsonEqual compares two decoded frames structurally (key order and numeric
// formatting are irrelevant on the wire).
func jsonEqual(t *testing.T, a, b map[string]any) bool {
	t.Helper()
	ab, err := json.Marshal(a)
	if err != nil {
		t.Fatal(err)
	}
	bb, err := json.Marshal(b)
	if err != nil {
		t.Fatal(err)
	}
	return string(ab) == string(bb)
}
