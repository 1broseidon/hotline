package app

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// FB92 — box-authored resolution edits. When an inbound /el action parses, the
// box synthesizes a confirming edit that records the resolution into the target
// element so cold hydration/replay renders the card settled. These tests cover
// the original spec's Tests section and REV 1's additional required tests.

// --- fixtures ----------------------------------------------------------------

func fb92Decision(id string, keys ...string) Element {
	opts := make([]decisionOption, len(keys))
	for i, k := range keys {
		opts[i] = decisionOption{Key: k, Label: "opt " + k}
	}
	return Element{El: "decision", ID: id, Fallback: "pick one", Prompt: "which?", Options: opts}
}

func fb92Approval(id string) Element {
	return Element{El: "approval", ID: id, Fallback: "approve?", Title: "deploy?"}
}

func fb92Checklist(id string, keys ...string) Element {
	items := make([]checklistItem, len(keys))
	for i, k := range keys {
		items[i] = checklistItem{Key: k, Label: "item " + k}
	}
	return Element{El: "checklist", ID: id, Fallback: "verify", Title: "verify", Items: items}
}

func fb92Chip(id string) Element {
	return Element{El: "chip", ID: id, Fallback: "sib", Kind: "ok", Label: "sib"}
}

// emitElMsg appends a real msg frame carrying els through the emit path (so the
// projection folds it, exactly as a live agent reply would) and returns its id.
func emitElMsg(srv *Server, els ...Element) string {
	var id string
	srv.emit(func(seq uint64) []byte {
		id = fmt.Sprintf("a-%d", seq)
		return msgFrame(seq, id, synthesizedText(els), nil, "", nil, els)
	})
	return id
}

// elActionFrame builds the device_send an /el tap rides: a "send" whose text is
// the zero-width-space marker + one-line JSON action object.
func elActionFrame(cid, msg, el, act string, v any) deviceSendFrame {
	inner := map[string]any{"msg": msg, "el": el, "act": act}
	if v != nil {
		inner["v"] = v
	}
	b, _ := json.Marshal(inner)
	text := elementActionMarker + string(b)
	payload, _ := json.Marshal(map[string]string{"t": "send", "text": text})
	return deviceSendFrame{T: "device_send", CID: cid, Payload: json.RawMessage(payload)}
}

// outboxEdits returns every edit frame in the outbox whose id == msgID, decoded.
func outboxEdits(t *testing.T, srv *Server, msgID string) []map[string]any {
	t.Helper()
	var out []map[string]any
	for _, data := range srv.outbox.since(0) {
		var m map[string]any
		if json.Unmarshal(data, &m) != nil {
			continue
		}
		if m["t"] == "edit" && m["id"] == msgID {
			out = append(out, m)
		}
	}
	return out
}

// oneResolutionEdit asserts exactly one resolution edit exists for msgID and
// returns it.
func oneResolutionEdit(t *testing.T, srv *Server, msgID string) map[string]any {
	t.Helper()
	edits := outboxEdits(t, srv, msgID)
	if len(edits) != 1 {
		t.Fatalf("want exactly 1 resolution edit for %s, got %d: %+v", msgID, len(edits), edits)
	}
	return edits[0]
}

// editElement pulls the single element out of a resolution edit frame, asserting
// the singleton shape (F3).
func editElement(t *testing.T, edit map[string]any) map[string]any {
	t.Helper()
	els, ok := edit["elements"].([]any)
	if !ok || len(els) != 1 {
		t.Fatalf("resolution edit must carry exactly one element, got %v", edit["elements"])
	}
	el, ok := els[0].(map[string]any)
	if !ok {
		t.Fatalf("element is not an object: %v", els[0])
	}
	return el
}

// fb92Harness builds a server with one active device + a bound fake sink so the
// full handleDeviceSend forward path runs, and returns the captured-forward sink.
func fb92Harness(t *testing.T) (*Server, string, *fakeSink) {
	t.Helper()
	srv, _, dev, _ := activeHarness(t)
	sink := newFakeSink()
	srv.bindSink(sink)
	return srv, dev, sink
}

func drainCaptures(sink *fakeSink) []capture {
	var out []capture
	for {
		select {
		case c := <-sink.ch:
			out = append(out, c)
		default:
			return out
		}
	}
}

// --- applyResolution unit matrix ---------------------------------------------

func TestFB92ApplyResolutionMatrix(t *testing.T) {
	approved, denied := "approved", "denied"
	optB := "b"
	cases := []struct {
		name    string
		el      Element
		act     string
		v       any
		changed bool
		verify  func(t *testing.T, out Element)
	}{
		{"pick valid", fb92Decision("el-d", "a", "b"), "pick", "a", true, func(t *testing.T, o Element) {
			if o.ChosenKey == nil || *o.ChosenKey != "a" {
				t.Fatalf("chosenKey = %v, want a", o.ChosenKey)
			}
		}},
		{"pick unmatched key", fb92Decision("el-d", "a", "b"), "pick", "zzz", false, nil},
		{"pick same key no-change", func() Element { e := fb92Decision("el-d", "a", "b"); e.ChosenKey = &optB; return e }(), "pick", "b", false, nil},
		{"pick different key re-edits", func() Element { e := fb92Decision("el-d", "a", "b"); e.ChosenKey = &optB; return e }(), "pick", "a", true, func(t *testing.T, o Element) {
			if o.ChosenKey == nil || *o.ChosenKey != "a" {
				t.Fatalf("re-pick chosenKey = %v, want a", o.ChosenKey)
			}
		}},
		{"pick on approval wrong type", fb92Approval("el-a"), "pick", "a", false, nil},
		{"approve", fb92Approval("el-a"), "approve", nil, true, func(t *testing.T, o Element) {
			if o.Resolved == nil || *o.Resolved != "approved" {
				t.Fatalf("resolved = %v, want approved", o.Resolved)
			}
		}},
		{"deny", fb92Approval("el-a"), "deny", nil, true, func(t *testing.T, o Element) {
			if o.Resolved == nil || *o.Resolved != "denied" {
				t.Fatalf("resolved = %v, want denied", o.Resolved)
			}
		}},
		{"approve already approved no-change", func() Element { e := fb92Approval("el-a"); e.Resolved = &approved; return e }(), "approve", nil, false, nil},
		{"deny already denied no-change", func() Element { e := fb92Approval("el-a"); e.Resolved = &denied; return e }(), "deny", nil, false, nil},
		{"approve flips denied", func() Element { e := fb92Approval("el-a"); e.Resolved = &denied; return e }(), "approve", nil, true, func(t *testing.T, o Element) {
			if o.Resolved == nil || *o.Resolved != "approved" {
				t.Fatalf("resolved = %v, want approved", o.Resolved)
			}
		}},
		{"approve on decision wrong type", fb92Decision("el-d", "a"), "approve", nil, false, nil},
		{"toggle merges", fb92Checklist("el-c", "x", "y"), "toggle", map[string]bool{"x": true}, true, func(t *testing.T, o Element) {
			if !o.Items[0].Done || o.Items[1].Done {
				t.Fatalf("items = %+v, want only x done", o.Items)
			}
		}},
		{"toggle no matching key", fb92Checklist("el-c", "x"), "toggle", map[string]bool{"nope": true}, false, nil},
		{"toggle same value no-change", func() Element { e := fb92Checklist("el-c", "x"); e.Items[0].Done = true; return e }(), "toggle", map[string]bool{"x": true}, false, nil},
		{"toggle on decision wrong type", fb92Decision("el-d", "a"), "toggle", map[string]bool{"a": true}, false, nil},
		{"unknown act", fb92Decision("el-d", "a"), "frobnicate", nil, false, nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var raw json.RawMessage
			if tc.v != nil {
				raw, _ = json.Marshal(tc.v)
			}
			out, changed := applyResolution(tc.el, elementAction{Msg: "a-1", El: tc.el.ID, Act: tc.act, V: raw})
			if changed != tc.changed {
				t.Fatalf("changed = %v, want %v", changed, tc.changed)
			}
			if changed && tc.verify != nil {
				tc.verify(t, out)
			}
		})
	}
}

// TestFB92ToggleDoesNotAliasProjection proves a toggle mutates only a copy: the
// stored projection element's Items slice is never disturbed.
func TestFB92ToggleDoesNotAliasProjection(t *testing.T) {
	stored := fb92Checklist("el-c", "x", "y")
	v, _ := json.Marshal(map[string]bool{"x": true, "y": true})
	out, changed := applyResolution(stored, elementAction{Act: "toggle", V: v})
	if !changed {
		t.Fatal("expected change")
	}
	if stored.Items[0].Done || stored.Items[1].Done {
		t.Fatalf("source items mutated: %+v", stored.Items)
	}
	if !out.Items[0].Done || !out.Items[1].Done {
		t.Fatalf("output items not set: %+v", out.Items)
	}
}

// --- projection fold + seed --------------------------------------------------

// TestFB92ProjectionFoldsLatestFrame proves the projection tracks the folded
// element state across a delta edit, and that a later text-only / sibling-only
// edit does NOT hide or clobber the target payload (REV 1 additional test).
func TestFB92ProjectionSiblingEditDoesNotHideTarget(t *testing.T) {
	dec := fb92Decision("el-d", "a", "b")
	sib := fb92Chip("el-s")

	t.Run("sibling-only edit", func(t *testing.T) {
		p := newElementProjection()
		p.foldFrame(msgFrame(1, "a-1", "real body", nil, "", nil, []Element{dec, sib}))
		// a later edit touches ONLY the sibling
		sib2 := fb92Chip("el-s")
		sib2.Label = "changed"
		p.foldFrame(editFrame(2, "a-1", synthesizedText([]Element{sib2}), []Element{sib2}))
		got, ok := p.get("a-1", "el-d")
		if !ok {
			t.Fatal("target el-d vanished after sibling-only edit")
		}
		if got.El != "decision" || len(got.Options) != 2 {
			t.Fatalf("target payload corrupted: %+v", got)
		}
	})

	t.Run("text-only edit", func(t *testing.T) {
		p := newElementProjection()
		p.foldFrame(msgFrame(1, "a-1", "real body", nil, "", nil, []Element{dec, sib}))
		// a later text-only edit carries NO elements
		p.foldFrame(editFrame(2, "a-1", "edited body", nil))
		got, ok := p.get("a-1", "el-d")
		if !ok {
			t.Fatal("target el-d vanished after text-only edit")
		}
		if got.El != "decision" || len(got.Options) != 2 {
			t.Fatalf("target payload corrupted: %+v", got)
		}
		if _, ok := p.get("a-1", "el-s"); !ok {
			t.Fatal("sibling el-s vanished after text-only edit")
		}
	})
}

// TestFB92OrphanEditNotSeeded proves an edit whose message id was never seen as a
// msg frame creates NO projection state (sol F2/F4 blocker): the app ignores such
// edits, so the box must too. An action against it is a projection miss →
// forward-only, no synthesized edit.
func TestFB92OrphanEditNotSeeded(t *testing.T) {
	// unit: an orphan edit does not create an entry
	p := newElementProjection()
	dec := fb92Decision("el-d", "a", "b")
	p.foldFrame(editFrame(1, "a-777", synthesizedText([]Element{dec}), []Element{dec}))
	if _, ok := p.get("a-777", "el-d"); ok {
		t.Fatal("orphan edit seeded a phantom projection entry")
	}

	// integration: action against an orphan-edited message → no edit, still forwards
	srv, dev, sink := fb92Harness(t)
	orphan := "a-777"
	srv.emit(func(seq uint64) []byte { return editFrame(seq, orphan, synthesizedText([]Element{dec}), []Element{dec}) })
	editsBefore := len(outboxEdits(t, srv, orphan)) // the orphan edit we planted
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000a01", orphan, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	if edits := outboxEdits(t, srv, orphan); len(edits) != editsBefore {
		t.Fatalf("orphan-edited message must not synthesize a resolution edit, got %d new", len(edits)-editsBefore)
	}
	if caps := drainCaptures(sink); len(caps) != 1 {
		t.Fatalf("action must still forward, got %d captures", len(caps))
	}
}

// TestFB92FifthElementDropped proves the projection honors the 4-per-message cap
// on fold: a 5th new id from an edit is dropped (matching wire.ts merge), so an
// action against that 5th id is a projection miss → no edit, still forwarded.
func TestFB92FifthElementDropped(t *testing.T) {
	// unit: 5th new id dropped, same-id replace still folds
	p := newElementProjection()
	full := []Element{fb92Chip("el-1"), fb92Chip("el-2"), fb92Chip("el-3"), fb92Decision("el-d", "a", "b")}
	p.foldFrame(msgFrame(1, "a-1", "t", nil, "", nil, full))
	// an edit introducing a NEW 5th id must be dropped
	fifth := fb92Decision("el-5", "x", "y")
	p.foldFrame(editFrame(2, "a-1", synthesizedText([]Element{fifth}), []Element{fifth}))
	if _, ok := p.get("a-1", "el-5"); ok {
		t.Fatal("5th new element must be dropped (cap 4), but projection tracked it")
	}
	// same-id replace on an existing id still folds
	resolved := fb92Decision("el-d", "a", "b")
	rk := "a"
	resolved.ChosenKey = &rk
	p.foldFrame(editFrame(3, "a-1", synthesizedText([]Element{resolved}), []Element{resolved}))
	got, ok := p.get("a-1", "el-d")
	if !ok || got.ChosenKey == nil || *got.ChosenKey != "a" {
		t.Fatalf("same-id replace did not fold: %+v ok=%v", got, ok)
	}

	// integration: action against the dropped 5th id → no synthesized edit
	srv, dev, sink := fb92Harness(t)
	msgID := emitElMsg(srv, full...)
	srv.emit(func(seq uint64) []byte { return editFrame(seq, msgID, synthesizedText([]Element{fifth}), []Element{fifth}) })
	editsBefore := len(outboxEdits(t, srv, msgID)) // the planted 5th-id edit
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000a02", msgID, "el-5", "pick", "x")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	if edits := outboxEdits(t, srv, msgID); len(edits) != editsBefore {
		t.Fatalf("action against dropped 5th id must not synthesize, got %d new", len(edits)-editsBefore)
	}
	if caps := drainCaptures(sink); len(caps) != 1 {
		t.Fatalf("action must still forward, got %d captures", len(caps))
	}
}

// TestFB92ProjectionDepthBoundEvicts proves the projection is bounded: past
// projectionMaxMessages tracked messages the oldest is evicted (FIFO), and an
// action against an evicted message is a projection miss → forward-only.
func TestFB92ProjectionDepthBoundEvicts(t *testing.T) {
	p := newElementProjection()
	oldest := "a-oldest"
	p.foldFrame(msgFrame(1, oldest, "t", nil, "", nil, []Element{fb92Decision("el-d", "a", "b")}))
	for i := 0; i < projectionMaxMessages; i++ { // one more than the cap total
		id := fmt.Sprintf("a-fill-%d", i)
		p.foldFrame(msgFrame(uint64(i+2), id, "t", nil, "", nil, []Element{fb92Chip("el-c")}))
	}
	if _, ok := p.get(oldest, "el-d"); ok {
		t.Fatal("oldest tracked message should have been evicted past the cap")
	}
	newest := fmt.Sprintf("a-fill-%d", projectionMaxMessages-1)
	if _, ok := p.get(newest, "el-c"); !ok {
		t.Fatal("newest tracked message must survive eviction")
	}
}

// TestFB92SeedFromFullJournalBeats500Tail proves the projection seeds from the
// COMPLETE journal (not the 500-record findByID tail): a target older than 500
// unrelated records still resolves after a restart, and its action produces an
// edit (REV 1 additional test).
func TestFB92SeedFromFullJournalBeats500Tail(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, StateRoot: dir, TranscriptFile: dir + "/transcript.jsonl", AccessFile: dir + "/access.json"}

	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	msgID := emitElMsg(srv, fb92Decision("el-old", "a", "b"))
	// bury it under >500 unrelated journal records
	for i := 0; i < 600; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, fmt.Sprintf("a-%d", seq), "filler", nil, "", nil, nil) })
	}
	srv.outbox.close()

	// findByID's bounded tail (ring window + last ~500 journal records) can NOT see
	// the old frame — 600 unrelated records bury it beyond both. This must hold
	// unconditionally: it is the whole point of maintaining a full-journal-seeded
	// projection instead of reusing findByID.
	if _, ok := srv.outbox.findByID(msgID); ok {
		t.Fatalf("precondition failed: %s still resolvable via findByID after 600 records", msgID)
	}

	// ...but a fresh server seeds the projection from the WHOLE journal.
	srv2 := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv2.initErr != nil {
		t.Fatal(srv2.initErr)
	}
	defer srv2.outbox.close()
	if _, ok := srv2.elementProj.get(msgID, "el-old"); !ok {
		t.Fatalf("projection did not resolve %s/el-old from full-journal seed", msgID)
	}
	setActiveDevice(t, srv2)
	srv2.bindSink(newFakeSink())
	if err := srv2.handleDeviceSend(context.Background(), "dev-testdevice01", elActionFrame("cid-0000000000000901", msgID, "el-old", "pick", "b")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	edit := oneResolutionEdit(t, srv2, msgID)
	if editElement(t, edit)["chosenKey"] != "b" {
		t.Fatalf("resolved edit missing chosenKey=b: %+v", edit)
	}
}

// --- full handleDeviceSend path ----------------------------------------------

// setActiveDevice makes dev-testdevice01 an active device with a provisioned
// mailbox on a bare server (used when a test constructs its own server rather
// than going through activeHarness).
func setActiveDevice(t *testing.T, srv *Server) {
	t.Helper()
	const dev = "dev-testdevice01"
	srv.store.st.Rooms["room1"] = RoomRecord{ID: "room1"}
	srv.store.st.CurrentRoom = "room1"
	srv.store.st.Devices[dev] = DeviceRecord{ID: dev, Room: "room1", SecretHash: "x", State: DeviceActive}
	srv.store.mu.Lock()
	_ = srv.store.saveLocked()
	srv.store.mu.Unlock()
	srv.mailbox.disk.Devices[dev] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	if _, _, _, _, err := srv.mailbox.stateAndSubscribe(dev); err != nil {
		t.Fatal(err)
	}
}

func TestFB92PickSetsChosenKeyAndForwards(t *testing.T) {
	srv, dev, sink := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000001", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	edit := oneResolutionEdit(t, srv, msgID)
	el := editElement(t, edit)
	if el["chosenKey"] != "a" {
		t.Fatalf("chosenKey = %v, want a", el["chosenKey"])
	}
	// the action still forwarded to the agent (structured summary)
	caps := drainCaptures(sink)
	if len(caps) != 1 || !strings.Contains(caps[0].content, "chose a") {
		t.Fatalf("want one forward summarizing the pick, got %+v", caps)
	}
	if caps[0].meta["kind"] != "element_action" {
		t.Fatalf("forward kind = %q, want element_action", caps[0].meta["kind"])
	}
}

func TestFB92ApproveDenySetResolved(t *testing.T) {
	for _, tc := range []struct{ act, want string }{{"approve", "approved"}, {"deny", "denied"}} {
		t.Run(tc.act, func(t *testing.T) {
			srv, dev, _ := fb92Harness(t)
			msgID := emitElMsg(srv, fb92Approval("el-a"))
			if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000002", msgID, "el-a", tc.act, nil)); err != nil {
				t.Fatalf("handleDeviceSend: %v", err)
			}
			el := editElement(t, oneResolutionEdit(t, srv, msgID))
			if el["resolved"] != tc.want {
				t.Fatalf("resolved = %v, want %v", el["resolved"], tc.want)
			}
		})
	}
}

func TestFB92ToggleMergesDone(t *testing.T) {
	srv, dev, _ := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Checklist("el-c", "x", "y", "z"))
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000003", msgID, "el-c", "toggle", map[string]bool{"x": true, "z": true})); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	el := editElement(t, oneResolutionEdit(t, srv, msgID))
	items, _ := el["items"].([]any)
	if len(items) != 3 {
		t.Fatalf("want 3 items in merged edit, got %v", el["items"])
	}
	done := map[string]bool{}
	for _, it := range items {
		m := it.(map[string]any)
		done[m["key"].(string)], _ = m["done"].(bool)
	}
	if !done["x"] || done["y"] || !done["z"] {
		t.Fatalf("merged done flags wrong: %+v", done)
	}
}

// TestFB92NoEditButStillForwards covers every no-edit-but-forward case: invalid
// option key, wrong-type act, unknown message, unknown element id.
func TestFB92NoEditButStillForwards(t *testing.T) {
	cases := []struct {
		name       string
		setup      func(srv *Server) (msgID string) // returns the msg id the action targets
		el, act    string
		v          any
		targetMsg  string // "" ⇒ use setup's msgID
		wantEditOn string // the id to assert has no edit; "" ⇒ setup's msgID
	}{
		{name: "invalid option key", setup: func(s *Server) string { return emitElMsg(s, fb92Decision("el-d", "a", "b")) }, el: "el-d", act: "pick", v: "zzz"},
		{name: "wrong-type act", setup: func(s *Server) string { return emitElMsg(s, fb92Approval("el-a")) }, el: "el-a", act: "pick", v: "a"},
		{name: "unknown element id", setup: func(s *Server) string { return emitElMsg(s, fb92Decision("el-d", "a")) }, el: "el-nope", act: "pick", v: "a"},
		{name: "unknown message id", setup: func(s *Server) string { emitElMsg(s, fb92Decision("el-d", "a")); return "a-99999" }, el: "el-d", act: "pick", v: "a"},
	}
	for i, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			srv, dev, sink := fb92Harness(t)
			msgID := tc.setup(srv)
			cid := fmt.Sprintf("cid-00000000000010%02d", i)
			if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame(cid, msgID, tc.el, tc.act, tc.v)); err != nil {
				t.Fatalf("handleDeviceSend: %v", err)
			}
			if edits := outboxEdits(t, srv, msgID); len(edits) != 0 {
				t.Fatalf("expected NO resolution edit, got %+v", edits)
			}
			if caps := drainCaptures(sink); len(caps) != 1 {
				t.Fatalf("action must still forward exactly once, got %d captures", len(caps))
			}
		})
	}
}

// TestFB92DuplicateSameValueOneEdit proves EDIT-idempotency: re-tapping the same
// value (fresh CID, as every real tap is) yields no second edit.
func TestFB92DuplicateSameValueOneEdit(t *testing.T) {
	srv, dev, sink := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	for _, cid := range []string{"cid-0000000000000201", "cid-0000000000000202"} {
		if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame(cid, msgID, "el-d", "pick", "a")); err != nil {
			t.Fatalf("handleDeviceSend: %v", err)
		}
	}
	if edits := outboxEdits(t, srv, msgID); len(edits) != 1 {
		t.Fatalf("same-value re-tap must yield ONE edit, got %d", len(edits))
	}
	if caps := drainCaptures(sink); len(caps) != 2 {
		t.Fatalf("each tap still forwards: want 2 captures, got %d", len(caps))
	}
}

// TestFB92RePickDifferentKeyReEdits proves last-writer-by-append-order: a second
// pick of a DIFFERENT key emits a second edit whose replay fold wins.
func TestFB92RePickDifferentKeyReEdits(t *testing.T) {
	srv, dev, _ := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000301", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("first pick: %v", err)
	}
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000302", msgID, "el-d", "pick", "b")); err != nil {
		t.Fatalf("second pick: %v", err)
	}
	edits := outboxEdits(t, srv, msgID)
	if len(edits) != 2 {
		t.Fatalf("want 2 edits (a then b), got %d", len(edits))
	}
	// fold the whole outbox: the resolved payload is the LAST writer, b.
	if k := foldedChosenKey(t, srv, msgID, "el-d"); k != "b" {
		t.Fatalf("replay fold chosenKey = %q, want b (last writer)", k)
	}
}

// TestFB92ActionAppliesToLatestAgentEdit proves the mutation applies to the LATEST
// folded payload, not the original frame: an agent edit re-chooses the element's
// options before the action lands (Mechanics #1 / F2).
func TestFB92ActionAppliesToLatestAgentEdit(t *testing.T) {
	srv, dev, _ := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	// agent edits the decision to a NEW option set (only c/d valid now)
	newDec := fb92Decision("el-d", "c", "d")
	srv.emit(func(seq uint64) []byte { return editFrame(seq, msgID, synthesizedText([]Element{newDec}), []Element{newDec}) })
	editsBefore := len(outboxEdits(t, srv, msgID)) // the agent edit itself
	// picking "a" (valid on the ORIGINAL, gone on the LATEST) must NOT resolve
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000401", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("stale pick: %v", err)
	}
	if edits := outboxEdits(t, srv, msgID); len(edits) != editsBefore {
		t.Fatalf("stale-option pick must not add a resolution edit against latest payload, got %+v", edits)
	}
	// picking "c" (valid on the latest) resolves
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000402", msgID, "el-d", "pick", "c")); err != nil {
		t.Fatalf("fresh pick: %v", err)
	}
	if k := foldedChosenKey(t, srv, msgID, "el-d"); k != "c" {
		t.Fatalf("chosenKey = %q, want c", k)
	}
}

// foldedChosenKey replays the whole outbox through a fresh projection and returns
// the target's chosenKey — the cold-replay ground truth (events, not state).
func foldedChosenKey(t *testing.T, srv *Server, msgID, elID string) string {
	t.Helper()
	p := newElementProjection()
	for _, data := range srv.outbox.since(0) {
		p.foldFrame(data)
	}
	el, ok := p.get(msgID, elID)
	if !ok || el.ChosenKey == nil {
		return ""
	}
	return *el.ChosenKey
}

// TestFB92ColdReplayFoldsToResolved proves since/page replay folds original + edit
// to the resolved payload (spec cold-replay test + F: events not materialized).
func TestFB92ColdReplayFoldsToResolved(t *testing.T) {
	srv, dev, _ := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000501", msgID, "el-d", "pick", "b")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	// since(0) replay must carry BOTH events — the original msg AND the edit — in
	// order. An edit-only replay (msg missing, or edit before msg) is a hydration
	// bug and must fail this test: the resolved payload is a FOLD of two events, not
	// a materialized snapshot.
	assertMsgThenEdit(t, srv.outbox.since(0), msgID)
	if k := foldedChosenKey(t, srv, msgID, "el-d"); k != "b" {
		t.Fatalf("since-replay chosenKey = %q, want b", k)
	}

	// page() replay (journal-backed) carries the same two ordered events and folds
	// identically.
	pageFrames := srv.outbox.page(0, 1000)
	assertMsgThenEdit(t, pageFrames, msgID)
	p := newElementProjection()
	for _, data := range pageFrames {
		p.foldFrame(data)
	}
	el, ok := p.get(msgID, "el-d")
	if !ok || el.ChosenKey == nil || *el.ChosenKey != "b" {
		t.Fatalf("page-replay did not fold to chosenKey=b: %+v ok=%v", el, ok)
	}
}

// assertMsgThenEdit asserts the replay stream contains the original msg frame for
// msgID strictly before an edit frame for the same id (both present, correct
// order).
func assertMsgThenEdit(t *testing.T, frames [][]byte, msgID string) {
	t.Helper()
	msgAt, editAt := -1, -1
	for i, data := range frames {
		var m struct {
			T  string `json:"t"`
			ID string `json:"id"`
		}
		if json.Unmarshal(data, &m) != nil || m.ID != msgID {
			continue
		}
		if m.T == "msg" && msgAt < 0 {
			msgAt = i
		}
		if m.T == "edit" && editAt < 0 {
			editAt = i
		}
	}
	if msgAt < 0 {
		t.Fatalf("replay missing the original msg event for %s (edit-only replay)", msgID)
	}
	if editAt < 0 {
		t.Fatalf("replay missing the resolution edit event for %s", msgID)
	}
	if msgAt >= editAt {
		t.Fatalf("replay order wrong: msg at %d must precede edit at %d", msgAt, editAt)
	}
}

// TestFB92EditPrecedesEchoAndMessageIDMatches proves the resolution edit is
// appended BEFORE the CID-keyed echo (F6) and that the predicted harness
// message_id equals the actual sent.id (F1 — synthesis runs before the
// prediction, no deadlock).
func TestFB92EditPrecedesEchoAndMessageIDMatches(t *testing.T) {
	srv, dev, sink := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	decisionSeq := srv.outbox.cursor()
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000601", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	// walk the frames appended after the decision, in seq order
	var editSeq, echoSeq uint64
	var echoID string
	for _, data := range srv.outbox.since(decisionSeq) {
		var m struct {
			T   string `json:"t"`
			Seq uint64 `json:"seq"`
			ID  string `json:"id"`
			CID string `json:"cid"`
		}
		if json.Unmarshal(data, &m) != nil {
			continue
		}
		if m.T == "edit" && m.ID == msgID {
			editSeq = m.Seq
		}
		if m.T == "sent" && m.CID == "cid-0000000000000601" {
			echoSeq, echoID = m.Seq, m.ID
		}
	}
	if editSeq == 0 || echoSeq == 0 {
		t.Fatalf("missing edit (%d) or echo (%d)", editSeq, echoSeq)
	}
	if editSeq >= echoSeq {
		t.Fatalf("resolution edit seq %d must precede echo seq %d", editSeq, echoSeq)
	}
	// F1: harness message_id prediction == actual sent.id
	caps := drainCaptures(sink)
	if len(caps) != 1 {
		t.Fatalf("want 1 forward, got %d", len(caps))
	}
	if caps[0].meta["message_id"] != echoID {
		t.Fatalf("predicted message_id %q != actual sent.id %q", caps[0].meta["message_id"], echoID)
	}
}

// TestFB92CrashBeforeEchoNoDuplicateEdit models F6's crash window directly: the
// first attempt appends the resolution edit and then CRASHES before the CID-keyed
// echo persists (and before the CID is recorded). The client retries with the
// SAME CID; because the crash never recorded the CID, the retry runs the full
// path, re-runs synthesis to a no-change (the projection already folded the edit),
// and completes the keyed echo — exactly one edit total, no duplicate.
func TestFB92CrashBeforeEchoNoDuplicateEdit(t *testing.T) {
	srv, dev, sink := fb92Harness(t)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	const cid = "cid-0000000000000701"
	key := deliveryKey(dev, cid)
	act := elementAction{Msg: msgID, El: "el-d", Act: "pick", V: json.RawMessage(`"a"`)}

	// Crash model: drive ONLY the synthesis under deliveryMu (the edit is appended
	// + folded), then stop — no keyed echo, no CID recorded.
	srv.deliveryMu.Lock()
	srv.synthesizeResolutionEditLocked(act)
	srv.deliveryMu.Unlock()
	if len(outboxEdits(t, srv, msgID)) != 1 {
		t.Fatal("first attempt should append exactly one resolution edit")
	}
	if _, _, _, ok := srv.outbox.delivery(key); ok {
		t.Fatal("precondition: keyed echo must NOT be persisted yet (crash before echo)")
	}
	if srv.cid.seen(key) {
		t.Fatal("precondition: CID must not be recorded yet (crash before echo)")
	}

	// Retry with the SAME CID — a fresh key to the deduper, so the full path runs.
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame(cid, msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("retry: %v", err)
	}
	// Exactly one edit total — the retry's synthesis is a no-change.
	if edits := outboxEdits(t, srv, msgID); len(edits) != 1 {
		t.Fatalf("crash+retry must yield exactly ONE edit, got %d", len(edits))
	}
	// The retry completed the KEYED echo (not just any sent frame).
	_, data, _, ok := srv.outbox.delivery(key)
	if !ok {
		t.Fatal("retry must persist the CID-keyed echo")
	}
	var m struct {
		T   string `json:"t"`
		CID string `json:"cid"`
	}
	if json.Unmarshal(data, &m) != nil || m.T != "sent" || m.CID != cid {
		t.Fatalf("delivery(key) frame = %+v, want a sent echo carrying cid %s", m, cid)
	}
	if caps := drainCaptures(sink); len(caps) != 1 {
		t.Fatalf("the retry forwards exactly once, got %d", len(caps))
	}
}

// TestFB92SameValueDifferentDevicesOneEditNForwards proves same-value actions from
// different devices/CIDs coalesce to ONE edit but N forwards (F7).
func TestFB92SameValueDifferentDevicesOneEditNForwards(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 2)
	sink := newFakeSink()
	srv.bindSink(sink)
	msgID := emitElMsg(srv, fb92Decision("el-d", "a", "b"))
	for i, dev := range ids {
		cid := fmt.Sprintf("cid-00000000000008%02d", i)
		if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame(cid, msgID, "el-d", "pick", "a")); err != nil {
			t.Fatalf("device %d: %v", i, err)
		}
	}
	if edits := outboxEdits(t, srv, msgID); len(edits) != 1 {
		t.Fatalf("two devices same value → want 1 edit, got %d", len(edits))
	}
	if caps := drainCaptures(sink); len(caps) != 2 {
		t.Fatalf("want 2 forwards (one per device), got %d", len(caps))
	}
}

// TestFB92SingletonEditPreservesTextSiblingsAndCap proves the confirming edit
// carries EXACTLY ONE element with sentinel text, and that folding it back
// preserves sibling identity/order and the 4-element message (F3 + cap 4).
func TestFB92SingletonEditPreservesTextSiblingsAndCap(t *testing.T) {
	srv, dev, _ := fb92Harness(t)
	// a full 4-element message whose body is REAL semantic text (not a fallback
	// join): target decision second, three siblings around it, definite order.
	const realBody = "Here are your options — pick a plan:"
	s0 := fb92Chip("el-0")
	dec := fb92Decision("el-d", "a", "b")
	s2, s3 := fb92Chip("el-2"), fb92Chip("el-3")
	origEls := []Element{s0, dec, s2, s3}
	var msgID string
	srv.emit(func(seq uint64) []byte {
		msgID = fmt.Sprintf("a-%d", seq)
		return msgFrame(seq, msgID, realBody, nil, "", nil, origEls)
	})
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000801", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	edit := oneResolutionEdit(t, srv, msgID)
	// F3: exactly one element, and it is the target.
	el := editElement(t, edit)
	if el["id"] != "el-d" {
		t.Fatalf("edit element id = %v, want el-d", el["id"])
	}
	// F3: text is the preserve-text sentinel (synthesizedText of JUST the target),
	// never a copy of the message body.
	sentinel := synthesizedText([]Element{dec})
	editText, _ := edit["text"].(string)
	if editText != sentinel {
		t.Fatalf("edit text = %q, want sentinel %q", editText, sentinel)
	}
	if editText == realBody {
		t.Fatal("edit must NOT copy the message body")
	}

	// Model the app-side fold (wire.ts mergeElements + the sentinel→preserve-text
	// rule) on the raw frames and assert the ORDERED outcome verbatim.
	editEls := elementsOf(t, edit)
	mergedText := appFoldText(realBody, editText, editEls)
	if mergedText != realBody {
		t.Fatalf("app-fold text = %q, want the original body preserved verbatim", mergedText)
	}
	merged := appFoldElements(origEls, editEls)
	wantOrder := []string{"el-0", "el-d", "el-2", "el-3"}
	if len(merged) != len(wantOrder) {
		t.Fatalf("merged element count = %d, want %d", len(merged), len(wantOrder))
	}
	for i, want := range wantOrder {
		if merged[i].ID != want {
			t.Fatalf("merged element %d id = %q, want %q (order not preserved)", i, merged[i].ID, want)
		}
	}
	// siblings byte-identical; only the target gained chosenKey.
	for i, want := range wantOrder {
		if want == "el-d" {
			if merged[i].ChosenKey == nil || *merged[i].ChosenKey != "a" {
				t.Fatalf("target not resolved after merge: %+v", merged[i])
			}
			continue
		}
		if !elementsEqual(t, merged[i], origEls[i]) {
			t.Fatalf("sibling %s payload changed by the merge", want)
		}
	}
}

// elementsOf decodes the elements array of a decoded frame back into typed
// Elements (through the wire round-trip the app would receive).
func elementsOf(t *testing.T, frame map[string]any) []Element {
	t.Helper()
	raw, _ := json.Marshal(frame["elements"])
	var els []Element
	if err := json.Unmarshal(raw, &els); err != nil {
		t.Fatalf("decode elements: %v", err)
	}
	return els
}

// appFoldElements mirrors wire.ts mergeElements: same-id replaces in place, a new
// id appends, order preserved. It is the app-side truth the box edit must respect.
func appFoldElements(base, edit []Element) []Element {
	out := append([]Element(nil), base...)
	for _, e := range edit {
		replaced := false
		for i := range out {
			if out[i].ID == e.ID {
				out[i] = e
				replaced = true
				break
			}
		}
		if !replaced {
			out = append(out, e)
		}
	}
	return out
}

// appFoldText mirrors the app's preserve-text sentinel: an edit whose text equals
// the synthesized fallback of its own elements leaves the message body untouched.
func appFoldText(baseText, editText string, editEls []Element) string {
	if editText == synthesizedText(editEls) {
		return baseText
	}
	return editText
}

func elementsEqual(t *testing.T, a, b Element) bool {
	t.Helper()
	ab, _ := json.Marshal(a)
	bb, _ := json.Marshal(b)
	return string(ab) == string(bb)
}

// TestFB92RestartedElIndexCannotReject proves the box edit skips elIndex.applyEdit:
// a fresh (empty) elIndex — which would falsely reject a legit resolution — does
// not block the projection-verified edit (F4).
func TestFB92RestartedElIndexCannotReject(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, StateRoot: dir, TranscriptFile: dir + "/transcript.jsonl", AccessFile: dir + "/access.json"}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	// The real message carries the target decision el-d (plus one sibling).
	dec := fb92Decision("el-d", "a", "b")
	msgID := emitElMsg(srv, dec, fb92Chip("el-1"))
	srv.outbox.close()

	// restart: the projection re-seeds from the journal, but the elIndex does NOT.
	srv2 := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv2.initErr != nil {
		t.Fatal(srv2.initErr)
	}
	defer srv2.outbox.close()

	// Make the elIndex DIVERGENT: populate it with four OTHER ids for this message
	// (a plausible post-restart drift). The target el-d is NOT among them, so the
	// naive belt would count 4 existing + 1 new append = 5 > cap and REJECT the box
	// edit — exactly the false rejection the F4 skip exists to avoid.
	srv2.elIndex.record(msgID, []Element{fb92Chip("el-w"), fb92Chip("el-x"), fb92Chip("el-y"), fb92Chip("el-z")})
	resolved := dec
	rk := "a"
	resolved.ChosenKey = &rk
	if err := srv2.elIndex.applyEdit(msgID, []Element{resolved}); err == nil {
		t.Fatal("precondition: divergent elIndex should reject the target edit (proving the skip matters)")
	}

	setActiveDevice(t, srv2)
	srv2.bindSink(newFakeSink())
	if err := srv2.handleDeviceSend(context.Background(), "dev-testdevice01", elActionFrame("cid-0000000000000811", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	if len(outboxEdits(t, srv2, msgID)) != 1 {
		t.Fatal("divergent elIndex must NOT reject the projection-verified box edit (F4 skip)")
	}
}

// TestFB92OverCapElementNoEditStillForwards proves a near-cap element that would
// exceed the per-element serialized cap once the resolution field is added is
// dropped (no edit) while the action still forwards (F8).
func TestFB92OverCapElementNoEditStillForwards(t *testing.T) {
	srv, dev, sink := fb92Harness(t)
	// build a decision padded to just under maxElementBytes so adding chosenKey
	// tips it over. Padding rides an option Detail (no per-field length cap).
	dec := fb92Decision("el-d", "a", "b")
	base, _ := json.Marshal(dec)
	// target a pre-resolution size that is valid (≤ cap) but where +chosenKey busts it
	room := maxElementBytes - len(base)
	// chosenKey adds ~ len(`,"chosenKey":"a"`) = 16 bytes; pad to within ~8 of the cap
	pad := room - 8
	if pad < 0 {
		t.Fatalf("unexpected: decision already over cap by %d", -pad)
	}
	dec.Options[0].Detail = strings.Repeat("z", pad)
	if b, _ := json.Marshal(dec); len(b) > maxElementBytes {
		// trim to land at exactly cap
		dec.Options[0].Detail = strings.Repeat("z", pad-(len(b)-maxElementBytes))
	}
	// sanity: it validates now (under cap)...
	if _, err := validateElements([]json.RawMessage{mustJSON(dec)}); err != nil {
		t.Fatalf("padded decision should validate pre-resolution: %v", err)
	}
	// ...but adding chosenKey pushes it over.
	resolved := dec
	k := "a"
	resolved.ChosenKey = &k
	if _, err := validateElements([]json.RawMessage{mustJSON(resolved)}); err == nil {
		t.Fatal("precondition: resolved element should exceed the element cap")
	}

	msgID := emitElMsg(srv, dec)
	if err := srv.handleDeviceSend(context.Background(), dev, elActionFrame("cid-0000000000000821", msgID, "el-d", "pick", "a")); err != nil {
		t.Fatalf("handleDeviceSend: %v", err)
	}
	if edits := outboxEdits(t, srv, msgID); len(edits) != 0 {
		t.Fatalf("over-cap resolution must emit NO edit, got %d", len(edits))
	}
	if caps := drainCaptures(sink); len(caps) != 1 {
		t.Fatalf("action must still forward despite the dropped edit, got %d captures", len(caps))
	}
}

func mustJSON(v any) json.RawMessage {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return b
}
