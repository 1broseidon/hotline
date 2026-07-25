package mcpchan

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestMissionSchemaValidJSON guards the verbatim literal.
func TestMissionSchemaValidJSON(t *testing.T) {
	var v map[string]any
	if err := json.Unmarshal([]byte(missionSchema), &v); err != nil {
		t.Fatalf("missionSchema is not valid JSON: %v", err)
	}
	props, _ := v["properties"].(map[string]any)
	for _, want := range []string{"action", "thread", "text", "status", "summary", "next", "state", "beware", "outcome", "trigger"} {
		if _, ok := props[want]; !ok {
			t.Errorf("missionSchema missing property %q", want)
		}
	}
	req, _ := v["required"].([]any)
	if len(req) != 1 || req[0] != "action" {
		t.Errorf("missionSchema required = %v, want [action]", req)
	}
}

// mcTeachingGolden pins the teaching segment wording (it rides every non-Claude
// session's instructions — a reword should be a reviewed diff).
const mcTeachingGolden = "Mission control is your memory across sessions, at /s/mc. The mc-index block below is your map — you wake up already holding it; read a thread file only when you need its detail. File as you go with the mission tool: note logs a durable fact or event; update upserts a thread (status, summary, next) and keeps the index current; handoff saves resume state before context runs long or a restart; archive closes a thread. Anything not filed is gone after a restart. Update the thread you're working before you tell the user it's done."

const mcPointerGolden = "Mission control memory: /s/mc/INDEX.md — read it first each session; file updates with the mission tool."

func TestMissionInstructionSegmentGoldens(t *testing.T) {
	if got := mcTeachingSegment("/s/mc"); got != mcTeachingGolden {
		t.Errorf("teaching segment changed:\n got: %s\nwant: %s", got, mcTeachingGolden)
	}
	if got := mcPointerSegment("/s/mc"); got != mcPointerGolden {
		t.Errorf("pointer segment changed:\n got: %s\nwant: %s", got, mcPointerGolden)
	}
	// Teaching segment is close to the ~540-char budget documented in the spec.
	if l := len(mcTeachingGolden); l < 480 || l > 620 {
		t.Errorf("teaching segment length %d outside the expected ~540-char band", l)
	}
}

// newTestMount builds a seeded, index-populated MC mount for a harness.
func newTestMount(t *testing.T, h Harness) *mcMount {
	t.Helper()
	store := mc.NewStore(t.TempDir() + "/mc")
	store.SetClock(func() time.Time { return time.Date(2026, 7, 16, 14, 2, 11, 0, time.UTC) })
	if err := store.Seed(); err != nil {
		t.Fatal(err)
	}
	store.Apply(mc.Input{Action: "update", Thread: "relay-cors", Summary: "CORS fix", Next: "verify on prod"})
	return &mcMount{store: store, budget: 4096, harness: h}
}

func TestMissionInjectionPi(t *testing.T) {
	m := newTestMount(t, HarnessPi)
	mech := mcMechanics(m)
	// Pi gets teaching + index + the fresh-at-boundary doctrine (spec §4).
	if len(mech) != 3 {
		t.Fatalf("pi MC should inject teaching + index + doctrine, got %d paragraphs", len(mech))
	}
	if !strings.HasPrefix(mech[0], "Mission control is your memory") {
		t.Errorf("first paragraph should be the teaching segment: %s", mech[0])
	}
	if !strings.HasPrefix(mech[1], "<mc-index>") || !strings.Contains(mech[1], "relay-cors") {
		t.Errorf("second paragraph should be the rendered index: %s", mech[1])
	}
	if mech[2] != mcDoctrineSegment || !strings.Contains(mech[2], "trigger: boundary") {
		t.Errorf("third paragraph should be the fresh-at-boundary doctrine: %s", mech[2])
	}

	// The whole thing appears in the Pi instructions render.
	pi := renderAgentInstructions("/state/transcript.jsonl", "", HarnessPi, mech)
	if !strings.Contains(pi, "<mc-index>") || !strings.Contains(pi, "Mission control is your memory") {
		t.Error("Pi instructions should carry the MC injection")
	}
	if !strings.Contains(pi, "call the restart tool") {
		t.Error("Pi instructions should carry the fresh-at-boundary doctrine")
	}
}

func TestMissionInjectionOpenCodeNoDoctrine(t *testing.T) {
	m := newTestMount(t, HarnessOpenCode)
	mech := mcMechanics(m)
	// OpenCode gets teaching + index but NOT the pi-only doctrine (no restart
	// tool, no compaction seam yet).
	if len(mech) != 2 {
		t.Fatalf("opencode MC should inject teaching + index only, got %d paragraphs", len(mech))
	}
	for _, p := range mech {
		if p == mcDoctrineSegment {
			t.Error("opencode must not get the pi-only doctrine")
		}
	}
}

func TestMissionInjectionClaudePointerOnly(t *testing.T) {
	m := newTestMount(t, HarnessClaude)
	mech := mcMechanics(m)
	if len(mech) != 1 || !strings.HasPrefix(mech[0], "Mission control memory:") {
		t.Fatalf("claude MC should inject the pointer line only, got %v", mech)
	}
	if strings.Contains(mech[0], "<mc-index>") {
		t.Error("claude must not get the injected index body (capped instruction field)")
	}
}

// TestCappedRenderCountsMCAgainstBudget is the P1-2 guard: a heavy MC block
// (teaching + a big index, the shape opencode routes through the capped path)
// must never balloon the capped instruction field or truncate the voice to nothing. The
// block is dropped whole from this capped field (opencode's real vehicle is the
// baked agent file), so the base mechanics and voice survive intact and the
// output stays within budget.
func TestCappedRenderCountsMCAgainstBudget(t *testing.T) {
	plain := instructions("/t", "")
	heavy := []string{"Mission control teaching…", "<mc-index>\n" + strings.Repeat("x", 6000) + "\n</mc-index>"}

	got := renderCapped("/t", "", heavy)
	if len(got) > instructionBudget {
		t.Errorf("capped render with heavy MC = %d bytes, must be ≤ %d", len(got), instructionBudget)
	}
	// The voice (built-in) must not have been starved to nothing by the MC block.
	if len(got) < len(plain) {
		t.Errorf("MC block truncated the base render (%d < %d) — voice was starved", len(got), len(plain))
	}

	// A small MC block (the claude pointer line) DOES land in the capped field.
	pointer := []string{mcPointerSegment("/s/mc")}
	withPointer := renderCapped("/t", "", pointer)
	if !strings.Contains(withPointer, "Mission control memory:") {
		t.Errorf("claude pointer line should ride the capped render:\n%s", withPointer)
	}
	if len(withPointer) > instructionBudget {
		t.Errorf("capped render with pointer = %d bytes, must be ≤ %d", len(withPointer), instructionBudget)
	}
}

// TestAgentInstructionsWithMCBakesIndex confirms the OpenCode agent-file vehicle
// carries the full MC block uncapped (spec §2), unlike the capped MCP field.
func TestAgentInstructionsWithMCBakesIndex(t *testing.T) {
	m := newTestMount(t, HarnessOpenCode)
	mech := mcMechanics(m)
	body := AgentInstructionsWithMC("/state/transcript.jsonl", "", mech, "app")
	if !strings.Contains(body, "<mc-index>") || !strings.Contains(body, "relay-cors") {
		t.Errorf("agent-file bake missing the MC index:\n%s", body)
	}
	// MCMechanicsForOptions must surface the same block the option would inject.
	opt := MCMechanicsForOptions([]ServerOption{WithMissionControl(m.store, m.budget, HarnessOpenCode)})
	if len(opt) != len(mech) || opt[0] != mech[0] {
		t.Errorf("MCMechanicsForOptions diverged from mcMechanics: %v vs %v", opt, mech)
	}
}

// TestMissionToolRegisteredWhenMounted drives a server with MC mounted and
// verifies the mission tool is present with the verbatim schema and that a call
// round-trips to the store.
func TestMissionToolRegisteredWhenMounted(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	m := newTestMount(t, HarnessPi)
	server := NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", nil, "", "", "", "",
		WithMissionControl(m.store, m.budget, HarnessPi))

	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatal(err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "t", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()

	lr, err := session.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	var found *mcp.Tool
	for _, tool := range lr.Tools {
		if tool.Name == "mission" {
			found = tool
		}
	}
	if found == nil {
		t.Fatal("mission tool not registered when MC mounted")
	}
	if !jsonEqual(t, []byte(missionSchema), found.InputSchema) {
		t.Error("mission tool schema is not the verbatim literal")
	}

	res, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name:      "mission",
		Arguments: map[string]any{"action": "note", "text": "a fact from the tool"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if res.IsError {
		t.Fatalf("mission note should succeed: %+v", res.Content)
	}
}

// TestMissionToolCarriesTrigger drives the mission tool with a handoff that sets
// trigger and verifies the value reaches the store (regression: the tool schema
// and missionInput used to drop trigger, so every model handoff recorded manual).
func TestMissionToolCarriesTrigger(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	m := newTestMount(t, HarnessPi)
	server := NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", nil, "", "", "", "",
		WithMissionControl(m.store, m.budget, HarnessPi))
	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatal(err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "t", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()

	res, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "mission",
		Arguments: map[string]any{
			"action":  "handoff",
			"state":   "half wired",
			"next":    "wire redial",
			"trigger": "boundary",
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if res.IsError {
		t.Fatalf("mission handoff should succeed: %+v", res.Content)
	}
	ho, has, err := m.store.ReadHandoff()
	if err != nil || !has {
		t.Fatalf("handoff.md missing: has=%v err=%v", has, err)
	}
	if !strings.Contains(ho, "trigger: boundary") {
		t.Errorf("tool trigger dropped; handoff.md:\n%s", ho)
	}

	// An unknown trigger is rejected (store-level validation covers the tool path).
	bad, err := session.CallTool(ctx, &mcp.CallToolParams{
		Name: "mission",
		Arguments: map[string]any{
			"action": "handoff", "state": "s", "next": "n", "trigger": "sideways",
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !bad.IsError {
		t.Error("unknown trigger should be rejected")
	}
}

// TestMissionToolAbsentWithoutMount confirms the default server has no mission tool.
func TestMissionToolAbsentWithoutMount(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	server := NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", nil, "", "", "", "")
	st, ct := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, st, nil); err != nil {
		t.Fatal(err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "t", Version: "0"}, nil)
	session, err := client.Connect(ctx, ct, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()
	lr, _ := session.ListTools(ctx, nil)
	for _, tool := range lr.Tools {
		if tool.Name == "mission" {
			t.Error("mission tool must be absent when MC is not mounted")
		}
	}
}
