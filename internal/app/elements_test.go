package app

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

// --- shared WP-A harness -----------------------------------------------------

// activeHarness builds a Server+Tools with one active device (instant bubble
// mode so replies emit no typing pauses) and a live mailbox subscription whose
// durable items and transients the caller can drain.
func activeHarness(t *testing.T) (*Server, *Tools, string, *mailboxSubscriber) {
	t.Helper()
	cfg := testConfig(t)
	a := access.Defaults()
	a.BubbleMode = "instant"
	writeAccess(t, cfg, a)
	srv, tools := newTools(cfg)
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	const dev = "dev-testdevice01"
	srv.store.st.Rooms["room1"] = RoomRecord{ID: "room1"}
	srv.store.st.CurrentRoom = "room1"
	srv.store.st.Devices[dev] = DeviceRecord{ID: dev, Room: "room1", SecretHash: "x", State: DeviceActive}
	srv.store.mu.Lock()
	_ = srv.store.saveLocked()
	srv.store.mu.Unlock()
	srv.mailbox.disk.Devices[dev] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(dev)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.mailbox.unsubscribe(dev, sub); srv.outbox.close() })
	return srv, tools, dev, sub
}

// drainItems collects the durable frames currently queued on the subscription.
func drainItems(t *testing.T, sub *mailboxSubscriber) []map[string]any {
	t.Helper()
	var out []map[string]any
	deadline := time.After(500 * time.Millisecond)
	for {
		select {
		case it := <-sub.items:
			var m map[string]any
			if err := json.Unmarshal(it.Payload, &m); err != nil {
				t.Fatalf("decode item: %v", err)
			}
			out = append(out, m)
		case <-deadline:
			return out
		default:
			// nothing pending right now; give the emitter a beat then stop.
			select {
			case it := <-sub.items:
				var m map[string]any
				_ = json.Unmarshal(it.Payload, &m)
				out = append(out, m)
			case <-time.After(50 * time.Millisecond):
				return out
			}
		}
	}
}

func rawEl(t *testing.T, v any) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// --- validation --------------------------------------------------------------

func TestValidateElementsGoldenPerType(t *testing.T) {
	cases := []struct {
		name string
		in   map[string]any
		want string // substring that must appear in the canonical serialization
	}{
		{"chip", map[string]any{"el": "chip", "id": "el-t1", "fallback": "tests 233/233", "kind": "ok", "label": "tests", "value": "233/233"}, `"el":"chip"`},
		{"job", map[string]any{"el": "job", "id": "el-b1", "fallback": "header pass: running", "title": "header pass", "state": "running", "detail": "tests green", "startedAt": 1783876000, "progress": 0.6}, `"progress":0.6`},
		{"decision", map[string]any{"el": "decision", "id": "el-icon", "fallback": "pick A or B", "prompt": "which icon?", "options": []any{map[string]any{"key": "A", "label": "Classic"}, map[string]any{"key": "B", "label": "Bold", "thumb": map[string]any{"xfer": "x1", "mime": "image/png", "size": 1200}}}}, `"options":`},
		{"approval", map[string]any{"el": "approval", "id": "el-deploy", "fallback": "approve deploy?", "title": "deploy to prod?", "detail": "wrangler deploy", "approveLabel": "deploy", "denyLabel": "hold"}, `"approveLabel":"deploy"`},
		{"checklist", map[string]any{"el": "checklist", "id": "el-v", "fallback": "verify", "title": "verify on device", "items": []any{map[string]any{"key": "pill", "label": "pill hugs name", "done": false}}}, `"items":`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			els, err := validateElements([]json.RawMessage{rawEl(t, tc.in)})
			if err != nil {
				t.Fatalf("valid %s rejected: %v", tc.name, err)
			}
			if len(els) != 1 {
				t.Fatalf("want 1 element, got %d", len(els))
			}
			canon, _ := json.Marshal(els[0])
			if !strings.Contains(string(canon), tc.want) {
				t.Errorf("canonical %s missing %q: %s", tc.name, tc.want, canon)
			}
			// fallback + id + el are always present.
			for _, must := range []string{`"el":`, `"id":`, `"fallback":`} {
				if !strings.Contains(string(canon), must) {
					t.Errorf("canonical %s missing %s: %s", tc.name, must, canon)
				}
			}
		})
	}
}

func TestValidateElementsHostileAndLimits(t *testing.T) {
	chip := func(over map[string]any) map[string]any {
		m := map[string]any{"el": "chip", "id": "el-x", "fallback": "f", "kind": "ok", "label": "l"}
		for k, v := range over {
			m[k] = v
		}
		return m
	}
	cases := []struct {
		name    string
		raw     []json.RawMessage
		wantSub string
	}{
		{"too many", []json.RawMessage{
			rawEl(t, chip(map[string]any{"id": "el-a"})), rawEl(t, chip(map[string]any{"id": "el-b"})),
			rawEl(t, chip(map[string]any{"id": "el-c"})), rawEl(t, chip(map[string]any{"id": "el-d"})),
			rawEl(t, chip(map[string]any{"id": "el-e"})),
		}, "at most 4"},
		{"bad id", []json.RawMessage{rawEl(t, chip(map[string]any{"id": "nope"}))}, "must match"},
		{"dup id", []json.RawMessage{rawEl(t, chip(map[string]any{"id": "el-a"})), rawEl(t, chip(map[string]any{"id": "el-a"}))}, "duplicate id"},
		{"no fallback", []json.RawMessage{rawEl(t, map[string]any{"el": "chip", "id": "el-a", "kind": "ok", "label": "l"})}, "fallback is required"},
		{"long fallback", []json.RawMessage{rawEl(t, chip(map[string]any{"fallback": strings.Repeat("x", 201)}))}, "fallback exceeds"},
		{"unknown el", []json.RawMessage{rawEl(t, map[string]any{"el": "widget", "id": "el-a", "fallback": "f"})}, "unknown el"},
		{"bad chip kind", []json.RawMessage{rawEl(t, chip(map[string]any{"kind": "purple"}))}, "chip kind"},
		{"bad job state", []json.RawMessage{rawEl(t, map[string]any{"el": "job", "id": "el-a", "fallback": "f", "title": "t", "state": "sideways"})}, "job state"},
		{"job progress oob", []json.RawMessage{rawEl(t, map[string]any{"el": "job", "id": "el-a", "fallback": "f", "title": "t", "state": "running", "progress": 5})}, "progress"},
		{"decision no options", []json.RawMessage{rawEl(t, map[string]any{"el": "decision", "id": "el-a", "fallback": "f", "prompt": "p", "options": []any{}})}, "options"},
		{"checklist over 12", []json.RawMessage{rawEl(t, checklistWith(t, 13))}, "items"},
		{"decision option key grammar", []json.RawMessage{rawEl(t, map[string]any{"el": "decision", "id": "el-a", "fallback": "f", "prompt": "p", "options": []any{map[string]any{"key": "bad key!", "label": "l"}}})}, "must match"},
		{"decision option key too long", []json.RawMessage{rawEl(t, map[string]any{"el": "decision", "id": "el-a", "fallback": "f", "prompt": "p", "options": []any{map[string]any{"key": strings.Repeat("k", 33), "label": "l"}}})}, "must match"},
		{"checklist item key grammar", []json.RawMessage{rawEl(t, map[string]any{"el": "checklist", "id": "el-a", "fallback": "f", "title": "t", "items": []any{map[string]any{"key": "pill\n", "label": "l"}}})}, "must match"},
		{"unknown field", []json.RawMessage{json.RawMessage(`{"el":"chip","id":"el-a","fallback":"f","kind":"ok","label":"l","bogus":1}`)}, "invalid element"},
		{"malformed json", []json.RawMessage{json.RawMessage(`{"el":"chip"`)}, "invalid element"},
		{"element too big", []json.RawMessage{rawEl(t, chip(map[string]any{"value": strings.Repeat("y", 2100)}))}, "exceeds"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := validateElements(tc.raw); err == nil {
				t.Fatalf("expected error containing %q, got nil", tc.wantSub)
			} else if !strings.Contains(err.Error(), tc.wantSub) {
				t.Fatalf("error %q does not contain %q", err.Error(), tc.wantSub)
			}
		})
	}
}

func TestValidateElementsLegalQuartetPasses(t *testing.T) {
	// The maximum legal shape: 4 chunky-but-under-2-KiB elements.
	big := strings.Repeat("z", 1800)
	els := make([]json.RawMessage, 0, 4)
	for _, id := range []string{"el-a", "el-b", "el-c", "el-d"} {
		els = append(els, rawEl(t, map[string]any{"el": "chip", "id": id, "fallback": "f", "kind": "ok", "label": "l", "value": big}))
	}
	if _, err := validateElements(els); err != nil {
		t.Fatalf("legal quartet rejected: %v", err)
	}
}

func checklistWith(t *testing.T, n int) map[string]any {
	items := make([]any, 0, n)
	for i := 0; i < n; i++ {
		items = append(items, map[string]any{"key": "k" + string(rune('a'+i)), "label": "l", "done": false})
	}
	return map[string]any{"el": "checklist", "id": "el-a", "fallback": "f", "title": "t", "items": items}
}

// --- reply / edit element pass-through ---------------------------------------

func TestReplyEmitsElementsOnLastBubble(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	el := rawEl(t, map[string]any{"el": "chip", "id": "el-t", "fallback": "tests 3/3", "kind": "ok", "label": "tests", "value": "3/3"})
	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: dev, Bubbles: []string{"one", "two"}, Elements: []json.RawMessage{el}})
	if isErr {
		t.Fatalf("reply failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 2 {
		t.Fatalf("want 2 msg frames, got %d: %+v", len(items), items)
	}
	if _, has := items[0]["elements"]; has {
		t.Error("first bubble should carry no elements")
	}
	els, ok := items[1]["elements"].([]any)
	if !ok || len(els) != 1 {
		t.Fatalf("last bubble should carry 1 element, got %v", items[1]["elements"])
	}
}

func TestReplyElementsOnlyStandaloneMessage(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	el := rawEl(t, map[string]any{"el": "approval", "id": "el-d", "fallback": "approve deploy?", "title": "deploy?"})
	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: dev, Elements: []json.RawMessage{el}})
	if isErr {
		t.Fatalf("reply failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 {
		t.Fatalf("want 1 standalone frame, got %d", len(items))
	}
	// E6: element-only messages carry the synthesized fallback join as text —
	// that is what old clients render and what a push preview shows.
	if items[0]["text"] != "approve deploy?" {
		t.Errorf("standalone element message text = %q, want the synthesized fallback", items[0]["text"])
	}
	if _, has := items[0]["elements"]; !has {
		t.Error("standalone message missing elements")
	}
}

func TestReplyRejectsBadElementsWithoutEmitting(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	bad := json.RawMessage(`{"el":"chip","id":"bad","fallback":"f","kind":"ok","label":"l"}`)
	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: dev, Text: "hi", Elements: []json.RawMessage{bad}})
	if !isErr {
		t.Fatal("expected reply failure on bad element id")
	}
	if !strings.Contains(msg, "must match") {
		t.Errorf("error should name the id rule: %s", msg)
	}
	if items := drainItems(t, sub); len(items) != 0 {
		t.Errorf("no frames should be emitted on validation failure, got %d", len(items))
	}
}

func TestEditMessageElementOnly(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	el := rawEl(t, map[string]any{"el": "job", "id": "el-b1", "fallback": "done", "title": "build", "state": "ok"})
	msg, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: dev, MessageID: "a-1", Elements: []json.RawMessage{el}})
	if isErr {
		t.Fatalf("edit failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 || items[0]["t"] != "edit" {
		t.Fatalf("want 1 edit frame, got %+v", items)
	}
	// E6: an element-only edit carries the synthesized fallback join as text
	// (old clients render it; the app suppresses the exact join).
	if items[0]["text"] != "done" {
		t.Errorf("element-only edit text = %q, want the synthesized fallback", items[0]["text"])
	}
	if _, has := items[0]["elements"]; !has {
		t.Error("edit frame missing elements")
	}
}

func TestEditMessageRequiresTextOrElements(t *testing.T) {
	_, tools, dev, _ := activeHarness(t)
	msg, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: dev, MessageID: "a-1"})
	if !isErr || !strings.Contains(msg, "text or elements") {
		t.Fatalf("expected text-or-elements error, got %q (isErr=%v)", msg, isErr)
	}
}

// TestFallbackUnicodeBoundary pins E5: the 200 limit is Unicode CODE POINTS on
// both sides — 200 emoji (400 UTF-16 units, 800 UTF-8 bytes) are legal; 201
// code points are not.
func TestFallbackUnicodeBoundary(t *testing.T) {
	emoji200 := strings.Repeat("🚀", 200)
	el := rawEl(t, map[string]any{"el": "chip", "id": "el-a", "fallback": emoji200, "kind": "ok", "label": "l"})
	if _, err := validateElements([]json.RawMessage{el}); err != nil {
		t.Fatalf("200-code-point fallback rejected: %v", err)
	}
	el = rawEl(t, map[string]any{"el": "chip", "id": "el-a", "fallback": emoji200 + "x", "kind": "ok", "label": "l"})
	if _, err := validateElements([]json.RawMessage{el}); err == nil {
		t.Fatal("201-code-point fallback accepted")
	}
}

// TestTruncateFallbackIncludesEllipsis pins E5's truncation rule: the result
// is at most 200 code points INCLUDING the ellipsis.
func TestTruncateFallbackIncludesEllipsis(t *testing.T) {
	long := strings.Repeat("🚀", 300)
	got := truncateFallback(long)
	if n := len([]rune(got)); n > maxFallbackRunes {
		t.Fatalf("truncated fallback is %d code points, max %d including the ellipsis", n, maxFallbackRunes)
	}
	if !strings.HasSuffix(got, "…") {
		t.Errorf("truncated fallback should end with an ellipsis: %q", got)
	}
	exact := strings.Repeat("y", maxFallbackRunes)
	if truncateFallback(exact) != exact {
		t.Error("exactly-200 fallback must pass through untouched")
	}
}

// TestReplyPayloadSizeCap pins E9: the COMPLETE msg payload (text + elements +
// all fields) is capped at 16 KiB, rejected before anything is emitted.
func TestReplyPayloadSizeCap(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	el := rawEl(t, map[string]any{"el": "chip", "id": "el-a", "fallback": "f", "kind": "ok", "label": "l", "value": strings.Repeat("v", 1500)})
	bigText := strings.Repeat("t", 15<<10)
	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: dev, Text: bigText, Elements: []json.RawMessage{el}})
	if !isErr {
		t.Fatal("15KiB text + 1.5KiB element must exceed the 16KiB payload cap")
	}
	if !strings.Contains(msg, "16384") {
		t.Errorf("error should state the byte limit: %s", msg)
	}
	if items := drainItems(t, sub); len(items) != 0 {
		t.Errorf("nothing may be emitted on a payload-size rejection, got %d frames", len(items))
	}
}

// TestEditPayloadSizeCap: same rule on the edit path.
func TestEditPayloadSizeCap(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	el := rawEl(t, map[string]any{"el": "chip", "id": "el-a", "fallback": "f", "kind": "ok", "label": "l", "value": strings.Repeat("v", 1500)})
	msg, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: dev, MessageID: "a-1", Text: strings.Repeat("t", 15<<10), Elements: []json.RawMessage{el}})
	if !isErr {
		t.Fatalf("oversized edit accepted: %s", msg)
	}
	if items := drainItems(t, sub); len(items) != 0 {
		t.Errorf("nothing may be emitted on a payload-size rejection, got %d frames", len(items))
	}
}

// TestEditMergeNeverExceedsFourElements pins the E8 belt: the box rejects an
// edit whose id-matched merge would grow a message past 4 elements, while a
// same-id replacement always passes.
func TestEditMergeNeverExceedsFourElements(t *testing.T) {
	_, tools, dev, sub := activeHarness(t)
	chip := func(id string) json.RawMessage {
		return rawEl(t, map[string]any{"el": "chip", "id": id, "fallback": "f", "kind": "ok", "label": "l"})
	}
	msg, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: dev, Text: "four chips",
		Elements: []json.RawMessage{chip("el-a"), chip("el-b"), chip("el-c"), chip("el-d")}})
	if isErr {
		t.Fatalf("reply failed: %s", msg)
	}
	items := drainItems(t, sub)
	if len(items) != 1 {
		t.Fatalf("want 1 msg, got %d", len(items))
	}
	msgID := items[0]["id"].(string)

	// same-id replacement: fine
	if out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: dev, MessageID: msgID, Elements: []json.RawMessage{chip("el-a")}}); isErr {
		t.Fatalf("same-id replacement rejected: %s", out)
	}
	// a new fifth id: rejected
	out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: dev, MessageID: msgID, Elements: []json.RawMessage{chip("el-e")}})
	if !isErr {
		t.Fatal("edit growing the message to 5 elements must be rejected")
	}
	if !strings.Contains(out, "merge cap") {
		t.Errorf("error should explain the merge cap: %s", out)
	}
}
