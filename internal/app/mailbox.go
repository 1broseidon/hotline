package app

import (
	"encoding/json"
	"errors"
	"fmt"
	"math/big"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	mailboxRetention = 7 * 24 * time.Hour
	mailboxMaxItems  = 10000
	mailboxSeedItems = 200
	// presenceLeaseTimeout is the box-side liveness lease: a foreground
	// subscriber whose lease has not been refreshed within this window is
	// treated as away for the enqueue-time push decision. 60s = 2×25s app ping
	// interval + 10s scheduling/network margin. This is the correctness backstop
	// behind the explicit presence frame — it fixes even old apps (which already
	// ping every 25s while foregrounded) with no new heartbeat mechanism.
	presenceLeaseTimeout = 60 * time.Second
	// maxDeviceHoles / maxDeviceAhead bound the per-device out-of-order tracking
	// (blocker #3). A device that falls this far behind — an interior hole with a
	// long tail of higher delivered seqs (Ahead), or a long run of undeliverable
	// frames (Holes, e.g. a mailbox stuck Full while emits continue) — is collapsed
	// into a FULL RESYNC instead of accumulating unbounded state: its tracking is
	// dropped, its mailbox reset so it re-adopts a fresh floor on next connect, and
	// it is EXCLUDED from W until it reconnects and catches up. The caps are large
	// enough that only a genuinely stuck/abandoned device trips them.
	maxDeviceHoles = 1024
	maxDeviceAhead = 1024
)

var ErrMailboxFull = errors.New("mailbox full")

type MailboxItem struct {
	T         string          `json:"t"`
	M         string          `json:"m"`
	J         string          `json:"j"`
	ID        string          `json:"id"`
	Payload   json.RawMessage `json:"payload"`
	CreatedAt string          `json:"created_at,omitempty"`
}

type mailboxRecord struct {
	Floor string `json:"floor"`
	Head  string `json:"head"`
	Ack   string `json:"ack"`
	// JHead is the highest DURABLE journal seq (global j-space) ever enqueued into
	// this device's mailbox. Unlike Head — which counts items in per-device m-space
	// and is unrelated to journal seqs — JHead survives ack-trimming and the
	// full-reset. It is a monotone MAX (it ignores interior holes) and so is NOT the
	// read-acceptance watermark; CHead below is. JHead is retained for diagnostics
	// and conservative reconstruction only. omitempty so an old binary drops it on
	// save (rollback-safe).
	JHead string `json:"jhead,omitempty"`
	// CHead is this device's highest CONTIGUOUS durable-delivered journal seq: the
	// largest seq N such that the device has received EVERY durable frame with seq
	// <= N. A hole at durable seq k FREEZES CHead at the durable seq just below k
	// until k is actually delivered (backfilled) — regardless of higher seqs
	// succeeding. This is what makes the read watermark hole-free by construction:
	// the box accepts read.j only up to W = MIN CHead over active devices, so an
	// interior hole on ANY device holds W below it even as higher seqs fan. CHead is
	// monotone, survives ack-trimming/full-reset, and only ever counts truly
	// delivered-and-durably-saved frames. omitempty; an old file reconstructs it
	// conservatively to 0 (UNDER-estimates W, never over-accepts).
	CHead string `json:"chead,omitempty"`
	// Ahead holds durable seqs > CHead that HAVE been delivered to this device out
	// of order (i.e. above an open hole), kept sorted ascending. When the lowest
	// hole is finally filled, the contiguous run folds into CHead and drains from
	// Ahead. Bounded in practice (holes are rare drop/full events).
	Ahead []string `json:"ahead,omitempty"`
	// Holes holds durable seqs > CHead that are KNOWN durable (the box attempted to
	// fan them here) but were NOT delivered to this device (enqueue failed / mailbox
	// full / reactivation gap), kept sorted ascending. The smallest hole caps CHead.
	// Backfill (reconcile on attach) re-enqueues the missing frames, clearing holes
	// and advancing CHead — until then W stays below the hole.
	Holes []string `json:"holes,omitempty"`
	Full  bool     `json:"full,omitempty"`
	// Resync marks a device that fell too far behind (its Holes/Ahead tracking blew
	// past the cap) and was collapsed into a full resync (blocker #3). While set, the
	// device is EXCLUDED from W regardless of its CHead — its state is not trusted to
	// pin or advance the watermark — until it reconnects, re-adopts a fresh floor, and
	// catches up (markParticipating clears it). omitempty so an old binary drops it on
	// save (rollback-safe) and old files load as not-resyncing.
	Resync bool          `json:"resync,omitempty"`
	Items  []MailboxItem `json:"items"`
}

type mailboxDisk struct {
	Devices map[string]*mailboxRecord `json:"devices"`
	// Read is the shared read cursor: the highest JOURNAL seq (j-space, global —
	// never per-device mailbox m-space) the single user has read on ANY device.
	// Monotone max-merged across every device's `read` frame (§4). omitempty so
	// an old binary that never learned the field drops it on its next save (the
	// documented rollback: cursor resets to 0, one-time "all unread" blip).
	Read string `json:"read,omitempty"`
}

type mailboxSubscriber struct {
	items      chan MailboxItem
	transients chan []byte
	controls   chan string
	overflow   chan struct{}
	// readmit is a coalescing (buffered-1) wake fired when a still-excluded resync
	// device's contiguous head advances via a fan-fold (BLOCKER 2). The connected
	// session selects on it and re-attempts markParticipating(durableHead()) in the
	// correct outbox→m.mu lock order, so a device that genuinely reaches
	// CHead>=durableHead is admitted WITHOUT waiting for a reconnect. A plain signal
	// (no payload): the session re-derives the live durableHead itself.
	readmit chan struct{}
	// Presence is tracked PER SUBSCRIBER and guarded by localMailbox.mu. A
	// subscriber is "present" (suppresses push; the item is delivered live) while
	// it is foreground AND its foreground lease is fresh (now-lastForeground <
	// presenceLeaseTimeout). presence:background latches background immediately;
	// a later ping/ack refreshes the lease but does NOT clear the latch — only a
	// presence:foreground frame or a fresh hello subscription returns it to
	// foreground. A device is "away" (push-eligible) when no subscriber is
	// present.
	background     bool
	lastForeground time.Time
	// participating gates this subscriber's device into the read watermark W. It is
	// set (markParticipating) only AFTER the session has completed reconcile/backfill
	// and any mailbox-gap adoption — i.e. the device is connected AND caught up (its
	// CHead honestly reflects delivery, with every gap recorded as a real Hole). A
	// device with no participating subscriber (disconnected, or connected-but-still-
	// reconciling) is excluded from W: it neither pins W low nor lets W leap. This is
	// the "a device participates in W only once it is caught up" model.
	participating bool
}

type localMailbox struct {
	mu                 sync.Mutex
	path               string
	disk               mailboxDisk
	subs               map[string]map[*mailboxSubscriber]struct{}
	onPushIntent       func(deviceID string, item MailboxItem)
	onCustomPushIntent func(deviceID string, item MailboxItem, intent pushIntent)
	clock              func() time.Time
}

func newLocalMailbox(path string) (*localMailbox, error) {
	m := &localMailbox{path: path, subs: map[string]map[*mailboxSubscriber]struct{}{}, clock: time.Now}
	m.disk.Devices = map[string]*mailboxRecord{}
	if data, err := os.ReadFile(path); err == nil {
		if err := json.Unmarshal(data, &m.disk); err != nil {
			return nil, fmt.Errorf("decode mailboxes: %w", err)
		}
	} else if !os.IsNotExist(err) {
		return nil, err
	}
	if m.disk.Devices == nil {
		m.disk.Devices = map[string]*mailboxRecord{}
	}
	for _, mb := range m.disk.Devices {
		normalizeMailbox(mb)
	}
	// A corrupt/hostile read cursor on disk must not poison max-merge comparisons
	// (decimalCmp assumes canonical decimals). Empty means "no read yet"; anything
	// non-decimal is dropped back to that.
	if m.disk.Read != "" && !validDecimal(m.disk.Read) {
		m.disk.Read = ""
	}
	return m, nil
}

func normalizeMailbox(m *mailboxRecord) {
	if !validDecimal(m.Floor) {
		m.Floor = "0"
	}
	if !validDecimal(m.Head) {
		m.Head = m.Floor
	}
	if !validDecimal(m.Ack) {
		m.Ack = m.Floor
	}
	// Reconstruct the durable journal high-water for mailboxes persisted before
	// JHead existed (or with a corrupt value): the max J among current items is a
	// safe lower bound. If items were already ack-trimmed away this under-estimates
	// (a one-time, boot-only conservative stall — never an over-accept), which is
	// the safe direction for B2 seeding.
	if !validDecimal(m.JHead) {
		m.JHead = "0"
		for _, it := range m.Items {
			if validDecimal(it.J) && decimalCmp(it.J, m.JHead) > 0 {
				m.JHead = it.J
			}
		}
	}
	// The contiguous durable head is the read-acceptance watermark. For a file
	// written before CHead existed (or with a corrupt value) reconstruct it
	// CONSERVATIVELY to 0: JHead is a monotone MAX that hides interior holes, so it
	// is unsafe to reuse here. Starting at 0 UNDER-estimates W (a one-time, boot-only
	// stall until frames re-fan) and can never over-accept a not-fully-delivered seq.
	if !validDecimal(m.CHead) {
		m.CHead = "0"
	}
	// Drop any Ahead/Holes entries that are corrupt or already <= CHead (stale after
	// a reconstruction). Keep both sorted ascending — recordDelivered/foldContiguous
	// and durableWatermark rely on Holes[0] being the smallest open hole.
	m.Ahead = sanitizeSeqSet(m.Ahead, m.CHead)
	m.Holes = sanitizeSeqSet(m.Holes, m.CHead)
}

// sanitizeSeqSet keeps only valid decimal seqs strictly greater than floor,
// deduplicated and sorted ascending. Used to normalize the persisted Ahead/Holes
// sets so downstream min/contiguity logic can trust their ordering.
func sanitizeSeqSet(in []string, floor string) []string {
	var out []string
	for _, s := range in {
		if !validDecimal(s) || decimalCmp(s, floor) <= 0 {
			continue
		}
		out = seqInsert(out, parseSeq(s))
	}
	return out
}

// seqInsert returns list with seq inserted, kept sorted ascending and deduped.
func seqInsert(list []string, seq uint64) []string {
	s := journalString(seq)
	for i, e := range list {
		v := parseSeq(e)
		if v == seq {
			return list
		}
		if v > seq {
			list = append(list, "")
			copy(list[i+1:], list[i:])
			list[i] = s
			return list
		}
	}
	return append(list, s)
}

// seqRemove returns list with seq removed if present (order preserved).
func seqRemove(list []string, seq uint64) []string {
	for i, e := range list {
		if parseSeq(e) == seq {
			return append(list[:i:i], list[i+1:]...)
		}
	}
	return list
}

// seqIn reports whether seq is present in list.
func seqIn(list []string, seq uint64) bool {
	for _, e := range list {
		if parseSeq(e) == seq {
			return true
		}
	}
	return false
}

func validDecimal(s string) bool {
	if s == "0" {
		return true
	}
	if s == "" || s[0] == '0' {
		return false
	}
	for _, c := range s {
		if c < '0' || c > '9' {
			return false
		}
	}
	return true
}

func decimalCmp(a, b string) int {
	if len(a) != len(b) {
		if len(a) < len(b) {
			return -1
		}
		return 1
	}
	return strings.Compare(a, b)
}

func decimalNext(s string) string {
	n := new(big.Int)
	if _, ok := n.SetString(s, 10); !ok {
		n.SetInt64(0)
	}
	return n.Add(n, big.NewInt(1)).String()
}

func deviceSuffix(deviceID string) string { return strings.TrimPrefix(deviceID, "dev-") }

func durableEnvelopeID(j string, deviceID string) string {
	return "env-j" + j + "-d" + deviceSuffix(deviceID)
}

func (m *localMailbox) provision(deviceID string, loadSeed func() []journalFrame) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.disk.Devices[deviceID]; ok {
		return nil
	}
	// Only read/parse the append-only outbox journal when we are actually creating
	// a missing mailbox (the loader is a no-op-avoiding lazy read, see
	// provisionMailbox).
	seed := loadSeed()
	mb := &mailboxRecord{Floor: "0", Head: "0", Ack: "0", JHead: "0", CHead: "0"}
	m.disk.Devices[deviceID] = mb
	if len(seed) > mailboxSeedItems {
		seed = seed[len(seed)-mailboxSeedItems:]
	}
	for _, jf := range seed {
		if !durableContent(jf.data) {
			continue
		}
		m.enqueueLockedAt(deviceID, fmt.Sprint(jf.seq), jf.data, jf.createdAt)
		// A freshly provisioned device is contiguous from the seed window forward:
		// the seeds arrive in ascending durable order with no holes, so CHead folds
		// straight up to the last seeded seq. Durable seqs BELOW the seed window are
		// intentionally not delivered (the device starts at a floor/gap) and are NOT
		// holes — so a new device never drags W down for history it legitimately
		// skipped.
		m.recordDeliveredLocked(mb, jf.seq)
	}
	// Roll back the in-memory record if it did not persist: a half-created mailbox
	// that never hit disk must NOT masquerade as provisioned (the existence check
	// above would otherwise return nil forever and never retry the save, welcoming
	// a client onto volatile state that vanishes on restart).
	if err := m.saveLocked(); err != nil {
		delete(m.disk.Devices, deviceID)
		return err
	}
	return nil
}

func durableContent(raw []byte) bool {
	t, ok := exactType(raw)
	if !ok {
		return false
	}
	switch t {
	case "msg", "sent", "edit", "react":
		return true
	default:
		return false
	}
}

// enqueue preserves the existing generic insertion path for every caller and
// test. Custom notification intents use enqueueWithPushIntent so their
// enqueue-time presence decision shares this same durable insertion boundary.
func (m *localMailbox) enqueue(deviceID, j string, payload []byte) (MailboxItem, error) {
	return m.enqueueWithPushIntent(deviceID, j, payload, nil)
}

func (m *localMailbox) enqueueAt(deviceID, j string, payload []byte, createdAt string) (MailboxItem, error) {
	return m.enqueueWithPushIntentAt(deviceID, j, payload, createdAt, nil, false)
}

func (m *localMailbox) enqueueWithPushIntent(deviceID, j string, payload []byte, intent *pushIntent) (MailboxItem, error) {
	return m.enqueueWithPushIntentAt(deviceID, j, payload, "", intent, true)
}

func (m *localMailbox) enqueueWithPushIntentAt(deviceID, j string, payload []byte, createdAt string, intent *pushIntent, stampIfMissing bool) (MailboxItem, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil {
		return MailboxItem{}, fmt.Errorf("mailbox for %s not provisioned", deviceID)
	}
	if t, _ := exactType(payload); t == "typing" {
		m.publishTransientLocked(deviceID, payload)
		return MailboxItem{}, nil
	}
	id := durableEnvelopeID(j, deviceID)
	for _, item := range mb.Items {
		if item.ID == id {
			// Dedup hit: the frame is already resident. Still mark it delivered so a
			// reconcile/backfill re-drive of an item that is in Items but whose
			// contiguous head never folded (a migration/crash artifact) advances CHead
			// instead of stalling — WITHOUT leaping, because any true gap below it is a
			// recorded Hole (blocker #1). Idempotent for an already-contiguous seq.
			if validDecimal(j) && m.recordDeliveredLocked(mb, parseSeq(j)) {
				m.enforceBoundsLocked(deviceID, mb)
				_ = m.saveLocked()
				// A resync device's CHead just advanced: re-trigger admission (BLOCKER 2).
				if mb.Resync {
					m.publishReadmitLocked(deviceID)
				}
			}
			return item, nil
		}
	}
	if mb.Full {
		return MailboxItem{}, ErrMailboxFull
	}
	if len(mb.Items) >= mailboxMaxItems {
		// The v2 full behavior is a mailbox reset, never an oldest-tail insert:
		// the retained boundary advances and the next hello receives a gap. New
		// inserts remain stopped until the device adopts and acks that boundary.
		mb.Items = nil
		mb.Floor = mb.Head
		mb.Full = true
		_ = m.saveLocked()
		m.publishControlLocked(deviceID, "full")
		return MailboxItem{}, ErrMailboxFull
	}
	away := !m.hasPresentSubscriberLocked(deviceID, m.clock())
	// Snapshot every in-memory field enqueueLocked/recordDeliveredLocked will mutate
	// so a persist failure can roll ALL of it back (blocker #3). enqueueLocked bumps
	// Head/JHead and appends to Items; recordDeliveredLocked advances CHead/Ahead/
	// Holes. Without a full rollback, a failed save leaves the item in Items (its ID
	// then dedup-hits on retry → false success, no save, no publish) while the
	// contiguous head advanced past a frame that never durably landed.
	prevHead, prevJHead, prevCHead := mb.Head, mb.JHead, mb.CHead
	prevItems := mb.Items
	prevAhead := append([]string(nil), mb.Ahead...)
	prevHoles := append([]string(nil), mb.Holes...)
	if createdAt == "" && stampIfMissing {
		createdAt = m.clock().UTC().Format(time.RFC3339Nano)
	}
	item := m.enqueueLockedAt(deviceID, j, payload, createdAt)
	advancedCHead := false
	if validDecimal(j) {
		advancedCHead = m.recordDeliveredLocked(mb, parseSeq(j))
	}
	if err := m.saveLocked(); err != nil {
		mb.Head, mb.JHead, mb.CHead = prevHead, prevJHead, prevCHead
		mb.Items = prevItems
		mb.Ahead, mb.Holes = prevAhead, prevHoles
		return MailboxItem{}, err
	}
	// Bound per-device tracking AFTER the item is durable (a resync collapse is its
	// own persisted step, so a save failure above never leaves a half-applied
	// resync). A successful enqueue can only overrun via the Ahead tail (a device
	// stuck behind one hole), so this is a no-op on the common path.
	if m.enforceBoundsLocked(deviceID, mb) {
		_ = m.saveLocked()
	}
	// A fan-fold that advanced a still-excluded resync device's CHead re-triggers its
	// admission (BLOCKER 2): the device may now have reached the live durable head, but
	// the one-shot markParticipating at attach already ran and saw it behind. The wake
	// makes the connected session re-check without waiting for a reconnect. enforceBounds
	// may have re-collapsed it to parked (Resync&&Full) — still Resync, and the re-check
	// is a safe no-op (Full holds it excluded), so the guard is just !participating.
	if advancedCHead && mb.Resync {
		m.publishReadmitLocked(deviceID)
	}
	m.publishLocked(deviceID, item)
	if away {
		switch {
		case intent != nil && m.onCustomPushIntent != nil:
			// A custom intent is attached only to this fresh durable insert. The
			// dedup, mailbox-full, and persistence-failure exits above never reach
			// this callback; backfill callers use the enqueue wrapper with nil.
			go m.onCustomPushIntent(deviceID, item, *intent)
		case intent == nil && pushEligible(payload) && m.onPushIntent != nil:
			// Keep the pre-FB44 generic push-intent behavior unchanged.
			go m.onPushIntent(deviceID, item)
		}
	}
	return item, nil
}

// pushEligible implements the push rule (ERRATA E10):
//   - msg: always eligible, element-only ones included (E6 guarantees their
//     text — the synthesized fallback join — is non-empty, so the preview is
//     the text as always);
//   - edit: eligible ONLY with non-empty user-readable text — an element-only
//     edit, whose text is exactly the synthesized fallback join of its
//     elements, is ineligible (job updates and terminal edits stay silent);
//   - react: eligible (unchanged).
func pushEligible(payload []byte) bool {
	t, _ := exactType(payload)
	switch t {
	case "msg", "react":
		return true
	case "edit":
		var p struct {
			Text     string `json:"text"`
			Elements []struct {
				Fallback string `json:"fallback"`
			} `json:"elements"`
		}
		if json.Unmarshal(payload, &p) != nil {
			return false
		}
		if strings.TrimSpace(p.Text) == "" {
			return false
		}
		if len(p.Elements) > 0 {
			fbs := make([]string, len(p.Elements))
			for i, el := range p.Elements {
				fbs[i] = el.Fallback
			}
			if p.Text == joinFallbacks(fbs) {
				return false // synthesized text = element-only edit
			}
		}
		return true
	default:
		return false
	}
}

func (m *localMailbox) enqueueLocked(deviceID, j string, payload []byte, stamp bool) MailboxItem {
	createdAt := ""
	if stamp {
		createdAt = m.clock().UTC().Format(time.RFC3339Nano)
	}
	return m.enqueueLockedAt(deviceID, j, payload, createdAt)
}

func (m *localMailbox) enqueueLockedAt(deviceID, j string, payload []byte, createdAt string) MailboxItem {
	mb := m.disk.Devices[deviceID]
	mb.Head = decimalNext(mb.Head)
	// Advance the durable journal high-water (monotone, j-space). This survives
	// ack-trimming and full-reset — unlike Head/Items — and is what boot seeds the
	// fully-fanned high-water from (B2).
	if validDecimal(j) && decimalCmp(j, mb.JHead) > 0 {
		mb.JHead = j
	}
	item := MailboxItem{T: "mailbox_item", M: mb.Head, J: j, ID: durableEnvelopeID(j, deviceID), Payload: append(json.RawMessage(nil), payload...), CreatedAt: createdAt}
	mb.Items = append(mb.Items, item)
	return item
}

func (m *localMailbox) expireLocked(mb *mailboxRecord) {
	cut := 0
	now := m.clock()
	for cut < len(mb.Items) {
		at, err := time.Parse(time.RFC3339Nano, mb.Items[cut].CreatedAt)
		if err != nil || now.Sub(at) < mailboxRetention {
			break
		}
		mb.Floor = mb.Items[cut].M
		cut++
	}
	if cut > 0 {
		mb.Items = append([]MailboxItem(nil), mb.Items[cut:]...)
	}
}

func (m *localMailbox) stateAndSubscribe(deviceID string) (floor, head string, items []MailboxItem, sub *mailboxSubscriber, err error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil {
		return "", "", nil, nil, errors.New("mailbox not found")
	}
	m.expireLocked(mb)
	_ = m.saveLocked()
	// A fresh subscription is foreground with a fresh lease: a new hello always
	// creates a foreground subscriber (the app only opens a socket while active),
	// which immediately suppresses phantom pushes for the reconnecting device.
	sub = &mailboxSubscriber{items: make(chan MailboxItem, 256), transients: make(chan []byte, 16), controls: make(chan string, 4), overflow: make(chan struct{}), readmit: make(chan struct{}, 1), lastForeground: m.clock()}
	if m.subs[deviceID] == nil {
		m.subs[deviceID] = map[*mailboxSubscriber]struct{}{}
	}
	m.subs[deviceID][sub] = struct{}{}
	return mb.Floor, mb.Head, append([]MailboxItem(nil), mb.Items...), sub, nil
}

func (m *localMailbox) unsubscribe(deviceID string, sub *mailboxSubscriber) {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.subs[deviceID], sub)
}

// setPresence records an explicit presence transition for one subscriber.
// foreground clears any background latch and refreshes the lease; background
// latches the subscriber away immediately (a later ping/ack will not reactivate
// it — see touchPresence).
func (m *localMailbox) setPresence(sub *mailboxSubscriber, foreground bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if foreground {
		sub.background = false
		sub.lastForeground = m.clock()
	} else {
		sub.background = true
	}
}

// touchPresence refreshes a subscriber's foreground lease on validated app
// activity (the existing 25s ping). It deliberately does NOT clear an explicit
// background latch: once the app has declared background, only a presence:
// foreground frame (or a fresh hello) brings it back.
func (m *localMailbox) touchPresence(sub *mailboxSubscriber) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if !sub.background {
		sub.lastForeground = m.clock()
	}
}

// subscriberPresentLocked reports whether a subscriber suppresses push: it must
// be foreground AND within its lease window. Callers hold m.mu.
func (m *localMailbox) subscriberPresentLocked(sub *mailboxSubscriber, now time.Time) bool {
	return !sub.background && now.Sub(sub.lastForeground) < presenceLeaseTimeout
}

// hasPresentSubscriberLocked implements the "any foreground-and-fresh subscriber
// suppresses push" rule: the device is away only when every subscriber is
// background, lease-expired, or absent. Callers hold m.mu.
func (m *localMailbox) hasPresentSubscriberLocked(deviceID string, now time.Time) bool {
	for sub := range m.subs[deviceID] {
		if m.subscriberPresentLocked(sub, now) {
			return true
		}
	}
	return false
}

// deviceAway is the enqueue-time push gate, exposed for tests: true when no
// subscriber for the device is present.
func (m *localMailbox) deviceAway(deviceID string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	return !m.hasPresentSubscriberLocked(deviceID, m.clock())
}

func (m *localMailbox) publishTransient(deviceID string, payload []byte) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.publishTransientLocked(deviceID, payload)
}

func (m *localMailbox) publishTransientLocked(deviceID string, payload []byte) {
	for sub := range m.subs[deviceID] {
		select {
		case sub.transients <- append([]byte(nil), payload...):
		default:
		}
	}
}

func (m *localMailbox) publishControlLocked(deviceID, code string) {
	for sub := range m.subs[deviceID] {
		select {
		case sub.controls <- code:
		default:
		}
	}
}

// publishReadmitLocked wakes every connected subscriber of deviceID to re-attempt
// resync admission (BLOCKER 2). Non-blocking coalescing send: the readmit channel is
// buffered-1, so a full buffer already carries a pending wake and the drop is safe.
// Callers hold m.mu; the session does the actual durableHead()+markParticipating
// re-check off-lock, preserving the outbox→m.mu order.
func (m *localMailbox) publishReadmitLocked(deviceID string) {
	for sub := range m.subs[deviceID] {
		if sub.participating {
			continue
		}
		select {
		case sub.readmit <- struct{}{}:
		default:
		}
	}
}

func (m *localMailbox) publishLocked(deviceID string, item MailboxItem) {
	for sub := range m.subs[deviceID] {
		select {
		case sub.items <- item:
		default:
			select {
			case <-sub.overflow:
			default:
				close(sub.overflow)
			}
			delete(m.subs[deviceID], sub)
		}
	}
}

func (m *localMailbox) ack(deviceID, cursor string) error {
	if !validDecimal(cursor) {
		return errors.New("mailbox_ack.m must be a decimal string")
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil {
		return errors.New("mailbox not found")
	}
	if decimalCmp(cursor, mb.Head) > 0 {
		return errors.New("mailbox_ack.m is ahead of head")
	}
	// Clearing Full for a device parked in RESYNC is owned by reconcileHolesAndClearFull
	// (BLOCKER 1): the gap-ack must NOT clear Full on its own, or a concurrent emit-fold
	// could leap the parked gap in the window before the catch-up reconcile records the
	// holes. A steady-state (non-resync) device clears Full here as before (verified
	// sound). Resync devices reach the atomic clear via the connector's gap-ack path.
	if mb.Full && !mb.Resync && decimalCmp(cursor, mb.Floor) >= 0 {
		mb.Full = false
	}
	if decimalCmp(cursor, mb.Ack) <= 0 {
		return m.saveLocked()
	}
	mb.Ack = cursor
	cut := 0
	for cut < len(mb.Items) && decimalCmp(mb.Items[cut].M, cursor) <= 0 {
		mb.Floor = mb.Items[cut].M
		cut++
	}
	if decimalCmp(cursor, mb.Floor) > 0 {
		mb.Floor = cursor
	}
	if cut > 0 {
		mb.Items = append([]MailboxItem(nil), mb.Items[cut:]...)
	}
	return m.saveLocked()
}

// setRead max-merges a device's reported read cursor (journal seq, j-space) into
// the single shared cursor and persists iff it advanced. It returns whether the
// cursor moved so the caller can decide to fan the transient (an idempotent
// no-advance re-send is skipped). The j must already be validated as a decimal
// AND bounded to the outbox cursor by the caller (the box owns the outbox; the
// mailbox does not) — setRead only enforces monotonicity here.
func (m *localMailbox) setRead(j string) (advanced bool, err error) {
	if !validDecimal(j) {
		return false, errors.New("read.j must be a decimal string")
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.setReadLocked(j)
}

// setReadLocked is the monotone max-merge-and-persist primitive shared by setRead
// (the plain unit-test entry) and acceptReadAtMost (the watermark-bounded entry).
// Callers hold m.mu.
func (m *localMailbox) setReadLocked(j string) (advanced bool, err error) {
	if m.disk.Read != "" && decimalCmp(j, m.disk.Read) <= 0 {
		return false, nil
	}
	prev := m.disk.Read
	m.disk.Read = j
	if err := m.saveLocked(); err != nil {
		// SF2: roll back the in-memory advance on a persist failure so an identical
		// retry re-attempts persist+fan. Without this the cursor is left advanced
		// in memory but never durable/fanned — the retry is a silent no-op and a
		// restart regresses the cursor. Mirrors the outbox record rollback pattern.
		m.disk.Read = prev
		return false, err
	}
	return true, nil
}

// acceptReadAtMost advances the shared read cursor to durable ONLY if durable ≤ W,
// where W = min CHead over participating devices — recomputed under the SAME lock
// as the persist. This closes the read-validation/activation race (blocker #2): a
// device becoming participating (with a lower CHead, dropping W) mid-validation is
// seen here, so a value its now-participating low head should reject cannot slip
// through between an earlier W snapshot and the persist. `durable` must already be
// a snapped durable seq. Returns whether the cursor advanced (so the caller fans).
func (m *localMailbox) acceptReadAtMost(durable uint64) (advanced bool, err error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	w := m.watermarkLocked()
	// W==0 means no participating device (read-state inert) or that some participant
	// has contiguously received nothing durable — either way there is no universally-
	// delivered durable frame to mark read. Journal seqs start at 1, so W==0 is never
	// a real acceptance target: nothing is accepted, persisted, or fanned (SHOULD-FIX).
	if w == 0 || durable > w {
		return false, nil
	}
	return m.setReadLocked(journalString(durable))
}

// readCursor returns the current shared read cursor ("" when nothing has been
// read yet). Used for the post-drain snapshot to a reconnecting device.
func (m *localMailbox) readCursor() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.disk.Read
}

// clampReadTo lowers the persisted shared read cursor to the durable journal
// high-water at boot. If outbox persistence had degraded (a durable seq a
// persisted read.j referenced never reached outbox.jsonl, so the seq is reused
// after restart), the restored cursor would otherwise sit at/above a seq a future
// durable message will inherit — pre-marking it read (B3). Clamping guarantees
// the cursor never points past the durable journal after a restart.
func (m *localMailbox) clampReadTo(durable uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.disk.Read == "" || decimalCmp(m.disk.Read, journalString(durable)) <= 0 {
		return
	}
	m.disk.Read = journalString(durable)
	_ = m.saveLocked()
}

// recordDeliveredLocked marks durable seq j as delivered to the device and folds
// any now-contiguous run into CHead. Idempotent: a replay of an already-contiguous
// seq (j <= CHead) is a no-op, so a reconciliation re-drive with an old seq never
// regresses the head. Callers hold m.mu.
func (m *localMailbox) recordDeliveredLocked(mb *mailboxRecord, j uint64) bool {
	if j <= parseSeq(mb.CHead) {
		return false
	}
	holesBefore, aheadBefore, cheadBefore := len(mb.Holes), len(mb.Ahead), mb.CHead
	mb.Holes = seqRemove(mb.Holes, j)
	mb.Ahead = seqInsert(mb.Ahead, j)
	m.foldContiguousLocked(mb)
	return len(mb.Holes) != holesBefore || len(mb.Ahead) != aheadBefore || mb.CHead != cheadBefore
}

// recordHoleLocked marks durable seq j as a KNOWN-durable frame this device did
// not receive (fan failure / full / reactivation gap), freezing CHead below it.
// A seq already contiguous (<= CHead) or already delivered (in Ahead) is ignored.
// Callers hold m.mu.
func (m *localMailbox) recordHoleLocked(mb *mailboxRecord, j uint64) {
	if j <= parseSeq(mb.CHead) || seqIn(mb.Ahead, j) {
		return
	}
	mb.Holes = seqInsert(mb.Holes, j)
}

// foldContiguousLocked advances CHead over every delivered seq that is now
// contiguous. Because Holes lists EVERY missing durable seq, everything durable
// below the smallest hole is delivered and folds into CHead; with no holes, the
// whole Ahead set is contiguous. Callers hold m.mu.
func (m *localMailbox) foldContiguousLocked(mb *mailboxRecord) {
	chead := parseSeq(mb.CHead)
	if len(mb.Holes) == 0 {
		for _, a := range mb.Ahead {
			if v := parseSeq(a); v > chead {
				chead = v
			}
		}
		mb.Ahead = nil
	} else {
		minH := parseSeq(mb.Holes[0]) // Holes kept sorted ascending
		var rest []string
		for _, a := range mb.Ahead {
			if v := parseSeq(a); v < minH {
				if v > chead {
					chead = v
				}
			} else {
				rest = append(rest, a)
			}
		}
		mb.Ahead = rest
	}
	mb.CHead = journalString(chead)
}

// recordHole marks durable seq j as missing for a device (fan failure at emit
// time). Best-effort persist: a save error keeps the hole in memory (W stays
// correct at runtime) and is logged by the caller. Locks m.mu.
func (m *localMailbox) recordHole(deviceID string, j uint64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil {
		return nil
	}
	// A device GENUINELY PARKED in full resync (Resync AND Full: disconnected, mailbox
	// reset to Floor=Head awaiting re-adoption) holds NO per-seq holes: it dropped its
	// tracking at collapse and will fully re-provision (re-adopt floor + re-drain) on
	// its next connect, and it is EXCLUDED from W the whole time. Recording a hole here
	// — plus the whole-file rewrite it triggers — on EVERY subsequent emit would make a
	// dead/parked device grow O(emits) in memory and fsync churn forever. Skip both so
	// its on-disk/in-memory state stays O(1) and static while parked (BLOCKER 1). Once
	// the device is mid-catch-up (Resync but Full CLEARED via gap-ack) it is actively
	// re-adopting and connected, so a live fan failure MUST record the hole again — that
	// hole is what keeps markParticipating from re-admitting it over an undelivered
	// frame (WP-S1 fix6). enforceBounds still re-collapses it to parked past the cap.
	if mb.Resync && mb.Full {
		return nil
	}
	before := len(mb.Holes)
	m.recordHoleLocked(mb, j)
	if len(mb.Holes) == before {
		return nil // no change (already delivered/contiguous)
	}
	m.enforceBoundsLocked(deviceID, mb)
	return m.saveLocked()
}

// reconcileHoles reconstructs a device's true delivery state against the
// authoritative append-only outbox: every durable seq in durableSeqs that is
// above the device's CHead and NOT already delivered (present in Ahead) is
// recorded as a Hole. This is the unified boot/reconnect reconcile — it turns a
// crash gap (a durable seq that reached outbox.jsonl but was never fanned), a
// reactivation gap (frames emitted while the device was inactive/unbound), and an
// old CHead-less mailbox's whole undelivered range into recorded Holes, so
// foldContiguousLocked can never leap them. A bounded run (over the cap) collapses
// the device into a full resync instead. Locks m.mu; persists iff something
// changed. durableSeqs need not be pre-filtered — non-durable/contiguous entries
// are ignored here.
func (m *localMailbox) reconcileHoles(deviceID string, durableSeqs []uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil {
		return
	}
	// A device GENUINELY PARKED in resync (Resync AND Full) must not re-accumulate holes
	// here: it will re-provision on connect and is excluded from W meanwhile (BLOCKER 1).
	// But once it is mid-catch-up (Resync, Full cleared by the gap-ack) this reconcile
	// MUST re-derive gaps from the authoritative outbox — a durable frame emitted while
	// it was parked (recorded no hole then) is otherwise invisible, and markParticipating
	// would clear Resync over it. Re-enabling gap detection during catch-up is the WP-S1
	// fix6 core: the reconstructed hole holds W below the missing frame until it lands.
	if mb.Resync && mb.Full {
		return
	}
	changed := false
	for _, seq := range durableSeqs {
		before := len(mb.Holes)
		m.recordHoleLocked(mb, seq)
		if len(mb.Holes) != before {
			changed = true
			// Enforce the cap INCREMENTALLY (BLOCKER 1): the moment tracking exceeds the
			// bound, collapse straight into a full resync — which drops Holes/Ahead — and
			// stop. The old code appended the ENTIRE undelivered range first (O(range)
			// temp memory, O(range^2) sorted seqInsert on a large journal) and only then
			// enforced the cap. Bounding here keeps it O(cap) regardless of journal size.
			if m.enforceBoundsLocked(deviceID, mb) {
				_ = m.saveLocked()
				return
			}
		}
	}
	if changed {
		_ = m.saveLocked()
	}
}

// reconcileHolesAndClearFull is the ATOMIC catch-up transition for a resync device's
// gap-ack (BLOCKER 1). In ONE m.mu critical section it (a) re-derives the device's
// holes for every undelivered durable seq above its CHead, THEN (b) clears Full — so
// no concurrent emit-fold can interleave between clearing Full and recording the holes.
// While Full is still set, an interleaving emit is bounced by the enqueue Full guard
// (ErrMailboxFull → recordHole, suppressed for a parked device) and CANNOT fold; the
// instant Full is cleared here every gap below is already a recorded Hole, so a later
// fold stops at the lowest hole. durableSeqs is computed by the CALLER from the outbox
// BEFORE taking m.mu, honoring the outbox→m.mu lock order (never take the outbox lock
// under m.mu). No-op unless the device is genuinely parked (Full set). If the re-derived
// gap overruns the cap, the device re-collapses to a parked full resync (Full stays set)
// rather than growing unbounded during catch-up (BLOCKER 3).
func (m *localMailbox) reconcileHolesAndClearFull(deviceID string, durableSeqs []uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb := m.disk.Devices[deviceID]
	if mb == nil || !mb.Full {
		return
	}
	// A resync device re-adopting a fresh floor cannot backfill an unbounded history
	// frame-by-frame: recording a Hole per undelivered durable seq would blow past
	// maxDeviceHoles and — before this guard — re-collapse the device into a parked
	// full resync via markResyncLocked WITHOUT ever clearing Full. Because that
	// collapse leaves CHead untouched (markResyncLocked snaps CHead up to JHead, which
	// is 0 for a mailbox migrated/parked before JHead existed), every subsequent
	// reconnect re-derives the identical over-cap range and re-parks: a device more
	// than maxDeviceHoles durable frames behind the outbox could NEVER clear Full and
	// went permanently deaf — no mailbox backlog AND no live items ever reached it.
	//
	// Fix: when the undelivered range exceeds the cap, adopt fresh-device semantics —
	// skip everything below a recent seed window (intentionally-skipped history, NOT
	// holes, exactly like a freshly provisioned device skipping pre-seed frames) by
	// snapping CHead up to the durable seq just below the window, and only track and
	// deliver the newest mailboxSeedItems durable frames. durableSeqs arrives ascending
	// (framesAfter reads the append-only journal in seq order). This bounds the holes
	// to the seed window (< cap) so Full clears cleanly and backfill re-delivers the
	// recent tail, breaking the trap while still handing the device its latest messages.
	if len(durableSeqs) > maxDeviceHoles {
		seedStart := len(durableSeqs) - mailboxSeedItems
		if seedStart < 0 {
			seedStart = 0
		}
		if seedStart > 0 {
			if base := journalString(durableSeqs[seedStart-1]); decimalCmp(base, mb.CHead) > 0 {
				mb.CHead = base
			}
			durableSeqs = durableSeqs[seedStart:]
		}
	}
	// Record holes FIRST, while Full is still set (so no emit can fold past them). The
	// parked-device hole suppression in recordHole/reconcileHoles is deliberately NOT in
	// play here: this is the controlled transition, recording directly into mb.Holes.
	for _, seq := range durableSeqs {
		before := len(mb.Holes)
		m.recordHoleLocked(mb, seq)
		if len(mb.Holes) != before && (len(mb.Holes) >= maxDeviceHoles || len(mb.Ahead) >= maxDeviceAhead) {
			// Over the cap during catch-up: snap back to a parked full resync (keeps
			// Full=true, drops Holes/Ahead) so per-device state stays O(cap). It will
			// re-provision on its next connect. enforceBoundsLocked is a no-op while
			// Resync&&Full, so collapse explicitly. The seed-window trim above keeps a
			// re-adopting device off this arm (its range is bounded to mailboxSeedItems),
			// so this now only fires if pre-existing Holes already sat near the cap.
			m.markResyncLocked(deviceID, mb)
			_ = m.saveLocked()
			return
		}
	}
	// Every undelivered durable frame above CHead is now a recorded Hole: clearing Full
	// can no longer let a fold leap the gap.
	mb.Full = false
	_ = m.saveLocked()
}

// enforceBoundsLocked collapses a device whose out-of-order tracking has blown
// past the cap into a full resync (blocker #3), keeping per-device state O(cap).
// Returns whether it changed anything (so the caller persists). A device already
// PARKED in resync (Resync AND Full) is left alone — it is already collapsed/static.
// A device mid-catch-up (Resync, Full cleared) whose re-derived holes/ahead overrun
// the cap IS re-collapsed here: it snaps back to a parked full resync (O(cap) bound
// preserved, BLOCKER 3) rather than growing the reconstructed gap unbounded during
// catch-up. Callers hold m.mu.
func (m *localMailbox) enforceBoundsLocked(deviceID string, mb *mailboxRecord) bool {
	if mb.Resync && mb.Full {
		return false
	}
	if len(mb.Holes) < maxDeviceHoles && len(mb.Ahead) < maxDeviceAhead {
		return false
	}
	m.markResyncLocked(deviceID, mb)
	return true
}

// markResyncLocked collapses a fallen-behind device into a full resync: it drops
// the unbounded Holes/Ahead tracking, snaps CHead up to the highest durable seq
// ever enqueued here (JHead) so a stale low head can't pin W, resets the mailbox
// so the device re-adopts a fresh floor and re-drains on its next connect, and
// sets Resync so it is EXCLUDED from W until it reconnects and catches up. Any
// connected subscriber is told to re-sync via a `full` control. Callers hold m.mu.
func (m *localMailbox) markResyncLocked(deviceID string, mb *mailboxRecord) {
	mb.Holes = nil
	mb.Ahead = nil
	if decimalCmp(mb.JHead, mb.CHead) > 0 {
		mb.CHead = mb.JHead
	}
	mb.Items = nil
	mb.Floor = mb.Head
	mb.Full = true
	mb.Resync = true
	m.publishControlLocked(deviceID, "full")
}

// markParticipating admits a device into W once its session has completed
// reconcile/backfill and any mailbox-gap adoption: it flags the subscriber
// participating and, for a device that was parked in full resync, clears Resync —
// but ONLY on a POSITIVE catch-up proof (BLOCKER 2 / WP-S1 fix6). markResyncLocked
// snaps CHead up to the scalar JHead (the collapse-time durable head), which hides
// any older interior undelivered frame; and hole-recording is suppressed while the
// device is parked, so "holes empty" is NOT trustworthy proof of catch-up on its own
// — a durable frame emitted post-collapse recorded no hole. So a resync device may
// clear Resync + participate only once it has:
//   - re-adopted the current floor and re-drained (Full cleared), AND
//   - no outstanding recorded holes, AND
//   - its contiguous delivered head CHead has actually REACHED the authoritative
//     current durable head (durableHead, the highest durable seq in the outbox /
//     ring-independent oracle). CHead >= durableHead is the positive proof it
//     provably holds every durable frame up to the live head — strictly stronger
//     than holes-empty, and it is precisely what a post-collapse undelivered frame
//     (durableHead above the JHead-snapped CHead) or a failed backfill leaves unmet.
//
// Clearing Resync short of this would expose W=CHead over an undelivered frame. Until
// then the resync device stays fully excluded — neither participating nor W-eligible.
// A non-resync device participates unconditionally (the verified-sound core path).
// durableHead is supplied by the caller from the outbox authority (0 when the box has
// no durable frame yet). Locks m.mu.
func (m *localMailbox) markParticipating(deviceID string, sub *mailboxSubscriber, durableHead uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if mb := m.disk.Devices[deviceID]; mb != nil && mb.Resync {
		// Not genuinely caught up yet: keep it excluded (do not participate, do not
		// clear Resync). It will re-run this after its floor adoption + backfill. The
		// CHead < durableHead arm catches a post-collapse/undelivered frame that left no
		// recorded hole (recording was suppressed while parked) — holes-empty alone is
		// insufficient proof of catch-up during resync.
		if mb.Full || len(mb.Holes) > 0 || parseSeq(mb.CHead) < durableHead {
			return
		}
		mb.Resync = false
		_ = m.saveLocked()
	}
	if sub != nil {
		sub.participating = true
	}
}

// deviceResync reports whether a device is currently parked in a full resync (used
// by the session to force floor re-adoption even when resume_from == floor). Locks
// m.mu.
func (m *localMailbox) deviceResync(deviceID string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb, ok := m.disk.Devices[deviceID]
	return ok && mb.Resync
}

// caughtUp reports whether a device has fully drained and has no outstanding holes
// (its CHead honestly reflects the live durable head). reconcileDevice returns this
// so the caller can decide whether a resync device is ready to rejoin W. Locks m.mu.
func (m *localMailbox) caughtUp(deviceID string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb, ok := m.disk.Devices[deviceID]
	return ok && !mb.Full && len(mb.Holes) == 0
}

// deviceParticipatingLocked reports whether a device currently has at least one
// participating (connected + caught-up) subscriber. Callers hold m.mu.
func (m *localMailbox) deviceParticipatingLocked(deviceID string) bool {
	for sub := range m.subs[deviceID] {
		if sub.participating {
			return true
		}
	}
	return false
}

// deviceIDs returns every device id with a persisted mailbox record. Used at boot
// to reconcile each device's holes against the outbox. Locks m.mu.
func (m *localMailbox) deviceIDs() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	ids := make([]string, 0, len(m.disk.Devices))
	for id := range m.disk.Devices {
		ids = append(ids, id)
	}
	return ids
}

// contiguousHead returns a device's highest contiguous durable-delivered journal
// seq (0 when it has no mailbox). Locks m.mu.
func (m *localMailbox) contiguousHead(deviceID string) uint64 {
	m.mu.Lock()
	defer m.mu.Unlock()
	mb, ok := m.disk.Devices[deviceID]
	if !ok {
		return 0
	}
	return parseSeq(mb.CHead)
}

// durableWatermark returns W = MIN over PARTICIPATING devices of each device's
// highest contiguous durable-delivered journal seq. Locks m.mu.
func (m *localMailbox) durableWatermark() uint64 {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.watermarkLocked()
}

// watermarkLocked computes W = MIN over PARTICIPATING devices (connected AND
// caught-up, and not mid-resync) of each device's highest contiguous durable-
// delivered journal seq. This is the hole-free read-acceptance bound under the
// "a device participates in W only once caught up" model:
//   - a hole on any participating device holds W below it even as higher seqs fan;
//   - a DISCONNECTED or still-RECONCILING device is excluded — it neither pins W
//     low (a dead paired device can't stall read-state) nor lets W leap past a
//     range it hasn't actually received;
//   - a device collapsed into a full RESYNC is excluded until it catches up.
//
// Zero participating devices ⇒ 0 (read-state is inert: nothing is accepted or
// fanned). Callers hold m.mu.
func (m *localMailbox) watermarkLocked() uint64 {
	var min uint64
	seen := false
	for id, mb := range m.disk.Devices {
		if mb.Resync || !m.deviceParticipatingLocked(id) {
			continue
		}
		h := parseSeq(mb.CHead)
		if !seen || h < min {
			min, seen = h, true
		}
	}
	return min
}

func parseSeq(s string) uint64 {
	n, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return 0
	}
	return n
}

func itemsAfter(items []MailboxItem, cursor, head string) []MailboxItem {
	var out []MailboxItem
	for _, item := range items {
		if decimalCmp(item.M, cursor) > 0 && decimalCmp(item.M, head) <= 0 {
			out = append(out, item)
		}
	}
	sort.Slice(out, func(i, j int) bool { return decimalCmp(out[i].M, out[j].M) < 0 })
	return out
}

func (m *localMailbox) saveLocked() error {
	if err := os.MkdirAll(filepath.Dir(m.path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(m.disk, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp, err := os.CreateTemp(filepath.Dir(m.path), ".mailboxes-*")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, m.path); err != nil {
		return err
	}
	return os.Chmod(m.path, 0o600)
}
