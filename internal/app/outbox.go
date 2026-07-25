package app

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// defaultOutboxCap is how many recent server→app frames the in-memory ring
// keeps for replay-on-reconnect. The ring is warmed on boot from the persisted
// outbox.jsonl (see newPersistentOutbox) and every frame is appended to it as it
// is issued, so history survives a process restart.
const defaultOutboxCap = 200

// The persisted outbox.jsonl is APPEND-ONLY and never trimmed: it is the
// permanent record of the whole thread — both sides — independent of whatever
// the harness compacts away. The in-memory ring is just the hot replay window;
// older history is served on demand via page() (the /history endpoint) so the
// client lazy-loads backwards. Plain text at ~200 B/line: even a year of heavy
// chat is a few tens of MB, and the operator can archive the file by hand if
// they ever care.

// outbox assigns the monotonic seq every server→app content frame carries and
// buffers the most recent frames so a reconnecting client can replay what it
// missed. seq is per-server (per state dir) and strictly increasing; gaps are
// legal — a client resumes from the highest seq it has seen.
//
// When constructed with newPersistentOutbox, every issued frame is also appended
// to outbox.jsonl and the ring + seq counter are restored from it on boot, so
// the replay window survives a bot restart. A plain newOutbox is memory-only
// (used by unit tests that don't exercise persistence).
type outbox struct {
	mu            sync.Mutex
	seq           uint64
	buf           []outEntry
	max           int
	path          string   // "" ⇒ memory-only (no persistence)
	f             *os.File // append handle for path; nil ⇒ not persisting
	deliveries    map[string]outEntry
	deliveryOrder []string
	// dangling records that the file's final line lacked a trailing newline (a
	// crash mid-append). openAppend terminates it before the first fresh write so
	// the next record can't glom onto the partial one.
	dangling bool
	clock    func() time.Time
}

type outEntry struct {
	seq       uint64
	data      []byte
	createdAt string
}

// persistLine is one record in outbox.jsonl: the seq plus the exact server→app
// frame bytes (embedded raw, not re-encoded as a string).
type persistLine struct {
	Seq         uint64          `json:"seq"`
	Frame       json.RawMessage `json:"frame"`
	DeliveryKey string          `json:"delivery_key,omitempty"`
	CreatedAt   string          `json:"created_at,omitempty"`
}

// newOutbox returns a memory-only ring capped at max entries (min 1). Frames are
// lost on restart; use newPersistentOutbox for durable history.
func newOutbox(max int) *outbox {
	if max < 1 {
		max = 1
	}
	return &outbox{max: max, deliveries: map[string]outEntry{}, clock: time.Now}
}

// newPersistentOutbox returns a ring backed by path (an outbox.jsonl file). On
// construction it warms the ring from the tail of the file (up to max entries)
// and restores the monotonic seq counter (max persisted seq), then opens the
// file for append. A missing file is a fresh start; a corrupt/partial trailing
// line (a crash mid-append) is skipped. If persistence can't be set up the ring
// still works in memory — history just won't survive the next restart. This runs
// single-threaded at server construction, before any client connects.
func newPersistentOutbox(max int, path string) *outbox {
	o := newOutbox(max)
	o.path = path
	o.load()
	o.openAppend()
	return o
}

// add assigns the next seq, builds the frame with it, stores the bytes in the
// ring (evicting the oldest past the cap), persists it, and returns both. The
// build callback keeps seq assignment and frame construction atomic under the
// lock; persistence rides the same lock so the file is a single-writer append.
func (o *outbox) add(build func(seq uint64) []byte) (uint64, []byte, string) {
	return o.addDelivery("", build)
}

// addDelivery persists the idempotency key in the same journal record as the
// device echo. A complete line therefore proves both that the turn was applied
// and which exact echo must be reconciled after a crash.
func (o *outbox) addDelivery(key string, build func(seq uint64) []byte) (uint64, []byte, string) {
	o.mu.Lock()
	defer o.mu.Unlock()
	o.seq++
	data := build(o.seq)
	createdAt := o.clock().UTC().Format(time.RFC3339Nano)
	entry := outEntry{seq: o.seq, data: data, createdAt: createdAt}
	o.buf = append(o.buf, entry)
	if len(o.buf) > o.max {
		o.buf = o.buf[len(o.buf)-o.max:]
	}
	if key != "" {
		o.rememberDeliveryLocked(key, entry)
	}
	o.persistLocked(o.seq, data, key, createdAt)
	return o.seq, data, createdAt
}

func (o *outbox) delivery(key string) (uint64, []byte, string, bool) {
	o.mu.Lock()
	defer o.mu.Unlock()
	entry, ok := o.deliveries[key]
	return entry.seq, append([]byte(nil), entry.data...), entry.createdAt, ok
}

func (o *outbox) rememberDeliveryLocked(key string, entry outEntry) {
	if key == "" {
		return
	}
	if _, exists := o.deliveries[key]; !exists {
		if len(o.deliveryOrder) >= defaultCIDDedupCap {
			delete(o.deliveries, o.deliveryOrder[0])
			o.deliveryOrder = o.deliveryOrder[1:]
		}
		o.deliveryOrder = append(o.deliveryOrder, key)
	}
	o.deliveries[key] = entry
}

// since returns copies of every buffered frame with seq strictly greater than
// after, oldest first — the replay set for a reconnecting client that reports
// last_seq = after. Frames older than the ring window are silently absent: a
// client whose last_seq predates the warmed ring simply resumes from the oldest
// frame the ring still holds (accepted, bounded loss — the same gap behaviour as
// the in-memory ring, now spanning restarts).
//
// TEST-ONLY, AND IT DROPS createdAt. since returns raw frame bytes and discards
// each entry's createdAt, so anything built from its output loses the frame's
// original timestamp. It has no production callers; the live replay path reads
// the mailbox, which preserves createdAt end to end. Do NOT wire this into
// replay or fanout — a replayed frame that gets re-dated at delivery is a real,
// previously-shipped bug class. Use the mailbox, or add a createdAt-carrying
// variant.
func (o *outbox) since(after uint64) [][]byte {
	o.mu.Lock()
	defer o.mu.Unlock()
	var out [][]byte
	for _, e := range o.buf {
		if e.seq > after {
			out = append(out, e.data)
		}
	}
	return out
}

// cursor is the highest seq issued so far — the value a fresh client is handed
// in its welcome frame.
func (o *outbox) cursor() uint64 {
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.seq
}

// highestDurableSeqAtMost returns the largest DURABLE journal seq ≤ target, and
// whether one exists. Only durable frames are appended to o.buf AND to
// outbox.jsonl (reserveTransient bumps the counter but retains/persists nothing),
// so every seq in either the ring or the file is by construction durable. This is
// the RING-INDEPENDENT durable oracle (blocker #2): a read.j that lands on a
// transient seq is snapped DOWN to the nearest durable seq so a transient value is
// never persisted as the read cursor — even after the seq has been EVICTED from the
// hot ring. The ring is the fast path; when target predates the ring window the
// append-only journal (the authoritative, never-trimmed durable record) is
// consulted. ok=false only when NO durable seq ≤ target exists anywhere.
func (o *outbox) highestDurableSeqAtMost(target uint64) (uint64, bool) {
	o.mu.Lock()
	defer o.mu.Unlock()
	best, ok := uint64(0), false
	var ringMin uint64
	haveRing := false
	for _, e := range o.buf {
		if !haveRing || e.seq < ringMin {
			ringMin, haveRing = e.seq, true
		}
		if e.seq <= target && e.seq > best {
			best, ok = e.seq, true
		}
	}
	// The ring holds EVERY durable seq ≥ ringMin, so if target is within the window
	// its answer is authoritative. Otherwise (target below the window, or the ring
	// is empty on a memory-only outbox) fall through to the persisted journal.
	if ok && haveRing && ringMin <= target {
		return best, true
	}
	if o.path == "" {
		return best, ok
	}
	data, err := os.ReadFile(o.path)
	if err != nil {
		return best, ok
	}
	for _, ln := range bytes.Split(data, []byte("\n")) {
		if len(bytes.TrimSpace(ln)) == 0 {
			continue
		}
		var pl persistLine
		if json.Unmarshal(ln, &pl) != nil {
			continue
		}
		if pl.Seq <= target && pl.Seq > best {
			best, ok = pl.Seq, true
		}
	}
	return best, ok
}

// reserveTransient assigns a live-only sequence without retaining a frame.
// Gaps in the durable journal are legal, and a transient-only sequence may be
// reused after a process restart because no reconnect can observe that frame.
func (o *outbox) reserveTransient() uint64 {
	o.mu.Lock()
	defer o.mu.Unlock()
	o.seq++
	return o.seq
}

// close releases the append handle. Safe to call on a memory-only outbox and
// idempotent enough for shutdown. Writes are direct syscalls (no user-space
// buffer), so frames are already durable before close.
func (o *outbox) close() {
	o.mu.Lock()
	defer o.mu.Unlock()
	if o.f != nil {
		_ = o.f.Close()
		o.f = nil
	}
}

// load reads the tail of the persisted file into the ring and restores the seq
// counter. Best-effort: a missing file leaves a fresh ring; corrupt lines are
// skipped so a crash mid-append can't wedge boot. Called before openAppend, at
// construction only (no lock needed).
func (o *outbox) load() {
	data, err := os.ReadFile(o.path)
	if err != nil {
		return // missing file (or unreadable) ⇒ fresh start
	}
	o.dangling = len(data) > 0 && data[len(data)-1] != '\n'
	var entries []outEntry
	for _, ln := range bytes.Split(data, []byte("\n")) {
		if len(bytes.TrimSpace(ln)) == 0 {
			continue
		}
		var pl persistLine
		if err := json.Unmarshal(ln, &pl); err != nil {
			continue // corrupt/partial line (e.g. crash mid-append) — skip it
		}
		if pl.Seq > o.seq {
			o.seq = pl.Seq // restore the monotonic counter from the max seen
		}
		entry := outEntry{seq: pl.Seq, data: []byte(pl.Frame), createdAt: pl.CreatedAt}
		entries = append(entries, entry)
		o.rememberDeliveryLocked(pl.DeliveryKey, entry)
	}
	if len(entries) > o.max {
		entries = entries[len(entries)-o.max:] // keep only the ring window
	}
	o.buf = entries
}

// openAppend opens the file for append. Best-effort: a failure leaves the ring
// memory-only (o.f == nil). Construction-time only.
func (o *outbox) openAppend() {
	if err := os.MkdirAll(filepath.Dir(o.path), 0o700); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app outbox: mkdir %s: %v (history won't persist)\n", filepath.Dir(o.path), err)
		return
	}
	f, err := os.OpenFile(o.path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app outbox: open %s: %v (history won't persist)\n", o.path, err)
		return
	}
	o.f = f
	if o.dangling {
		// Terminate a partial trailing line (crash mid-append) so the next
		// record starts on its own line instead of concatenating onto the stub.
		_, _ = o.f.Write([]byte("\n"))
		o.dangling = false
	}
}

// persistLocked appends one frame record to the file. Caller holds o.mu. A
// write error is logged and persistence is disabled (the in-memory ring keeps
// serving live clients) rather than failing the send. The file is never
// trimmed — it is the permanent thread record.
func (o *outbox) persistLocked(seq uint64, data []byte, deliveryKey, createdAt string) {
	if o.f == nil {
		return
	}
	line, err := json.Marshal(persistLine{Seq: seq, Frame: json.RawMessage(data), DeliveryKey: deliveryKey, CreatedAt: createdAt})
	if err != nil {
		return
	}
	line = append(line, '\n')
	if _, err := o.f.Write(line); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app outbox: append failed: %v (history persistence disabled)\n", err)
		_ = o.f.Close()
		o.f = nil
		return
	}
	if deliveryKey != "" {
		_ = o.f.Sync()
	}
}

// findByID returns the raw stored frame whose "id" field equals id, searching
// the hot ring first (newest-first, no I/O) and then a bounded tail of the
// persisted journal — the most recent ~500 records. Read-only; used to resolve
// the target of a reply so the harness gets the quoted sender and text. A blank
// id, a miss, or an unreadable file returns ok=false.
func (o *outbox) findByID(id string) ([]byte, bool) {
	if id == "" {
		return nil, false
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	for i := len(o.buf) - 1; i >= 0; i-- {
		if frameID(o.buf[i].data) == id {
			return o.buf[i].data, true
		}
	}
	if o.path == "" {
		return nil, false
	}
	data, err := os.ReadFile(o.path)
	if err != nil {
		return nil, false
	}
	lines := bytes.Split(data, []byte("\n"))
	const tail = 500
	start := 0
	if len(lines) > tail {
		start = len(lines) - tail
	}
	for i := len(lines) - 1; i >= start; i-- {
		if len(bytes.TrimSpace(lines[i])) == 0 {
			continue
		}
		var pl persistLine
		if err := json.Unmarshal(lines[i], &pl); err != nil {
			continue
		}
		if frameID([]byte(pl.Frame)) == id {
			return []byte(pl.Frame), true
		}
	}
	return nil, false
}

// page returns up to limit persisted frames with seq strictly below beforeSeq,
// oldest first — one backwards page of thread history for the /history
// endpoint (the client lazy-loads older chat as the user scrolls up). It reads
// the journal file per call: history paging is a human-scroll-rate operation
// and the file is line-oriented JSON, so a full read keeps the code honest and
// simple. A memory-only outbox pages from the ring instead. beforeSeq of 0
// means "from the newest".
//
// TEST-ONLY, AND IT DROPS createdAt. Like since, page returns raw frame bytes;
// the journal-read branch below also rebuilds each outEntry WITHOUT createdAt,
// so even the intermediate entries are stripped. It has no production callers
// today. Do NOT wire it into the replay path — replay must preserve the
// original createdAt, and this cannot. If /history is ever re-pointed here,
// carry createdAt through persistLine → outEntry first.
func (o *outbox) page(beforeSeq uint64, limit int) [][]byte {
	if limit < 1 {
		return nil
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	if beforeSeq == 0 {
		beforeSeq = o.seq + 1
	}
	var entries []outEntry
	if o.path == "" {
		entries = o.buf
	} else {
		data, err := os.ReadFile(o.path)
		if err != nil {
			entries = o.buf // fall back to the ring
		} else {
			for _, ln := range bytes.Split(data, []byte("\n")) {
				if len(bytes.TrimSpace(ln)) == 0 {
					continue
				}
				var pl persistLine
				if err := json.Unmarshal(ln, &pl); err != nil {
					continue
				}
				// NOTE: createdAt is deliberately not reconstructed here — see the
				// TEST-ONLY warning on page. Anything that needs it must not use page.
				entries = append(entries, outEntry{seq: pl.Seq, data: []byte(pl.Frame)})
			}
		}
	}
	var out [][]byte
	for i := len(entries) - 1; i >= 0 && len(out) < limit; i-- {
		if entries[i].seq < beforeSeq {
			out = append(out, entries[i].data)
		}
	}
	// Collected newest-first; flip to oldest-first for rendering.
	for i, j := 0, len(out)-1; i < j; i, j = i+1, j-1 {
		out[i], out[j] = out[j], out[i]
	}
	return out
}
