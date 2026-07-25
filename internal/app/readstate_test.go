package app

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// readStateTestServer builds a server with n active devices, each bound to its
// own additively-minted room (one device per room, the shipped shape), with a
// provisioned + subscribed mailbox per device. It returns the server, the device
// ids, and per-device subscribers whose transients channel captures fanned frames.
func readStateTestServer(t *testing.T, n int) (*Server, []string, []*mailboxSubscriber) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:       dir,
		StateRoot:      dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	t.Cleanup(func() { srv.outbox.close() })

	ids := make([]string, n)
	subs := make([]*mailboxSubscriber, n)
	for i := 0; i < n; i++ {
		link, err := srv.store.MintLink("ws://127.0.0.1:8787", "pi")
		if err != nil {
			t.Fatal(err)
		}
		dev := "dev-" + string(rune('a'+i)) + "00000000000"
		if res, linked, err := srv.store.VerifyAndLink(link.Room, dev, link.Secret); err != nil || !linked || res != VerifyActive {
			t.Fatalf("link device %d: res=%v linked=%v err=%v", i, res, linked, err)
		}
		if err := srv.provisionMailbox(dev); err != nil {
			t.Fatalf("provision %d: %v", i, err)
		}
		_, _, _, sub, err := srv.mailbox.stateAndSubscribe(dev)
		if err != nil {
			t.Fatalf("subscribe %d: %v", i, err)
		}
		// A freshly provisioned + subscribed device is caught up (contiguous at the
		// seed head, no holes), so it participates in W — the steady-state the real
		// attach path reaches after reconcile/backfill. Without this a directly-driven
		// test device (which never runs serveV2Session) would be excluded from W and
		// every read rejected under the "participate only once caught up" model.
		srv.mailbox.markParticipating(dev, sub, srv.durableHead())
		ids[i] = dev
		subs[i] = sub
	}
	return srv, ids, subs
}

// drainRead reads all pending read frames on a subscriber's transient channel and
// returns the last j seen (or "" if none), asserting every transient IS a read.
func drainRead(t *testing.T, sub *mailboxSubscriber) string {
	t.Helper()
	last := ""
	for {
		select {
		case raw := <-sub.transients:
			var f struct {
				T string `json:"t"`
				J string `json:"j"`
			}
			if err := json.Unmarshal(raw, &f); err != nil {
				t.Fatalf("decode transient: %v", err)
			}
			if f.T != "read" {
				t.Fatalf("unexpected transient type %q, want read", f.T)
			}
			last = f.J
		default:
			return last
		}
	}
}

func writeSink() func([]byte) error { return func([]byte) error { return nil } }

// TestReadMaxMergeMonotonic proves the shared cursor only ever advances: a lower
// or equal j is a no-op (no persist, no fan); a higher j advances and persists.
func TestReadMaxMergeMonotonic(t *testing.T) {
	m, err := newLocalMailbox(filepath.Join(t.TempDir(), "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	m.disk.Devices["dev-x"] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}

	if adv, err := m.setRead("5"); err != nil || !adv {
		t.Fatalf("first read: adv=%v err=%v", adv, err)
	}
	if adv, _ := m.setRead("5"); adv {
		t.Fatal("equal read wrongly reported as advanced")
	}
	if adv, _ := m.setRead("3"); adv {
		t.Fatal("lower read wrongly reported as advanced")
	}
	if adv, _ := m.setRead("9"); !adv {
		t.Fatal("higher read not reported as advanced")
	}
	if got := m.readCursor(); got != "9" {
		t.Fatalf("cursor = %q, want 9", got)
	}
	// Multi-digit vs single-digit ordering is length-aware (decimalCmp), not
	// lexicographic: "10" > "9".
	if adv, _ := m.setRead("10"); !adv {
		t.Fatal("10 should advance past 9")
	}
	// Persistence: reload from disk and confirm the cursor survived.
	m2, err := newLocalMailbox(m.path)
	if err != nil {
		t.Fatal(err)
	}
	if got := m2.readCursor(); got != "10" {
		t.Fatalf("reloaded cursor = %q, want 10", got)
	}
}

// TestReadInvalidCursorDropped proves a corrupt on-disk read cursor is dropped to
// "" on load so it can never poison decimalCmp.
func TestReadInvalidCursorDropped(t *testing.T) {
	path := filepath.Join(t.TempDir(), "mailboxes.json")
	if err := os.WriteFile(path, []byte(`{"devices":{},"read":"0123"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	m, err := newLocalMailbox(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := m.readCursor(); got != "" {
		t.Fatalf("corrupt cursor not dropped: %q", got)
	}
}

// TestReadFrameHandlerValidation proves the connector handler accepts a valid
// read, rejects j > outbox cursor as bad_frame, rejects a non-decimal j, and that
// the read frame is inert when read-sync is gated off.
func TestReadFrameHandlerValidation(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	ctx := context.Background()

	// Push the outbox cursor forward so a bounded read has headroom.
	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}
	cursor := srv.outbox.cursor() // 3

	// Valid: j == cursor is allowed.
	bad, fatal := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink())
	if bad || fatal {
		t.Fatalf("valid read rejected: bad=%v fatal=%v", bad, fatal)
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("cursor after valid read = %q, want 3", got)
	}

	// j > cursor -> bad_frame, cursor unchanged.
	over := journalStringPlus(cursor, 1)
	bad, _ = srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"`+over+`"}`), writeSink())
	if !bad {
		t.Fatal("read past outbox cursor not counted as bad_frame")
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("cursor fast-forwarded past outbox: %q", got)
	}

	// Non-decimal j -> bad_frame.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"x"}`), writeSink()); !bad {
		t.Fatal("non-decimal read.j not counted as bad_frame")
	}

	// Gated off: inert (not bad_frame, ignored).
	srv.readSync = false
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"x"}`), writeSink()); bad {
		t.Fatal("read counted as bad_frame while read-sync disabled")
	}
}

// TestReadFansToSiblings proves a read from device A fans a read transient to ALL
// active devices (A included) carrying the SAME global journal seq j — never a
// per-device mailbox m cursor. This is the global-seq invariant.
func TestReadFansToSiblings(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 3)
	ctx := context.Background()

	for i := 0; i < 4; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}
	// Devices A/B/C now each have mailbox items with m cursors 1..4; the journal
	// seq j is the shared 1..4. Drain the enqueued items so only read transients
	// remain on the channels.
	for _, sub := range subs {
		drainItems(t, sub)
	}

	// Device A reads through journal seq 4.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"4"}`), writeSink()); bad {
		t.Fatal("valid read rejected")
	}

	// Every device — including A — receives read.j == "4" (global j-space).
	for i, sub := range subs {
		if got := drainRead(t, sub); got != "4" {
			t.Fatalf("device %d read fan = %q, want 4 (global journal seq)", i, got)
		}
	}
}

// TestReadSnapshotAfterDrainOrdering proves snapshotReadTo delivers the current
// shared cursor to a single (reconnecting) device, and only after that device's
// mailbox items are drained — the ordering the connector enforces by calling it
// post-drain. Here we assert the snapshot carries the persisted cursor and is a
// transient (never a mailbox item that would need acking).
func TestReadSnapshotAfterDrainOrdering(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	ctx := context.Background()

	for i := 0; i < 2; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}
	for _, sub := range subs {
		drainItems(t, sub)
	}
	// Device A sets read to 2; B receives the live fan.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"2"}`), writeSink()); bad {
		t.Fatal("read rejected")
	}
	drainRead(t, subs[0])
	drainRead(t, subs[1])

	// A brand-new subscriber for device B (simulating reconnect) must get the
	// snapshot via snapshotReadTo carrying the persisted cursor.
	_, _, _, fresh, err := srv.mailbox.stateAndSubscribe(ids[1])
	if err != nil {
		t.Fatal(err)
	}
	srv.snapshotReadTo(ids[1])
	if got := drainRead(t, fresh); got != "2" {
		t.Fatalf("reconnect snapshot = %q, want 2", got)
	}

	// Gated off -> no snapshot.
	srv.readSync = false
	srv.snapshotReadTo(ids[1])
	if got := drainRead(t, fresh); got != "" {
		t.Fatalf("snapshot emitted while read-sync off: %q", got)
	}
}

func journalStringPlus(seq uint64, d uint64) string { return journalString(seq + d) }

// wireTypeJ pulls the frame type and j field off a recorded wire frame.
func wireTypeJ(frame string) (typ, j string) {
	var f struct {
		T string `json:"t"`
		J string `json:"j"`
	}
	_ = json.Unmarshal([]byte(frame), &f)
	return f.T, f.J
}

func wireHas(wire []string, typ, j string) bool {
	for _, f := range wire {
		if tt, jj := wireTypeJ(f); tt == typ && jj == j {
			return true
		}
	}
	return false
}

// TestReadCursorIsJournalSpaceNotMailboxSpace proves the read cursor lives in
// GLOBAL journal-seq (j) space, never per-device mailbox (m) space, even when the
// two DIVERGE. Device A is enqueued journal seqs 1,2,3 (m = 1,2,3) while device B
// is enqueued only journal seq 3 (m = 1) — so j=3 is A's m=3 but B's m=1. A read
// through j=3 must fan the JOURNAL seq "3" to both, never each device's local m.
func TestReadCursorIsJournalSpaceNotMailboxSpace(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	ctx := context.Background()

	for _, j := range []string{"1", "2", "3"} {
		if _, err := srv.mailbox.enqueue(ids[0], j, msgFrame(0, "a", "hi", nil, "", nil, nil)); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := srv.mailbox.enqueue(ids[1], "3", msgFrame(0, "a", "hi", nil, "", nil, nil)); err != nil {
		t.Fatal(err)
	}

	// Confirm the cursors genuinely diverge: A's item for j=3 has m=3, B's has m=1.
	aItems := srv.mailbox.disk.Devices[ids[0]].Items
	bItems := srv.mailbox.disk.Devices[ids[1]].Items
	if got := aItems[len(aItems)-1]; got.J != "3" || got.M != "3" {
		t.Fatalf("device A last item j/m = %q/%q, want 3/3", got.J, got.M)
	}
	if got := bItems[len(bItems)-1]; got.J != "3" || got.M != "1" {
		t.Fatalf("device B last item j/m = %q/%q, want 3/1 (divergent m-space)", got.J, got.M)
	}

	// Each direct enqueue above folded into the devices' contiguous heads: A holds
	// 1,2,3 (CHead 3) and B holds 3 (CHead 3), so W=3 and the handler accepts read.j
	// up to 3 — no scalar to poke.
	for _, sub := range subs {
		drainItems(t, sub)
	}

	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("valid read rejected")
	}
	// BOTH devices receive read.j="3" (the shared journal seq), never their local m
	// (3 for A, 1 for B). A regression to m-space would send B "1".
	if got := drainRead(t, subs[0]); got != "3" {
		t.Fatalf("device A read = %q, want 3", got)
	}
	if got := drainRead(t, subs[1]); got != "3" {
		t.Fatalf("device B read = %q, want 3 (journal seq, NOT its m=1)", got)
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("persisted cursor = %q, want 3", got)
	}
}

// TestReadNeverPrecedesItemOnWire drives the REAL serveSessionStream write path
// and proves the B2 wire guarantee: a read.j frame never reaches the wire before
// the mailbox_item.j it refers to. Two items and a read.j=2 are all queued, then
// the stream is run; because the read fan only publishes after its item is on the
// ordered channel and the writer flushes ready items before any transient, the
// item must always precede the read. Looped to shake out select randomness.
func TestReadNeverPrecedesItemOnWire(t *testing.T) {
	for iter := 0; iter < 50; iter++ {
		srv, ids, subs := readStateTestServer(t, 1)
		dev, sub := ids[0], subs[0]

		for i := 0; i < 2; i++ {
			srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
		}
		srv.setRead(2) // fans read.j=2 onto the transient channel while items 1,2 queue on sub.items

		dv, _ := srv.store.Device(dev)
		room := RoomRecord{ID: dv.Room}

		var mu sync.Mutex
		var wire []string
		write := func(b []byte) error {
			mu.Lock()
			wire = append(wire, string(append([]byte(nil), b...)))
			mu.Unlock()
			return nil
		}
		ctx, cancel := context.WithCancel(context.Background())
		inputs := make(chan sessionInput)
		done := make(chan struct{})
		go func() { srv.serveSessionStream(ctx, room, dev, sub, nil, inputs, write); close(done) }()

		deadline := time.After(2 * time.Second)
		for {
			mu.Lock()
			ready := wireHas(wire, "mailbox_item", "2") && wireHas(wire, "read", "2")
			mu.Unlock()
			if ready {
				break
			}
			select {
			case <-deadline:
				cancel()
				<-done
				t.Fatalf("iter %d: timed out waiting for item+read on the wire", iter)
			case <-time.After(2 * time.Millisecond):
			}
		}
		cancel()
		<-done

		mu.Lock()
		firstRead, lastItem2 := -1, -1
		for i, f := range wire {
			typ, j := wireTypeJ(f)
			if typ == "read" && j == "2" && firstRead < 0 {
				firstRead = i
			}
			if typ == "mailbox_item" && j == "2" {
				lastItem2 = i
			}
		}
		snap := append([]string(nil), wire...)
		mu.Unlock()
		if lastItem2 < 0 || firstRead < 0 || lastItem2 >= firstRead {
			t.Fatalf("iter %d: read.j=2 not strictly after mailbox_item.j=2 (item=%d read=%d): %v", iter, lastItem2, firstRead, snap)
		}
		srv.outbox.close()
	}
}

// TestReadRejectsTransientReservedSeq proves B3: a read.j targeting a
// transient-reserved outbox seq (past the durable journal) is rejected, never
// persisted — so a restart can't reuse that seq for a durable message that is
// already marked read.
func TestReadRejectsTransientReservedSeq(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	ctx := context.Background()

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}
	// A transient reserves the next RAW outbox seq (4) without journaling anything
	// durable — the durable high-water stays 3.
	srv.emitTransient(func(seq uint64) []byte { return readFrame("3") })
	if got := srv.outbox.cursor(); got != 4 {
		t.Fatalf("outbox cursor = %d, want 4 (durable 3 + 1 transient)", got)
	}

	// read.j=4 (the transient seq) must be a bad_frame and must not persist.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"4"}`), writeSink()); !bad {
		t.Fatal("read.j at a transient-reserved seq was accepted")
	}
	if got := srv.mailbox.readCursor(); got == "4" {
		t.Fatalf("read cursor advanced onto a transient seq: %q", got)
	}
	// read.j=3 (the durable high-water) is accepted.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read.j at the durable high-water was rejected")
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("cursor = %q, want 3", got)
	}
}

// TestReadCursorClampedToDurableOnRestart proves the B3 boot clamp: if the
// persisted read cursor sits ABOVE the restored durable journal cursor (the
// degraded-outbox-persistence case, where the referenced seq never reached
// outbox.jsonl and will be reused), it is clamped down to the durable high-water
// so a future durable message can't inherit an already-read seq.
func TestReadCursorClampedToDurableOnRestart(t *testing.T) {
	dir := t.TempDir()
	var b strings.Builder
	for seq := 1; seq <= 10; seq++ {
		fmt.Fprintf(&b, `{"seq":%d,"frame":{"t":"msg","seq":%d}}`+"\n", seq, seq)
	}
	if err := os.WriteFile(filepath.Join(dir, "outbox.jsonl"), []byte(b.String()), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "mailboxes.json"), []byte(`{"devices":{},"read":"11"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{
		StateDir:       dir,
		StateRoot:      dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	t.Cleanup(func() { srv.outbox.close() })

	if got := srv.outbox.cursor(); got != 10 {
		t.Fatalf("restored durable cursor = %d, want 10", got)
	}
	if got := srv.mailbox.readCursor(); got != "10" {
		t.Fatalf("read cursor after restart = %q, want 10 (clamped to durable high-water)", got)
	}
}

// TestReadHighWaterNoAdvanceOnPartialFan proves B1: when a sibling's enqueue
// fails (here its mailbox is Full so enqueue returns ErrMailboxFull), the
// fully-fanned durable high-water must NOT advance past the last universally
// delivered seq — so a read.j for the not-fully-fanned item is rejected until the
// lagging device recovers.
func TestReadHighWaterNoAdvanceOnPartialFan(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 2)
	ctx := context.Background()

	// Seq 1 fans to BOTH devices: high-water = 1.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("high-water after full fan = %d, want 1", got)
	}
	// Force device B's mailbox Full so its next enqueue fails (item j never lands).
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[ids[1]].Full = true
	srv.mailbox.mu.Unlock()

	// Seq 2 fans to A but FAILS for B (ErrMailboxFull): high-water must stay 1.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi2", nil, "", nil, nil) })
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("high-water advanced past a partial fan: %d, want 1", got)
	}

	// read.j=2 (only A holds it) must be rejected as bad_frame.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"2"}`), writeSink()); !bad {
		t.Fatal("read.j past the fully-fanned high-water was accepted")
	}
	if got := srv.mailbox.readCursor(); got == "2" {
		t.Fatalf("read cursor advanced onto a not-fully-fanned seq: %q", got)
	}
	// read.j=1 (universally delivered) is accepted.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"1"}`), writeSink()); bad {
		t.Fatal("read.j at the fully-fanned high-water was rejected")
	}
	if got := srv.mailbox.readCursor(); got != "1" {
		t.Fatalf("cursor = %q, want 1", got)
	}
}

// TestCrashGapHoleNotLeapedThenReconciled proves blocker #1 end-to-end, PAST the
// next emission (the prior boot test stopped at restart). Durable persistence
// happens BEFORE the mailbox fan, so a crash can leave a durable seq2 in
// outbox.jsonl that never fanned. On boot the reconcile reconstructs a Hole at 2;
// a LATER emit of seq3 must NOT fold the contiguous head past the seq2 gap. While
// the device is disconnected it is excluded from W (inert). Once it reconnects but
// before it is reconciled/caught up, its CHead=1 holds W at 1 (read.j=2 rejected);
// only after the attach reconcile actually backfills seq2 does W reach 3.
func TestCrashGapHoleNotLeapedThenReconciled(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	ctx := context.Background()
	dev := ids[0]

	// Seq 1 is fully fanned and persisted to both outbox.jsonl and the mailbox.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) })
	drainItems(t, subs[0])

	// Simulate the crash window: seq 2 reached outbox.jsonl but never fanned to the
	// mailbox. Close the live handle, then append the orphaned durable record.
	srv.outbox.close()
	f, err := os.OpenFile(filepath.Join(srv.cfg.StateDir, "outbox.jsonl"), os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(`{"seq":2,"frame":{"t":"msg","seq":2}}` + "\n"); err != nil {
		t.Fatal(err)
	}
	f.Close()

	// Restart: fresh Server on the same state dir. Boot reconcile reconstructs the
	// seq2 hole; the device is disconnected, so read-state is inert (W=0).
	srv2 := NewServer(srv.cfg, transcript.New(srv.cfg.TranscriptFile))
	if srv2.initErr != nil {
		t.Fatal(srv2.initErr)
	}
	t.Cleanup(func() { srv2.outbox.close() })

	if got := srv2.outbox.cursor(); got != 2 {
		t.Fatalf("restored outbox cursor = %d, want 2 (the orphaned seq)", got)
	}
	if got := srv2.mailbox.contiguousHead(dev); got != 1 {
		t.Fatalf("boot contiguous head = %d, want 1 (seq2 hole reconstructed)", got)
	}
	if got := srv2.durableWatermark(); got != 0 {
		t.Fatalf("boot W = %d, want 0 (disconnected device excluded — inert)", got)
	}

	// THE KEY ASSERTION: emit seq3 while the device is still disconnected. A missing
	// seq2 hole would let the fold leap to 3; the reconstructed hole freezes it at 1.
	srv2.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) })
	if got := srv2.mailbox.contiguousHead(dev); got != 1 {
		t.Fatalf("contiguous head after offline seq3 = %d, want 1 (must NOT leap the seq2 gap)", got)
	}

	// The device reconnects (subscribes + participates) but is not yet reconciled:
	// its CHead=1 holds W at 1, so read.j=2 is rejected.
	_, _, _, sub, err := srv2.mailbox.stateAndSubscribe(dev)
	if err != nil {
		t.Fatal(err)
	}
	srv2.mailbox.markParticipating(dev, sub, srv2.durableHead())
	if got := srv2.durableWatermark(); got != 1 {
		t.Fatalf("W with reconnected-but-lagging device = %d, want 1", got)
	}
	if bad, _ := srv2.handleSessionInput(ctx, dev, nil, []byte(`{"t":"read","j":"2"}`), writeSink()); !bad {
		t.Fatal("read.j=2 accepted while seq2 is still an unfilled hole")
	}
	if bad, _ := srv2.handleSessionInput(ctx, dev, nil, []byte(`{"t":"read","j":"1"}`), writeSink()); bad {
		t.Fatal("read.j=1 at the held watermark was rejected")
	}

	// The attach reconcile actually backfills seq2 (and re-offers seq3): the head
	// folds up to 3 and W reaches 3.
	srv2.reconcileDevice(dev)
	if got := srv2.mailbox.contiguousHead(dev); got != 3 {
		t.Fatalf("contiguous head after reconcile = %d, want 3", got)
	}
	if got := srv2.durableWatermark(); got != 3 {
		t.Fatalf("W after reconcile = %d, want 3", got)
	}
	if bad, _ := srv2.handleSessionInput(ctx, dev, nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read.j=3 rejected after reconcile caught the device up")
	}
}

// TestReadClampsInteriorTransientGap proves B3-interior: with durable seq 1, a
// transient reservation at seq 2, and durable seq 3 (high-water 3), a read.j=2
// (≤ the high-water yet NOT a durable frame) is clamped DOWN to the nearest
// durable seq (1) and never persisted as a transient seq.
func TestReadClampsInteriorTransientGap(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	ctx := context.Background()

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // durable 1
	srv.emitTransient(func(seq uint64) []byte { return readFrame("1") })                      // transient 2 (no durable frame)
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) }) // durable 3
	drainItems(t, subs[0])
	drainRead(t, subs[0])

	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("high-water = %d, want 3", got)
	}
	// read.j=2 targets the interior transient seq: accepted (not bad_frame) but
	// clamped down to the durable seq 1.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"2"}`), writeSink()); bad {
		t.Fatal("interior-gap read.j=2 was rejected instead of clamped")
	}
	if got := srv.mailbox.readCursor(); got != "1" {
		t.Fatalf("read cursor = %q, want 1 (clamped off the transient seq 2)", got)
	}
	// The fan carried the clamped durable seq, never the transient 2.
	if got := drainRead(t, subs[0]); got != "1" {
		t.Fatalf("fanned read.j = %q, want 1 (clamped)", got)
	}
	// read.j=3 (durable) is accepted unchanged.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("durable read.j=3 was rejected")
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("cursor = %q, want 3", got)
	}
}

// TestReadHighWaterMonotoneOnRetry proves SF-A: reconciliation re-drives
// enqueueDurableLocked with an OLD persisted delivery seq; the plain Store used
// before regressed the high-water (3 → 1). The monotone-max advance must leave it
// at 3.
func TestReadHighWaterMonotoneOnRetry(t *testing.T) {
	srv, _, _ := readStateTestServer(t, 1)

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("high-water = %d, want 3", got)
	}
	// Simulate a delivery-reconcile retry with the OLD seq 1 (idempotent re-enqueue).
	srv.deliveryMu.Lock()
	srv.enqueueDurableLocked(1, msgFrame(1, "a", "hi", nil, "", nil, nil))
	srv.deliveryMu.Unlock()
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("high-water regressed to %d on an old-seq retry, want 3", got)
	}
}

// TestReadFromDivergentDeviceIsJournalSpace proves SF-B: a read SENT by the
// divergent device B (whose mailbox m-space differs from journal j-space) must be
// interpreted in GLOBAL journal space, never as B's local mailbox cursor. Device B
// holds journal seq 3 at its local m=1; a read.j=3 from B must persist/fan the
// journal seq 3, not B's m=1.
func TestReadFromDivergentDeviceIsJournalSpace(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	ctx := context.Background()

	for _, j := range []string{"1", "2", "3"} {
		if _, err := srv.mailbox.enqueue(ids[0], j, msgFrame(0, "a", "hi", nil, "", nil, nil)); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := srv.mailbox.enqueue(ids[1], "3", msgFrame(0, "a", "hi", nil, "", nil, nil)); err != nil {
		t.Fatal(err)
	}
	// Confirm divergence: B's item for j=3 sits at local m=1.
	bItems := srv.mailbox.disk.Devices[ids[1]].Items
	if got := bItems[len(bItems)-1]; got.J != "3" || got.M != "1" {
		t.Fatalf("device B last item j/m = %q/%q, want 3/1 (divergent)", got.J, got.M)
	}

	// A holds 1,2,3 and B holds 3 (both CHead 3 after the direct enqueues), so W=3.
	for _, sub := range subs {
		drainItems(t, sub)
	}

	// The read is SENT BY B (the divergent device). If any code path read this as
	// B's mailbox cursor it would treat 3 as m=3 (B only has m=1) — the interpretation
	// gap the earlier test (read sent by A, where m==j) could not catch.
	if bad, _ := srv.handleSessionInput(ctx, ids[1], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read from divergent device B rejected")
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("persisted cursor = %q, want 3 (journal seq from B, not its m)", got)
	}
	// Both devices receive the journal seq 3, never B's local m.
	if got := drainRead(t, subs[0]); got != "3" {
		t.Fatalf("device A read fan = %q, want 3", got)
	}
	if got := drainRead(t, subs[1]); got != "3" {
		t.Fatalf("device B read fan = %q, want 3 (journal seq, NOT its m=1)", got)
	}
}

// TestReadInertDuringGapRecovery proves SF1: a `read` sent during mailbox-gap
// recovery is inert — ignored, never counted toward the 4002 bad-frame limit —
// so a client that always emits reads is never penalized while recovering.
func TestReadInertDuringGapRecovery(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(ids[0])
	if err != nil {
		t.Fatal(err)
	}

	inputs := make(chan sessionInput, 8)
	for i := 0; i < maxBadFrames+2; i++ {
		inputs <- sessionInput{raw: []byte(`{"t":"read","j":"1"}`)}
	}
	inputs <- sessionInput{raw: []byte(`{"t":"mailbox_ack","m":"0"}`)}

	res, proceed := waitGapAck(context.Background(), inputs, writeSink(), srv, sub, ids[0], "0")
	if !proceed {
		t.Fatalf("gap ack did not proceed — reads were counted as bad_frames: %+v", res)
	}
	if res.closeCode != 0 {
		t.Fatalf("gap loop closed with %d, want 0 (reads must be inert during recovery)", res.closeCode)
	}
}

// TestReadSaveFailureRollsBackAndRetries proves SF2: when persisting an advanced
// read cursor fails, the in-memory advance is rolled back so an identical retry
// re-attempts the persist+fan (rather than becoming a silent no-op that never
// converges and regresses on restart).
func TestReadSaveFailureRollsBackAndRetries(t *testing.T) {
	dir := t.TempDir()
	good := filepath.Join(dir, "mailboxes.json")
	m, err := newLocalMailbox(good)
	if err != nil {
		t.Fatal(err)
	}
	m.disk.Devices["dev-x"] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}

	// Force saveLocked to fail: point the mailbox at a path whose parent is a
	// regular file so MkdirAll/CreateTemp error out.
	blocker := filepath.Join(dir, "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	m.path = filepath.Join(blocker, "nested", "mailboxes.json")

	if adv, err := m.setRead("5"); adv || err == nil {
		t.Fatalf("setRead should fail to persist: adv=%v err=%v", adv, err)
	}
	// Rolled back: the cursor did NOT advance, so the retry is not a no-op.
	if got := m.readCursor(); got != "" {
		t.Fatalf("cursor left advanced after save failure: %q (must roll back)", got)
	}

	// Recover the path; the identical retry now persists and advances.
	m.path = good
	if adv, err := m.setRead("5"); !adv || err != nil {
		t.Fatalf("retry after recovery: adv=%v err=%v (want advanced, no error)", adv, err)
	}
	if got := m.readCursor(); got != "5" {
		t.Fatalf("cursor = %q, want 5", got)
	}
	// And it is durable.
	m2, err := newLocalMailbox(good)
	if err != nil {
		t.Fatal(err)
	}
	if got := m2.readCursor(); got != "5" {
		t.Fatalf("reloaded cursor = %q, want 5", got)
	}
}

// TestReadHoleHoldsWatermarkThenAdvances proves the partial-fan hole FOLLOW-ON
// that the monotone-max scalar could not: seq1 fans (W=1), seq2 is DROPPED by
// sibling B (hole), then seq3 FULLY fans to both. A monotone-max would jump the
// mark to 3; the hole-free min-of-contiguous-heads holds W at 1 (B's contiguous
// head is frozen below the seq2 hole even though it received seq3), so read.j=2 is
// rejected. Once seq2 is actually delivered (backfilled) to B, W advances to 3.
func TestReadHoleHoldsWatermarkThenAdvances(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 2)
	ctx := context.Background()

	// seq1 fans to BOTH: W=1.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) })
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W after full fan = %d, want 1", got)
	}
	// Force B full so seq2 is dropped for B (a hole), delivered to A.
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[ids[1]].Full = true
	srv.mailbox.mu.Unlock()
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "two", nil, "", nil, nil) })

	// B "acks the gap" (clears Full) but seq2 is NOT replayed; seq3 now fully fans.
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[ids[1]].Full = false
	srv.mailbox.mu.Unlock()
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) })

	// W is HELD at 1 by B's interior seq2 hole even though seq3 fanned to both.
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W jumped over the seq2 hole: %d, want 1 (monotone-max regression)", got)
	}
	if got := srv.mailbox.contiguousHead(ids[1]); got != 1 {
		t.Fatalf("B contiguous head = %d, want 1 (frozen below the seq2 hole)", got)
	}
	// read.j=2 is rejected while the hole is open.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"2"}`), writeSink()); !bad {
		t.Fatal("read.j=2 accepted while seq2 is missing from B")
	}
	if got := srv.mailbox.readCursor(); got == "2" || got == "3" {
		t.Fatalf("read cursor advanced past the held watermark: %q", got)
	}

	// Backfill actually delivers the missing seq2 (and re-offers seq3) to B.
	srv.backfillDevice(ids[1])
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("W after backfill = %d, want 3 (hole filled, head folds up)", got)
	}
	// Now read.j=3 is accepted.
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read.j=3 rejected after the hole was backfilled")
	}
	if got := srv.mailbox.readCursor(); got != "3" {
		t.Fatalf("read cursor = %q, want 3", got)
	}
}

// TestStaleMailboxReactivationHoldsWatermark proves a reactivating device that
// missed several interior durable frames holds W down until it is backfilled —
// exactly what happens on attach (backfillDevice runs after provision). Device B
// misses seq2 AND seq3 (mailbox full across both fans); W stays at 1 until the
// on-attach reconcile fills BOTH, then folds B's contiguous head up to 3.
func TestStaleMailboxReactivationHoldsWatermark(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 2)
	ctx := context.Background()

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // both, W=1
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[ids[1]].Full = true
	srv.mailbox.mu.Unlock()
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "two", nil, "", nil, nil) })   // B misses seq2
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) }) // B misses seq3

	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W with a stale lagging device = %d, want 1", got)
	}
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); !bad {
		t.Fatal("read.j=3 accepted while B is two interior frames behind")
	}

	// Reactivation reconcile (the attach path): clear full, backfill.
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[ids[1]].Full = false
	srv.mailbox.mu.Unlock()
	srv.backfillDevice(ids[1])

	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("W after reactivation backfill = %d, want 3", got)
	}
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read.j=3 rejected after reactivation caught B up")
	}
}

// TestReactivationOverFullGapHoldsWatermarkNoLeap drives the REAL attach ordering
// (reconcile → gap-ack → reconcile → participate) WITHOUT manually clearing Full
// first (blocker #2). Device B goes away, its mailbox fills, and seq2/seq3 fan to A
// but fail for B. On reattach the reconcile records the gap as holes (backfill
// can't deliver while Full), so B's contiguous head is HELD at 1 (no leap) and W is
// held at 1 once B participates. Only after the gap ack clears Full and the
// reconcile re-runs does B actually receive seq2/seq3 and W advance to 3.
func TestReactivationOverFullGapHoldsWatermarkNoLeap(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	ctx := context.Background()
	A, B := ids[0], ids[1]

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // both, CHead=1
	drainItems(t, subs[0])
	drainItems(t, subs[1])

	// B disconnects and its mailbox fills; seq2/seq3 fan to A but fail for B.
	srv.mailbox.unsubscribe(B, subs[1])
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[B].Full = true
	srv.mailbox.mu.Unlock()
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "two", nil, "", nil, nil) })
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) })

	// A DISCONNECTED B does not pin W: A alone participates ⇒ W=3.
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("W with B disconnected = %d, want 3 (B excluded, not pinning)", got)
	}

	// B reattaches. Reconcile records the seq2/seq3 gap as holes; backfill can't
	// deliver while Full, so the head stays frozen at 1 (no leap).
	srv.reconcileDevice(B)
	if got := srv.mailbox.contiguousHead(B); got != 1 {
		t.Fatalf("B head after Full reconcile = %d, want 1 (gap held, no leap)", got)
	}
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(B)
	if err != nil {
		t.Fatal(err)
	}
	srv.mailbox.markParticipating(B, sub, srv.durableHead())
	// B now participates with its honest CHead=1: W is HELD at 1 by the gap.
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W after B rejoins mid-gap = %d, want 1 (held by seq2/seq3 holes)", got)
	}
	if bad, _ := srv.handleSessionInput(ctx, A, nil, []byte(`{"t":"read","j":"2"}`), writeSink()); !bad {
		t.Fatal("read.j=2 accepted while B is missing seq2/seq3")
	}

	// The gap ack clears Full (the real waitGapAck acks at floor); the re-run reconcile
	// now backfills seq2/seq3, folding B up to 3 — advancing only because B genuinely
	// received the frames, never leaping while the holes were open.
	if err := srv.mailbox.ack(B, "0"); err != nil {
		t.Fatal(err)
	}
	srv.reconcileDevice(B)
	if got := srv.mailbox.contiguousHead(B); got != 3 {
		t.Fatalf("B head after ack+reconcile = %d, want 3", got)
	}
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("W after B caught up = %d, want 3", got)
	}
	if bad, _ := srv.handleSessionInput(ctx, A, nil, []byte(`{"t":"read","j":"3"}`), writeSink()); bad {
		t.Fatal("read.j=3 rejected after B caught up")
	}
}

// TestReadAcceptReBoundsToParticipantSet proves the read-validation/activation race
// fix (blocker #2): the acceptance bound is re-applied to the CURRENT participating
// set atomically under the persist lock, not against a stale pre-check snapshot. A
// read.j=5 passes its initial bound while only A (CHead=5) participates; a device B
// then becomes participating with a low CHead=1 (dropping W to 1); the atomic
// re-bound in acceptReadAtMost must now reject the persist of 5.
func TestReadAcceptReBoundsToParticipantSet(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	A, B := ids[0], ids[1]

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // both CHead=1
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[B].Full = true
	srv.mailbox.mu.Unlock()
	for i := 0; i < 4; i++ { // seq2..seq5: A→5, B misses (holes), CHead stays 1
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	_ = A

	// B excluded (as if still reconciling): W = A alone = 5 — the stale snapshot a
	// read.j=5 passes its initial bound against.
	srv.mailbox.unsubscribe(B, subs[1])
	if got := srv.durableWatermark(); got != 5 {
		t.Fatalf("pre-check W = %d, want 5 (B excluded)", got)
	}

	// B becomes participating mid-validation with CHead=1 ⇒ W drops to 1.
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(B)
	if err != nil {
		t.Fatal(err)
	}
	srv.mailbox.markParticipating(B, sub, srv.durableHead())
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W after B participates = %d, want 1", got)
	}

	// acceptReadAtMost(5) recomputes W=1 under the persist lock ⇒ reject.
	srv.setRead(5)
	if got := srv.mailbox.readCursor(); got == "5" {
		t.Fatal("read.j=5 persisted though a newly-participating device holds CHead=1 (race not closed)")
	}
	// A read consistent with the current W is accepted.
	srv.setRead(1)
	if got := srv.mailbox.readCursor(); got != "1" {
		t.Fatalf("read cursor = %q, want 1 (consistent read accepted)", got)
	}
}

// TestHolesCapTriggersFullResyncAndUnpinsWatermark proves blocker #3: a device that
// falls past the per-device holes cap is collapsed into a bounded FULL RESYNC
// (tracking dropped) and EXCLUDED from W so it stops pinning the watermark at its
// stale gap; and a disconnected device never pins W either.
func TestHolesCapTriggersFullResyncAndUnpinsWatermark(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 2)
	A, B := ids[0], ids[1]

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // both CHead=1
	srv.mailbox.mu.Lock()
	srv.mailbox.disk.Devices[B].Full = true
	srv.mailbox.mu.Unlock()
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "two", nil, "", nil, nil) })   // B hole 2
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) }) // B hole 3

	// Before the cap: B (participating, CHead=1) pins W at its interior gap.
	if got := srv.durableWatermark(); got != 1 {
		t.Fatalf("W before resync = %d, want 1 (B pins at its gap)", got)
	}

	// B falls far enough behind to blow the holes cap → full-resync collapse.
	extra := make([]uint64, 0, maxDeviceHoles+8)
	for s := uint64(4); len(extra) < maxDeviceHoles+4; s++ {
		extra = append(extra, s)
	}
	srv.mailbox.reconcileHoles(B, extra)

	// Bounded state: tracking dropped, marked for resync.
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[B]
	resync, nh, na := mb.Resync, len(mb.Holes), len(mb.Ahead)
	srv.mailbox.mu.Unlock()
	if !resync || nh != 0 || na != 0 {
		t.Fatalf("B not collapsed to bounded resync: resync=%v holes=%d ahead=%d", resync, nh, na)
	}

	// A resync device is EXCLUDED from W even while connected: it no longer pins W at
	// its stale gap. W is now A's head (3).
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("W after B resync = %d, want 3 (dead-behind device released, not pinning)", got)
	}

	// And a DISCONNECTED device never pins W: drop A's subscription ⇒ no participant
	// remains ⇒ read-state is inert (W=0).
	srv.mailbox.unsubscribe(A, subs[0])
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W with no connected participant = %d, want 0 (inert)", got)
	}
}

// TestReadAgedOutTransientNeverPersisted proves blocker #2 structurally: a
// transient seq that has been EVICTED from the hot ring is never persisted as the
// read cursor — the durable-seq snap is ring-independent (it consults the
// append-only journal) and clamps down to the real durable seq below it.
func TestReadAgedOutTransientNeverPersisted(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	ctx := context.Background()

	// Shrink the hot ring so the interior transient seq is evicted after a few
	// more durable frames — the append-only journal remains authoritative.
	srv.outbox.mu.Lock()
	srv.outbox.max = 3
	srv.outbox.mu.Unlock()

	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "one", nil, "", nil, nil) }) // durable 1
	srv.emitTransient(func(seq uint64) []byte { return readFrame("1") })                      // transient 2
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "three", nil, "", nil, nil) }) // durable 3
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "four", nil, "", nil, nil) })  // durable 4
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "five", nil, "", nil, nil) })  // durable 5

	// The transient seq 2 is now below the ring window (ring holds 3,4,5).
	if _, ok := srv.outbox.highestDurableSeqAtMost(2); !ok {
		t.Fatal("durable oracle lost the pre-ring history (should read the journal)")
	}
	if got, _ := srv.outbox.highestDurableSeqAtMost(2); got != 1 {
		t.Fatalf("durable ≤2 = %d, want 1 (ring-independent snap)", got)
	}

	// A read on the evicted transient seq 2 must snap DOWN to durable 1, never persist "2".
	if bad, _ := srv.handleSessionInput(ctx, ids[0], nil, []byte(`{"t":"read","j":"2"}`), writeSink()); bad {
		t.Fatal("interior transient read.j=2 wrongly rejected instead of snapped")
	}
	if got := srv.mailbox.readCursor(); got == "2" {
		t.Fatal("an aged-out transient seq was persisted as the read cursor")
	}
	if got := srv.mailbox.readCursor(); got != "1" {
		t.Fatalf("read cursor = %q, want 1 (snapped off the transient seq)", got)
	}
}

// TestEnqueueSaveFailureRollsBackAndReenqueues proves blocker #3: when saveLocked
// fails, enqueue rolls back ALL in-memory mutations (Head, JHead, contiguous head,
// Items, and the implicit ID dedup entry) so a retry genuinely re-enqueues rather
// than hitting a false-success dedup — and the contiguous head only ever reflects
// durably-saved frames.
func TestEnqueueSaveFailureRollsBackAndReenqueues(t *testing.T) {
	dir := t.TempDir()
	good := filepath.Join(dir, "mailboxes.json")
	m, err := newLocalMailbox(good)
	if err != nil {
		t.Fatal(err)
	}
	m.disk.Devices["dev-x"] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0", JHead: "0", CHead: "0"}
	payload := msgFrame(0, "a", "hi", nil, "", nil, nil)

	// Force saveLocked to fail: a path whose parent is a regular file.
	blocker := filepath.Join(dir, "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	m.path = filepath.Join(blocker, "nested", "mailboxes.json")

	if _, err := m.enqueue("dev-x", "1", payload); err == nil {
		t.Fatal("enqueue should fail to persist")
	}
	// EVERYTHING rolled back: no item, no head/contiguous-head advance, no dedup entry.
	mb := m.disk.Devices["dev-x"]
	if len(mb.Items) != 0 {
		t.Fatalf("Items left mutated after save failure: %d", len(mb.Items))
	}
	if mb.Head != "0" || mb.JHead != "0" || mb.CHead != "0" {
		t.Fatalf("heads left mutated after save failure: Head=%q JHead=%q CHead=%q", mb.Head, mb.JHead, mb.CHead)
	}

	// Recover the path; the identical retry must genuinely re-enqueue (not a dedup
	// false-success) AND persist.
	m.path = good
	item, err := m.enqueue("dev-x", "1", payload)
	if err != nil {
		t.Fatalf("retry after recovery failed: %v", err)
	}
	if item.ID == "" || item.J != "1" {
		t.Fatalf("retry did not re-enqueue a real item: %+v", item)
	}
	if got := m.contiguousHead("dev-x"); got != 1 {
		t.Fatalf("contiguous head = %d, want 1 (only after a durable save)", got)
	}
	m2, err := newLocalMailbox(good)
	if err != nil {
		t.Fatal(err)
	}
	if len(m2.disk.Devices["dev-x"].Items) != 1 {
		t.Fatal("re-enqueued item did not persist to disk")
	}
	if got := m2.contiguousHead("dev-x"); got != 1 {
		t.Fatalf("reloaded contiguous head = %d, want 1", got)
	}
}

// TestParkedResyncDeviceIsStaticAcrossEmits proves BLOCKER 1: a device parked in a
// full resync must stay O(1)/static — every subsequent emit fans, fails (mailbox
// Full), and must NOT record a per-seq hole NOR rewrite/fsync mailboxes.json. Before
// the fix each emit appended one hole + rewrote the whole file forever.
func TestParkedResyncDeviceIsStaticAcrossEmits(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	dev := ids[0]

	srv.mailbox.mu.Lock()
	srv.mailbox.markResyncLocked(dev, srv.mailbox.disk.Devices[dev])
	_ = srv.mailbox.saveLocked()
	srv.mailbox.mu.Unlock()

	before, err := os.ReadFile(srv.mailbox.path)
	if err != nil {
		t.Fatal(err)
	}

	for i := 0; i < 50; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}

	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	holes, resync := len(mb.Holes), mb.Resync
	srv.mailbox.mu.Unlock()
	if holes != 0 {
		t.Fatalf("parked resync device grew %d holes across emits, want 0", holes)
	}
	if !resync {
		t.Fatal("device unexpectedly left resync while parked")
	}
	after, err := os.ReadFile(srv.mailbox.path)
	if err != nil {
		t.Fatal(err)
	}
	if string(before) != string(after) {
		t.Fatalf("mailboxes.json was rewritten while a resync device was parked")
	}
}

// TestReconcileHolesEnforcesCapIncrementally proves BLOCKER 1: reconcileHoles over a
// huge undelivered range must collapse into a bounded resync after ~maxDeviceHoles
// inserts, never materializing the whole range (O(range) memory, O(range^2) sorted
// insertion). A quadratic implementation would not finish this in the time budget.
func TestReconcileHolesEnforcesCapIncrementally(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	dev := ids[0]

	const n = 200000
	base := srv.mailbox.contiguousHead(dev) + 1
	big := make([]uint64, 0, n)
	for i := uint64(0); i < n; i++ {
		big = append(big, base+i)
	}
	done := make(chan struct{})
	go func() { srv.mailbox.reconcileHoles(dev, big); close(done) }()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("reconcileHoles did not bound the range incrementally (quadratic blowup)")
	}

	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	resync, holes, ahead := mb.Resync, len(mb.Holes), len(mb.Ahead)
	srv.mailbox.mu.Unlock()
	if !resync || holes != 0 || ahead != 0 {
		t.Fatalf("large range did not collapse to bounded resync: resync=%v holes=%d ahead=%d", resync, holes, ahead)
	}
}

// TestResyncDeviceExcludedFromWUntilGenuineCatchUp proves BLOCKER 2: a resync device
// (CHead snapped to JHead, hiding whether every frame was really delivered) must NOT
// clear Resync / participate — and must never expose W=JHead — until it has genuinely
// re-adopted the floor (Full cleared) and caught up (no holes). markParticipating
// must GUARD on that, not clear Resync unconditionally.
func TestResyncDeviceExcludedFromWUntilGenuineCatchUp(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	srv.mailbox.markResyncLocked(dev, mb) // Full=true, Resync=true, CHead=JHead=3
	_ = srv.mailbox.saveLocked()
	subs[0].participating = false // fresh reconnect sub (readStateTestServer pre-marks it)
	srv.mailbox.mu.Unlock()

	// Not re-adopted yet (Full still set): markParticipating must keep it excluded.
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	stillResync, participating := mb.Resync, subs[0].participating
	srv.mailbox.mu.Unlock()
	if !stillResync {
		t.Fatal("Resync cleared before genuine re-adoption (Full still set)")
	}
	if participating {
		t.Fatal("not-yet-adopted resync device wrongly marked participating")
	}
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W exposed %d over a not-yet-adopted resync device (an older undelivered frame), want 0", got)
	}

	// Genuine re-adoption: floor adopted + drained (Full cleared), no holes ⇒ rejoin W.
	srv.mailbox.mu.Lock()
	mb.Full = false
	srv.mailbox.mu.Unlock()
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	clearedResync, nowParticipating := mb.Resync, subs[0].participating
	srv.mailbox.mu.Unlock()
	if clearedResync || !nowParticipating {
		t.Fatalf("genuine catch-up did not admit device: resync=%v participating=%v", clearedResync, nowParticipating)
	}
	if got := srv.durableWatermark(); got != 3 {
		t.Fatalf("caught-up resync device W = %d, want 3", got)
	}
}

// TestResyncReconnectAtFloorForcesGapAdoption proves BLOCKER 2's connector half: a
// resync device reconnecting with resume_from == floor (the equality case a normal
// device skips) MUST still be driven through mailbox-gap adoption. Before the fix it
// skipped the gap, kept Full=true, and markParticipating cleared Resync anyway.
func TestResyncReconnectAtFloorForcesGapAdoption(t *testing.T) {
	const dev = "dev-af31fd290542"
	h := newSessionHarness(t, dev)
	h.srv.mailbox.mu.Lock()
	mb := h.srv.mailbox.disk.Devices[dev]
	mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741050", "9007199254741050"
	mb.JHead, mb.CHead = "9007199254741050", "9007199254741050"
	mb.Full, mb.Resync = true, true
	h.srv.mailbox.mu.Unlock()

	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+dev+`","secret":"`+h.secret+`","resume_from":"9007199254741050"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	if typ, _ := exactType(readRaw(t, conn)); typ != "mailbox_gap" {
		t.Fatal("resync device at resume_from==floor must be driven through mailbox_gap")
	}

	writeRaw(t, conn, []byte(`{"t":"mailbox_ack","m":"9007199254741050"}`))
	deadline := time.Now().Add(3 * time.Second)
	for {
		h.srv.mailbox.mu.Lock()
		r, full := mb.Resync, mb.Full
		h.srv.mailbox.mu.Unlock()
		if !r {
			if full {
				t.Fatal("Resync cleared but Full still set")
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("Resync not cleared after genuine floor adoption")
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// TestResyncOverCapReAdoptionEscapesTrap reproduces the box→device deaf-mailbox bug:
// a device parked in full resync (Full=true, CHead=0) whose undelivered durable range
// exceeds maxDeviceHoles could never clear Full. reconcileHolesAndClearFull recorded a
// hole per undelivered seq, hit the cap, and re-parked via markResyncLocked WITHOUT
// clearing Full — and because CHead snapped back to JHead (0 for a migrated/parked
// mailbox), every reconnect re-derived the same over-cap range and re-parked forever, so
// the device received no backlog AND no live items. The seed-window trim must instead
// skip old history, clear Full, and re-deliver the recent tail.
func TestResyncOverCapReAdoptionEscapesTrap(t *testing.T) {
	srv, ids, _ := readStateTestServer(t, 1)
	dev := ids[0]

	// Build an outbox with MORE than maxDeviceHoles durable frames.
	const n = maxDeviceHoles + 60
	var lastSeq uint64
	for i := 0; i < n; i++ {
		lastSeq = srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "hi", nil, "", nil, nil) })
	}

	// Force the exact production-broken parked state: fully drained, resync+full, and —
	// as a migrated/parked mailbox would load — CHead/JHead reconstructed to 0.
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	mb.Items = nil
	mb.Floor, mb.Head, mb.Ack = "744", "744", "744"
	mb.JHead, mb.CHead = "0", "0"
	mb.Holes, mb.Ahead = nil, nil
	mb.Full, mb.Resync = true, true
	_ = srv.mailbox.saveLocked()
	srv.mailbox.mu.Unlock()

	// A fresh reconnect subscription (fresh floor already adopted via the gap-ack path).
	_, _, _, sub, err := srv.mailbox.stateAndSubscribe(dev)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}

	// The re-adoption transition (connector runs this after the gap-ack).
	srv.clearResyncFull(dev)

	srv.mailbox.mu.Lock()
	full, holes := mb.Full, len(mb.Holes)
	srv.mailbox.mu.Unlock()
	if full {
		t.Fatal("Full still set after over-cap re-adoption: device is trapped deaf")
	}
	if holes > mailboxSeedItems {
		t.Fatalf("over-cap re-adoption recorded %d holes, want <= seed window %d", holes, mailboxSeedItems)
	}

	// reconcile + backfill re-delivers the seed-window tail; the device catches up to the
	// live durable head and is admitted to W.
	srv.reconcileDevice(dev)
	srv.mailbox.markParticipating(dev, sub, srv.durableHead())

	srv.mailbox.mu.Lock()
	resync, full2, chead := mb.Resync, mb.Full, mb.CHead
	srv.mailbox.mu.Unlock()
	if resync || full2 {
		t.Fatalf("device not caught up after re-adoption: resync=%v full=%v", resync, full2)
	}
	if want := journalString(lastSeq); chead != want {
		t.Fatalf("CHead=%s after catch-up, want durable head %s", chead, want)
	}
	if got := srv.durableWatermark(); got != lastSeq {
		t.Fatalf("W=%d after recovery, want %d", got, lastSeq)
	}

	// The most recent durable frame must actually have been enqueued for delivery
	// (streamed to the reconnecting device), not silently skipped.
	srv.mailbox.mu.Lock()
	var sawLast bool
	for _, it := range mb.Items {
		if it.J == journalString(lastSeq) {
			sawLast = true
		}
	}
	srv.mailbox.mu.Unlock()
	if !sawLast {
		t.Fatal("newest durable frame not delivered to the recovered device")
	}
}

// TestZeroParticipantsReadIsInert proves the SHOULD-FIX: with no participating device
// (W==0), an inbound read.j="0" must be fully inert — nothing accepted, persisted, or
// fanned (no setRead(0), no readCursor advance from "").
func TestZeroParticipantsReadIsInert(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]

	// Remove the only participant ⇒ zero participants ⇒ W==0.
	srv.mailbox.unsubscribe(dev, subs[0])
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W with no participant = %d, want 0", got)
	}

	// Direct primitive: acceptReadAtMost(0) must not persist "0".
	if advanced, err := srv.mailbox.acceptReadAtMost(0); err != nil || advanced {
		t.Fatalf("acceptReadAtMost(0) advanced=%v err=%v, want false/nil", advanced, err)
	}
	if got := srv.mailbox.readCursor(); got != "" {
		t.Fatalf("zero-participant acceptReadAtMost persisted cursor %q, want empty", got)
	}

	// Full frame path: read.j="0" must not be a bad_frame, and must persist/fan nothing.
	ctx := context.Background()
	if bad, _ := srv.handleSessionInput(ctx, dev, nil, []byte(`{"t":"read","j":"0"}`), writeSink()); bad {
		t.Fatal("zero-participant read.j=0 wrongly rejected as bad_frame")
	}
	if got := srv.mailbox.readCursor(); got != "" {
		t.Fatalf("zero-participant read.j=0 persisted cursor %q, want empty (inert)", got)
	}
	if last := drainRead(t, subs[0]); last != "" {
		t.Fatalf("zero-participant read.j=0 fanned a read frame %q, want none", last)
	}
}

// TestResyncPostCollapseUndeliveredFrameHoldsExclusion proves the WP-S1 fix6 core:
// a durable frame emitted while a device is PARKED in resync records no per-device
// hole (recording is suppressed while parked, keeping it O(1)/static — BLOCKER 1).
// So "holes empty" is NOT proof of catch-up. The device must stay excluded from W
// until its contiguous head CHead actually reaches the authoritative durable head,
// and a later higher enqueue must not fold CHead over the still-missing frame.
func TestResyncPostCollapseUndeliveredFrameHoldsExclusion(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	// Collapse into a full resync: CHead snapped to JHead=3, Full=true, Resync=true.
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	srv.mailbox.markResyncLocked(dev, mb)
	_ = srv.mailbox.saveLocked()
	subs[0].participating = false
	srv.mailbox.mu.Unlock()

	// Emit ONE durable frame while parked (seq 4). It blocks at the Full mailbox and —
	// because the device is genuinely parked (Resync && Full) — records NO hole and does
	// not grow: the parked-static BLOCKER 1 guarantee.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post4", nil, "", nil, nil) })
	srv.mailbox.mu.Lock()
	parkedHoles, parkedChead := len(mb.Holes), mb.CHead
	srv.mailbox.mu.Unlock()
	if parkedHoles != 0 {
		t.Fatalf("parked resync device grew %d holes on post-collapse emits (must stay static)", parkedHoles)
	}
	if parkedChead != "3" {
		t.Fatalf("parked CHead moved to %q on post-collapse emit (must not fold over the gap), want 3", parkedChead)
	}
	if dh := srv.durableHead(); dh != 4 {
		t.Fatalf("durableHead = %d, want 4 (post-collapse frame is durable in the outbox)", dh)
	}

	// Reconnect catch-up, THE BLOCKER 1 RACE. The gap-ack path (clearResyncFull) must
	// re-derive the parked gap (hole 4) ATOMICALLY with clearing Full. To reproduce the
	// exact over-accept schedule, emit a LATER higher frame (seq 5) in the window that,
	// in the OLD code, sat BETWEEN "clear Full" and "reconcile records holes" — where a
	// fold would leap the still-unrecorded gap 4. With the atomic fix, seq 5 lands after
	// hole 4 is already recorded, so foldContiguousLocked stops at 4 and CHead stays 3.
	srv.clearResyncFull(dev) // records hole{4} THEN clears Full, in one m.mu section
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post5", nil, "", nil, nil) })
	srv.mailbox.mu.Lock()
	raceHoles, raceChead, raceFull := append([]string(nil), mb.Holes...), mb.CHead, mb.Full
	srv.mailbox.mu.Unlock()
	if raceFull {
		t.Fatalf("clearResyncFull left Full set")
	}
	if raceChead != "3" {
		t.Fatalf("CHead = %q after post-clear emit of seq 5 — the fold LEAPED the unrecorded gap 4 (BLOCKER 1 over-accept), want 3", raceChead)
	}
	if len(raceHoles) == 0 || raceHoles[0] != "4" {
		t.Fatalf("hole for missing seq 4 not recorded atomically with clearing Full: holes=%v (BLOCKER 1)", raceHoles)
	}

	// Still excluded: CHead 3 < durableHead 5, and there is a recorded hole. W must be 0.
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	stillResync, participating := mb.Resync, subs[0].participating
	srv.mailbox.mu.Unlock()
	if !stillResync || participating {
		t.Fatalf("post-collapse device wrongly admitted: resync=%v participating=%v (undelivered frame above CHead)", stillResync, participating)
	}
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W = %d over a device missing durable frames 4,5, want 0", got)
	}

	// Catch-up-phase reconcile (Full cleared) MUST re-derive the remaining gap (5) from
	// the outbox and backfill 4,5 — proving gap detection is re-enabled once re-adopting.
	srv.reconcileDevice(dev)
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	clearedResync, nowParticipating, chead := mb.Resync, subs[0].participating, mb.CHead
	srv.mailbox.mu.Unlock()
	if clearedResync || !nowParticipating {
		t.Fatalf("genuine catch-up did not admit device: resync=%v participating=%v", clearedResync, nowParticipating)
	}
	if chead != "5" {
		t.Fatalf("CHead = %q after genuine catch-up, want 5", chead)
	}
	if got := srv.durableWatermark(); got != 5 {
		t.Fatalf("caught-up device W = %d, want 5", got)
	}
}

// TestResyncClearFullConcurrentEmitNoLeap is the concurrent (-race) form of BLOCKER 1:
// while a parked resync device is completing its gap-ack (clearResyncFull), a stream of
// emits fans concurrently. The atomic record-holes-then-clear-Full transition must never
// let a fold leap the parked gap. Whatever the interleave, once the dust settles the
// device's CHead must never exceed the smallest durable seq it was actually missing at
// clear time, and W (which excludes the resync device) must stay 0.
func TestResyncClearFullConcurrentEmitNoLeap(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]
	subs[0].participating = false // exclude the lone device: W is driven purely by resync state

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	srv.mailbox.markResyncLocked(dev, mb) // CHead=3, parked
	_ = srv.mailbox.saveLocked()
	srv.mailbox.mu.Unlock()

	// A frame emitted while parked (seq 4) is the gap that must not be leaped.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post4", nil, "", nil, nil) })

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		for i := 0; i < 40; i++ {
			srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "flood", nil, "", nil, nil) })
		}
	}()
	// Complete the gap-ack concurrently with the flood.
	srv.clearResyncFull(dev)
	wg.Wait()

	srv.mailbox.mu.Lock()
	chead := parseSeq(mb.CHead)
	holes := len(mb.Holes)
	srv.mailbox.mu.Unlock()
	// The device was missing seq 4 at clear time; CHead must never fold to or past it.
	if chead >= 4 {
		t.Fatalf("CHead = %d leaped the parked gap at seq 4 under concurrent emit (BLOCKER 1 over-accept)", chead)
	}
	if holes == 0 {
		t.Fatalf("no hole recorded for the parked gap after concurrent clear+emit (BLOCKER 1)")
	}
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W = %d while the resync device is still excluded, want 0", got)
	}
}

// TestResyncClearFullStaleSnapshotNoLeap is the deterministic form of the WP-S1 fix8
// stale-snapshot race: clearResyncFull captures an EMPTY durable snapshot (nothing above
// CHead at capture time), then a gap frame (seq 4) is emitted AFTER the snapshot. Pre-fix
// clearResyncFull ran off deliveryMu, so that emit completed during the clear window,
// bounced under Full, and its hole was SUPPRESSED (parked device) — leaving NO record of
// the gap. reconcileHolesAndClearFull then recorded its stale empty snapshot and cleared
// Full, and a later emit (seq 5) folded CHead straight over the never-delivered seq 4.
//
// The fix makes clearResyncFull hold deliveryMu across the whole transition, so the
// concurrent emit is serialized: it cannot append+bounce inside the clear window. It is
// blocked until Full is cleared and deliveryMu released, then lands normally — delivering
// seq 4 to the device. The seam afterResyncSnapshot reproduces the exact ordering
// deterministically.
//
// Discriminator: after the schedule, CHead must never sit at/above seq 4 unless the device
// actually HOLDS seq 4 (in Items). Pre-fix: CHead=5 but seq 4 was never delivered → FAIL.
// Post-fix: seq 4 is delivered before CHead reaches it → PASS.
func TestResyncClearFullStaleSnapshotNoLeap(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]
	subs[0].participating = false // W is driven purely by resync state

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	srv.mailbox.markResyncLocked(dev, mb) // CHead=3, parked (Resync&&Full)
	_ = srv.mailbox.saveLocked()
	srv.mailbox.mu.Unlock()

	// Seam: fire once, AFTER the (empty) snapshot is captured and BEFORE Full is cleared.
	// Drive the gap-frame emit from another goroutine so it races the transition exactly
	// as a live emit would. Pre-fix it completes here (deliveryMu free); post-fix it blocks
	// on deliveryMu (held by clearResyncFull) and only lands after the transition finishes.
	emitDone := make(chan struct{})
	fired := false
	srv.afterResyncSnapshot = func() {
		if fired {
			return
		}
		fired = true
		started := make(chan struct{})
		go func() {
			close(started)
			srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post4", nil, "", nil, nil) })
			close(emitDone)
		}()
		<-started
		// Give the emit a chance to either complete (pre-fix) or park on deliveryMu
		// (post-fix, where it can NEVER complete during this hold). A generous bound: the
		// trivial emit finishes well under it pre-fix; post-fix any value works.
		select {
		case <-emitDone:
		case <-time.After(500 * time.Millisecond):
		}
	}

	srv.clearResyncFull(dev)
	<-emitDone // the concurrent seq-4 emit has fully landed
	srv.afterResyncSnapshot = nil

	// A second emit (seq 5) then folds. Pre-fix it leaps the unrecorded seq 4.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post5", nil, "", nil, nil) })

	// The device HOLDS journal seq 4 iff its durable envelope (keyed by journal seq, not
	// the per-mailbox position counter mb.Items[].M) is resident.
	seq4ID := durableEnvelopeID("4", dev)
	srv.mailbox.mu.Lock()
	chead := parseSeq(mb.CHead)
	hasFour := false
	for _, it := range mb.Items {
		if it.ID == seq4ID {
			hasFour = true
		}
	}
	holes := len(mb.Holes)
	srv.mailbox.mu.Unlock()

	if chead >= 4 && !hasFour {
		t.Fatalf("CHead = %d advanced past seq 4 but the device NEVER received seq 4 "+
			"(stale-snapshot leap; holes=%d) — the clear window let an emit bounce+suppress under Full", chead, holes)
	}
	if !hasFour && holes == 0 {
		t.Fatalf("seq 4 was neither delivered nor recorded as a hole after the clear — it vanished (stale-snapshot suppression)")
	}
}

// TestResyncCHeadAdvanceReadmitsWithoutReconnect is BLOCKER 2 (liveness): admission runs
// while the device's CHead is behind the durable head (the fan hasn't delivered the last
// frame yet), so markParticipating excludes it. The later fan advances CHead to the head
// under m.mu — and the CHead-advance readmit wake must re-trigger admission so the device
// is admitted WITHOUT waiting for a reconnect. Here we drive the real fan (enqueue) and
// assert the readmit signal fires and the follow-up markParticipating admits.
func TestResyncCHeadAdvanceReadmitsWithoutReconnect(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]
	sub := subs[0]

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	// Put the device into a mid-catch-up resync state: Resync but Full cleared, connected,
	// caught up through seq 3 (CHead=3), not yet participating. This is the post-gap-ack
	// state right before final admission.
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	mb.Resync = true
	mb.Full = false
	sub.participating = false
	// drain any pending readmit so we observe only the one the fan below produces
	select {
	case <-sub.readmit:
	default:
	}
	srv.mailbox.mu.Unlock()

	// A new durable frame (seq 4) is emitted. outbox.add publishes durableHead=4 BEFORE
	// the fan delivers it. The one-shot admission that ran at CHead=3 saw durableHead=4
	// and excluded the device. Model that: admission BEFORE the fan lands leaves it out.
	// (We emit, which both publishes head 4 AND fans it to this connected device.)
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post4", nil, "", nil, nil) })

	// The fan-fold advanced CHead 3→4 for a resync device: a readmit wake must have fired.
	select {
	case <-sub.readmit:
	default:
		t.Fatalf("CHead advanced to the durable head but no readmit wake fired (BLOCKER 2 liveness gap)")
	}

	// The session's readmit handler re-attempts admission in outbox→m.mu order. It must
	// now admit: CHead=4 >= durableHead=4, Full cleared, no holes.
	srv.mailbox.markParticipating(dev, sub, srv.durableHead())
	srv.mailbox.mu.Lock()
	stillResync, participating, chead := mb.Resync, sub.participating, mb.CHead
	srv.mailbox.mu.Unlock()
	if stillResync || !participating {
		t.Fatalf("caught-up device not admitted on CHead-advance readmit: resync=%v participating=%v (needed a reconnect — BLOCKER 2)", stillResync, participating)
	}
	if chead != "4" {
		t.Fatalf("CHead = %q, want 4", chead)
	}
	if got := srv.durableWatermark(); got != 4 {
		t.Fatalf("admitted device W = %d, want 4", got)
	}
}

// TestResyncBackfillPersistFailureStaysExcluded proves a backfill that FAILS to
// deliver a frame (enqueue persistence failure during catch-up) leaves the device
// EXCLUDED — it must never end up (!Full && holes empty && participating) while a
// durable frame is missing, and W must never cover the missing frame. Once the
// persistence recovers and the frame genuinely lands, the device is admitted.
func TestResyncBackfillPersistFailureStaysExcluded(t *testing.T) {
	srv, ids, subs := readStateTestServer(t, 1)
	dev := ids[0]

	for i := 0; i < 3; i++ {
		srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "x", nil, "", nil, nil) })
	}
	srv.mailbox.mu.Lock()
	mb := srv.mailbox.disk.Devices[dev]
	srv.mailbox.markResyncLocked(dev, mb)
	_ = srv.mailbox.saveLocked()
	subs[0].participating = false
	goodPath := srv.mailbox.path
	srv.mailbox.mu.Unlock()

	// Post-collapse durable frame (seq 4) emitted while parked: no hole recorded.
	srv.emit(func(seq uint64) []byte { return msgFrame(seq, "a", "post4", nil, "", nil, nil) })

	// Enter catch-up: clear Full (gap-ack), then break mailbox persistence so the
	// catch-up reconcile/backfill CANNOT deliver seq4 (enqueue rolls back on save fail).
	dir := t.TempDir()
	blocker := filepath.Join(dir, "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv.mailbox.mu.Lock()
	mb.Full = false
	srv.mailbox.path = filepath.Join(blocker, "nested", "mailboxes.json") // saves now fail
	srv.mailbox.mu.Unlock()

	srv.reconcileDevice(dev) // reconstructs hole for 4 (in memory), backfill enqueue fails

	// The device must NOT be admissible: seq4 is undelivered. It is held out both by the
	// re-derived hole AND by CHead(3) < durableHead(4).
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	resync, full, holes, chead, participating := mb.Resync, mb.Full, len(mb.Holes), mb.CHead, subs[0].participating
	srv.mailbox.mu.Unlock()
	if !resync || participating {
		t.Fatalf("backfill-failed device wrongly admitted: resync=%v participating=%v", resync, participating)
	}
	if full && holes == 0 {
		// A collapse back to parked (Full=true) is an acceptable exclusion; what must
		// NEVER happen is (!Full && holes==0 && admitted) over the missing frame.
	}
	if !full && holes == 0 && chead != "4" {
		t.Fatalf("backfill-failed device reached !Full && holes-empty while CHead=%q < durableHead=4 (undelivered frame hidden)", chead)
	}
	if got := srv.durableWatermark(); got != 0 {
		t.Fatalf("W = %d over an undelivered (backfill-failed) frame, want 0", got)
	}

	// Recover persistence: the frame is now deliverable. Genuine catch-up ⇒ admitted.
	srv.mailbox.mu.Lock()
	srv.mailbox.path = goodPath
	srv.mailbox.mu.Unlock()
	srv.reconcileDevice(dev)
	srv.mailbox.markParticipating(dev, subs[0], srv.durableHead())
	srv.mailbox.mu.Lock()
	clearedResync, nowParticipating, chead2 := mb.Resync, subs[0].participating, mb.CHead
	srv.mailbox.mu.Unlock()
	if clearedResync || !nowParticipating || chead2 != "4" {
		t.Fatalf("after recovery genuine catch-up failed: resync=%v participating=%v chead=%q", clearedResync, nowParticipating, chead2)
	}
	if got := srv.durableWatermark(); got != 4 {
		t.Fatalf("recovered device W = %d, want 4", got)
	}
}
