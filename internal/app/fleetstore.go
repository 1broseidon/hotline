package app

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"
)

// This file is the box's fleet (agent-to-agent) registry — Lane L1 of the A2A v2
// design (a2a-design-v2-2026-07-23.md §1). Fleet lanes get their OWN transport
// and their OWN durable state, sharing nothing with the operator relay path: the
// registry lives in <stateDir>/fleet/fleet.json (NOT relay-state.json, which is
// byte-untouched by every fleet operation — F5/F6), and each edge owns a
// <stateDir>/fleet/<edgeID>/ directory (journal.jsonl + state.json). An old
// binary cannot even see fleet rooms, so no RoomRecord Kind field is ever needed.
//
// Every mutation is a flock(LOCK_EX)-guarded read-modify-write on fleet.json (the
// access.Mutate / loop.Mutate idiom) so the running box and a CLI subcommand in a
// second process never race (lost edges, resurrected tombstones, cap races). The
// mint recipe is reused verbatim (mintRoom) but NEVER MintLinkMode, so a fleet
// operation cannot touch the operator current_room.

const (
	fleetDirName       = "fleet"
	fleetStateFile     = "fleet.json"
	fleetLockFile      = "fleet.json.lock"
	fleetJournalFile   = "journal.jsonl"
	fleetEdgeStateFile = "state.json"
)

// fleetLimits — the §5 abuse-envelope constants Lane L1 touches. Kept in one
// block so L2 can extend it (rate cap, journal rotation, decrypt-failure and
// pending-queue caps) without hunting the tree.
const (
	// fleetTextCap is the per-message fleet_msg text cap (§4/§5, F16): 16 KiB.
	fleetTextCap = 16 * 1024
	// fleetMaxFrameBytes is the outer decoder/transport bound (B3): a whole fleet
	// frame (envelope or inner) larger than this is refused before parsing. It
	// leaves headroom above fleetTextCap for the frame's other fields + e1 overhead.
	fleetMaxFrameBytes = 32 * 1024
	// fleetMaxEdges caps the number of live (non-tombstoned) fleet edges (§2/§5,
	// F6). Fleet edges are counted SEPARATELY from the operator 8-room cap.
	fleetMaxEdges = 16
	// fleetMaxAliasRunes caps a display alias.
	fleetMaxAliasRunes = 32
	// fleetMaxCIDLen caps a fleet_msg client id.
	fleetMaxCIDLen = 64
	// fleetMaxIdentField caps each welcome/from identity string.
	fleetMaxIdentField = 128
	// fleetJournalVersion freezes the on-disk journal entry shape (B4).
	fleetJournalVersion = 1
	// fleetInboundRateN / fleetInboundRateWindow are the L2 inbound abuse cap
	// (§5, F16): at most 60 accepted fleet_msg per 5 minutes per edge. Excess is
	// dropped (not journaled, not injected), logged to fleet.log, and counted in
	// the edge state.json (dropped_inbound) for observability.
	fleetInboundRateN      = 60
	fleetInboundRateWindow = 5 * time.Minute
	// fleetPendingCap is the L2 pending-outbound cap (§5, F16): the oldest queued
	// outbound fleet_msg is dropped (with a fleet.log line) past this depth.
	fleetPendingCap = 512
	// fleetJournalRotateBytes is the L4 journal-rotation threshold (§5/§6): once an
	// edge's journal.jsonl reaches this size, it is rotated to journal.jsonl.1
	// (replacing any prior .1) and a fresh journal.jsonl is started. Exactly ONE
	// prior generation is retained — the scan/replay readers concatenate .1 then the
	// live file, so a SINGLE rotation loses no dedup/WAL history; the generation
	// before last is dropped (the documented "keep one generation" bound). 32 MiB is
	// ~2000× the 16 KiB text cap, so a real fleet edge rotates rarely.
	fleetJournalRotateBytes = 32 * 1024 * 1024
)

// fleetJournalRotateBytesOverride is a TEST-ONLY seam: when > 0 it replaces the
// fleetJournalRotateBytes rotation threshold so a test can exercise rotation without
// writing 32 MiB. Zero in production (the documented const applies).
var fleetJournalRotateBytesOverride int64

var (
	// fleetEdgeIDRE matches an 8-char immutable edge id (base64url alphabet).
	fleetEdgeIDRE = regexp.MustCompile(`^[A-Za-z0-9_-]{8}$`)
	// fleetDeviceRE matches the stable device id a dialer presents: flt-<edgeID>.
	fleetDeviceRE = regexp.MustCompile(`^flt-[A-Za-z0-9_-]{8}$`)
	// fleetCIDRE matches a fleet_msg client id: bounded base64url/ascii-id charset.
	fleetCIDRE = regexp.MustCompile(`^[A-Za-z0-9_.:-]{1,64}$`)
)

// fleetKinds is the closed enum for fleet_msg.kind (§4, F9), validated at BOTH ends
// (fleet_send on the way out, deliverInbound on the way in). F1 extends it with the
// orchestrator vocabulary: `cancel` + `status_req` down (joining `task`=assign — see
// fleetDirectiveKinds) and `refuse` up (a first-class "outside my charter" answer;
// `accept` rides `ack`).
//
// The enum is the STRUCTURAL ceiling on authority: `restart`, pairing approval, access
// change, and cap escalation have no kind here, so a compromised orchestrator cannot
// express them at all. Do not add one.
//
// Wire compat: a peer on a pre-F1 binary protocol_errors on an unknown kind, so the new
// kinds are only usable once BOTH boxes run this binary. brief/task/result/ack/ping are
// unchanged and always safe.
var fleetKinds = map[string]bool{
	"brief": true, "task": true, "result": true, "ack": true, "ping": true,
	"cancel": true, "status_req": true, "refuse": true,
}

// FleetKindList is the human-readable enum for tool schemas and error strings.
const FleetKindList = "brief|task|result|ack|ping|cancel|status_req|refuse"

// FleetDirection is the pairing role for an edge: serve (this box minted the
// room and answers the peer's device hello) or dial (this box holds device
// creds and dials the peer's room — Lane L3).
type FleetDirection string

const (
	FleetServe FleetDirection = "serve"
	FleetDial  FleetDirection = "dial"
)

// FleetTombstone marks a removed edge (§2 rm). The edge dir (journal history) is
// retained; only the creds are zeroed and the edge is excluded from serving.
type FleetTombstone struct {
	At     string `json:"at"`
	Reason string `json:"reason,omitempty"`
}

// FleetUnreachable is the F2 RECOVERABLE dead-mark: a known-good dial edge whose
// handshakes have been failing long enough that the dialer dropped to the cold-retry
// tier. It is deliberately NOT a tombstone — creds are retained, the edge keeps
// serving and accepting frames, and one successful handshake clears it. Operator
// removal (`removed`) and peer revocation (`revoked`) stay terminal tombstones; only
// this state, which nothing but a network outage produces, is reversible.
//
// It exists because permanence was wrong: an ~8-minute relay wobble (54 × code=1006
// "unexpected EOF" in 24h) retired a working box-to-box edge forever, with no
// revive path in the CLI, while the peer kept queueing frames for a box that had
// written it off.
type FleetUnreachable struct {
	Since    string `json:"since"`
	Attempts int    `json:"attempts"`
	LastAt   string `json:"last_at,omitempty"`
}

// DeviceCreds is the dial-side (direction=dial) credential set the Lane L3
// dialer will use to run the device hello against the peer's fleet room. Stored
// on the registry entry AND mirrored into the edge state.json. Secret is zeroed
// on rm (patched in place — the cursor is never reset).
type DeviceCreds struct {
	DeviceID string `json:"device_id"`
	Room     string `json:"room"`
	RelayURL string `json:"relay_url"`
	Secret   string `json:"secret,omitempty"`
	Envelope bool   `json:"envelope"`
}

// FleetEdge is one registry entry (§1). edge_id is the immutable durable address
// everywhere (chat_id, journal, logs — F17); alias is display-only and
// renameable. Envelope is REQUIRED (e=1 always, F1). Serve edges keep the raw
// room Secret so the fleet handler can derive the e1 content keys; dial edges
// keep DeviceCreds instead. RelayURL is always the NORMALIZED rendezvous (B7);
// RelayOrigin records the operator-approved origin the dialer is allowed to use.
type FleetEdge struct {
	EdgeID      string          `json:"edge_id"`
	Alias       string          `json:"alias"`
	Direction   FleetDirection  `json:"direction"`
	Room        string          `json:"room"`
	RelayURL    string          `json:"relay_url"`
	RelayOrigin string          `json:"relay_origin,omitempty"`
	Secret      string          `json:"secret,omitempty"`
	DeviceCreds *DeviceCreds    `json:"device_creds,omitempty"`
	Envelope    bool            `json:"envelope"`
	PeerBoxName string          `json:"peer_box_name,omitempty"`
	PeerKeyFP   string          `json:"peer_key_fp,omitempty"`
	AddedAt     string          `json:"added_at"`
	LastSeenAt  string          `json:"last_seen_at,omitempty"`
	Tombstone   *FleetTombstone `json:"tombstone,omitempty"`
	// Authority is the F1 operator-granted orchestrator authority for INBOUND frames
	// on this edge (fleetauthority.go). It is written ONLY by `hotline fleet
	// grant|revoke` — never by any wire, session, or tool path — and is cleared with
	// the creds on any removal.
	Authority *FleetAuthority `json:"authority,omitempty"`
	// Unreachable is the F2 recoverable dead-mark (dial side): set when the dialer
	// drops to the cold-retry tier, cleared by the next successful handshake. Never a
	// tombstone — the edge stays live and its creds are retained.
	Unreachable *FleetUnreachable `json:"unreachable,omitempty"`
}

// Removed reports whether the edge is tombstoned.
func (e FleetEdge) Removed() bool { return e.Tombstone != nil }

// FleetEdgeView is the redacted DTO for `fleet ls --json` (B5): it NEVER carries
// a serve secret or a dial device_creds.secret. The storage struct (FleetEdge)
// is never serialized to a caller.
type FleetEdgeView struct {
	EdgeID      string          `json:"edge_id"`
	Alias       string          `json:"alias"`
	Direction   FleetDirection  `json:"direction"`
	Room        string          `json:"room"`
	RelayURL    string          `json:"relay_url"`
	RelayOrigin string          `json:"relay_origin,omitempty"`
	Envelope    bool            `json:"envelope"`
	DeviceID    string          `json:"device_id,omitempty"`
	PeerBoxName string          `json:"peer_box_name,omitempty"`
	PeerKeyFP   string          `json:"peer_key_fp,omitempty"`
	AddedAt     string          `json:"added_at"`
	LastSeenAt  string          `json:"last_seen_at,omitempty"`
	Tombstone   *FleetTombstone `json:"tombstone,omitempty"`
	// Authority is the F1 grant (never a secret — it is a key fingerprint the operator
	// already sees, plus timestamps), so the operator surfaces can render it.
	Authority *FleetAuthority `json:"authority,omitempty"`
	// Unreachable is the F2 recoverable dead-mark, surfaced so the operator can see a
	// cold-retrying edge without tailing fleet.log.
	Unreachable *FleetUnreachable `json:"unreachable,omitempty"`
}

// FleetMaxEdges exposes the live-edge cap for CLI reporting.
const FleetMaxEdges = fleetMaxEdges

// Redacted is the exported secret-free view of an edge (B5), for CLI JSON output.
func (e FleetEdge) Redacted() FleetEdgeView { return e.redacted() }

// redacted returns the view with every secret dropped.
func (e FleetEdge) redacted() FleetEdgeView {
	v := FleetEdgeView{
		EdgeID: e.EdgeID, Alias: e.Alias, Direction: e.Direction, Room: e.Room,
		RelayURL: e.RelayURL, RelayOrigin: e.RelayOrigin, Envelope: e.Envelope,
		PeerBoxName: e.PeerBoxName, PeerKeyFP: e.PeerKeyFP, AddedAt: e.AddedAt,
		LastSeenAt: e.LastSeenAt, Tombstone: e.Tombstone, Authority: e.Authority,
		Unreachable: e.Unreachable,
	}
	if e.DeviceCreds != nil {
		v.DeviceID = e.DeviceCreds.DeviceID
	}
	return v
}

// fleetState is the on-disk registry document.
type fleetState struct {
	Edges map[string]FleetEdge `json:"edges"`
}

// fleetPendingOut is one durably-queued outbound fleet_msg awaiting delivery or
// a peer fleet_ack (§4/§5). Frame is the COMPLETE frozen wire fleet_msg (the same
// bytes journaled dir=out); CID is the ACK KEY — the peer's fleet_ack echoes this
// exact cid and drains this exact entry (C1: acks are keyed by cid, not by any
// mixed-journal seq, so the two boxes never need a shared counter). Seq is kept
// for logging/journal cross-reference only. The queue is capped at fleetPendingCap
// (oldest dropped) and drained on every serve session attach, staying until the
// peer's fleet_ack echoes its cid.
type fleetPendingOut struct {
	Seq   uint64          `json:"seq"`
	CID   string          `json:"cid"`
	Frame json.RawMessage `json:"frame"`
	At    string          `json:"at"`
}

// fleetEdgeState is the per-edge runtime state.json (§1). L1 persists the cursor
// and the dial creds copy; L2 adds the pending-outbound queue, the delivered-
// inbound dedup set, the inbound-drop counter, and the key-fingerprint mismatch
// flag (§4/§5). L3 extends it further (generation, seen-CID ring). The cursor is
// NEVER reset by a rm (B6) — only creds are patched.
//
// v1 wire ack semantics (C1, frozen): fleet_ack carries the {cid} of the message
// being acknowledged — the receiver echoes the sender's cid. There is NO
// cross-box sequence agreement; delivery watermarks are per-cid membership, not a
// scalar. The pre-C1 PeerAckSeq/InAckSeq seq-watermark fields are GONE (any
// persisted value is ignored on load — a fresh L2 edge simply re-drives from the
// pending queue by cid).
type fleetEdgeState struct {
	DeviceCreds *DeviceCreds `json:"device_creds,omitempty"`
	// Cursor is the highest inbound journal seq committed, as a decimal string. It is
	// DERIVED from and reconciled to the durable journal (never the dedup authority —
	// that is the journal itself, see CommitInbound). Never reset by a rm (B6).
	Cursor string `json:"cursor"`
	// Pending is a legacy CACHE field, no longer the source of truth (review B4): the
	// outbound WAL is the journal (dir=out minus dir=ack). It is left unpopulated;
	// PendingOutbound derives the live queue from the journal on every call, so a
	// crash between the journal fsync and any state write can never strand an
	// outbound frame. Retained only so an old state.json still decodes.
	Pending []fleetPendingOut `json:"pending,omitempty"`
	// InboundDelivered is the bounded FIFO of inbound fleet_msg cids that were
	// successfully injected to the agent (H2). Startup replay of the inbound
	// journal skips any cid in this set, so a delivered turn is never re-injected
	// and an undelivered one (box died between journal and inject) is. Order-
	// independent, hole-free — unlike a mixed-journal seq watermark.
	InboundDelivered []string `json:"inbound_delivered,omitempty"`
	// DroppedInbound counts fleet_msg dropped by the inbound rate cap (§5). Never
	// resurfaced to the agent; surfaced to the operator (fleet ls / L4 fleet_state).
	DroppedInbound uint64 `json:"dropped_inbound,omitempty"`
	// KeyFPMismatch flags that a peer presented a from.key_fp differing from the
	// first-seen pinned PeerKeyFP (M5): the display never trusts the new claim and
	// the operator is warned (fleet.log + this flag). Cleared only by an operator rm.
	KeyFPMismatch bool `json:"key_fp_mismatch,omitempty"`
	// Generation counts inbound-cursor reconciliations (L3, §3.2 — the review's B3
	// fix). The durable journal is the single source of truth for inbound dedup and
	// the cursor; Generation bumps whenever a load-time reconcile realigns Cursor to
	// the journal's inbound tail (divergence in EITHER direction). Purely observational.
	Generation uint64 `json:"generation,omitempty"`
	// Activity is the L4 rolling 24h send/recv tally (§6): one bucket per UTC hour,
	// pruned to the trailing 24h on every bump. sent_24h / recv_24h (fleet ls +
	// fleet_state) are the sum of the in-window buckets. Bounded (≤25 buckets), so it
	// never bloats state.json. Persisted so counts survive a box restart.
	Activity []fleetActivityBucket `json:"activity,omitempty"`
	// PeerCaps is the L5 box-attested capabilities manifest last received from the
	// peer (caps-design §3), with a LOCAL received_at (staleness anchor). Latest-wins,
	// clamped on receipt, transient on the wire (never journaled). Survives a restart
	// so caps display works offline with staleness marking. nil = no caps yet (old
	// peer). Zeroed with the creds on rm.
	PeerCaps *FleetPeerCaps `json:"peer_caps,omitempty"`
}

// fleetActivityBucket is one UTC-hour tally of fleet_msg sent/received/dropped on an
// edge (L4 counters, §6). Hour is unix-epoch-hours (epoch/3600). Dropped counts
// inbound fleet_msg refused by the §5 rate cap in that hour.
type fleetActivityBucket struct {
	Hour    int64  `json:"hour"`
	Sent    uint64 `json:"sent"`
	Recv    uint64 `json:"recv"`
	Dropped uint64 `json:"dropped,omitempty"`
}

// windowCounts sums the sent/recv/dropped tallies for buckets within the trailing
// 24h of now (L4). A bucket exactly 24h old is excluded.
func (st fleetEdgeState) windowCounts(now time.Time) (sent, recv, dropped uint64) {
	cutoff := now.Add(-24*time.Hour).Unix() / 3600
	for _, b := range st.Activity {
		if b.Hour > cutoff {
			sent += b.Sent
			recv += b.Recv
			dropped += b.Dropped
		}
	}
	return sent, recv, dropped
}

// bumpActivity adds sent/recv/dropped counts to the current-hour bucket, first
// pruning any bucket older than the trailing 24h window (L4). Bounded to ≤25 live
// buckets.
func (st *fleetEdgeState) bumpActivity(now time.Time, sent, recv, dropped uint64) {
	cutoff := now.Add(-24*time.Hour).Unix() / 3600
	hour := now.Unix() / 3600
	kept := st.Activity[:0]
	for _, b := range st.Activity {
		if b.Hour > cutoff {
			kept = append(kept, b)
		}
	}
	st.Activity = kept
	for i := range st.Activity {
		if st.Activity[i].Hour == hour {
			st.Activity[i].Sent += sent
			st.Activity[i].Recv += recv
			st.Activity[i].Dropped += dropped
			return
		}
	}
	st.Activity = append(st.Activity, fleetActivityBucket{Hour: hour, Sent: sent, Recv: recv, Dropped: dropped})
}

// fleetInboundDedupCap bounds InboundDelivered. Fleet inbound is injected
// synchronously right after journaling, so the only undelivered entries are ones
// the sink could not take (startup before the fleet sink binds, or a sink error);
// they are delivered on the next start and enter the set. The set therefore never
// grows far ahead of an undelivered entry in practice; this cap is a safety bound.
const fleetInboundDedupCap = 2048

// fleetResumeHave bounds the cid list a side advertises in a fleet_resume frame
// (fleet leg v1.1, review B2): the last N inbound-committed cids, so the peer prunes
// already-delivered outbound without a body retransmit. N ≥ the pending cap so it
// always covers the peer's in-flight window.
const fleetResumeHave = fleetPendingCap

// fleetResumeGrace is how long the concurrent drain waits for the peer's
// fleet_resume (which prunes already-delivered outbound) before draining anyway —
// so a resume-capable peer causes zero body retransmission, and an old peer that
// sends none still gets its drain shortly (review B2).
const fleetResumeGrace = 250 * time.Millisecond

// fleetWelcomeTimeout bounds how long the dialer waits for welcome_fleet before
// closing and backing off (review B5): a dial to an empty relay (peer removed the
// room) never waits forever.
const fleetWelcomeTimeout = 30 * time.Second

// fleetHelloResend re-sends the device hello while awaiting welcome. The relay is a
// dumb pipe (§2): a hello sent before the peer's /c leg is connected is DROPPED, so
// the dialer periodically re-sends until welcome arrives (or the timeout fires),
// making both the initial connect race and reconnects robust without waiting the
// full welcome timeout. Safe: the peer only welcomes once its /c is up, and once the
// dialer is welcomed it stops re-sending.
const fleetHelloResend = 2 * time.Second

// fleetDialDeadAfter is the count of consecutive failed handshakes (no welcome) on a
// PREVIOUSLY-CONNECTED dial edge after which the dialer stops hot-retrying. Generous,
// so a transient peer reboot never trips it.
//
// F2 changed what happens NEXT. It used to tombstone the edge `unreachable` and zero
// its creds — terminal, with no revive path in the CLI, so ~8 minutes of relay churn
// permanently retired a known-good edge (the 2026-07-24 field finding).
// Now the dialer drops to the COLD-RETRY tier instead: the edge is flagged
// Unreachable (recoverable, creds retained) and keeps being dialed every
// fleetDialColdRetry, so it revives by itself when the network comes back. Only an
// authenticated revoked/unauthorized, bad creds, or an operator `fleet rm` still kills
// an edge for good.
const fleetDialDeadAfter = 12

// fleetDialColdRetry is the F2 cold tier's reconnect floor: once an edge is flagged
// unreachable the dialer stops hammering (a dead relay leg is not going to answer in
// 60s) but never stops trying, so recovery costs nothing and needs no operator.
const fleetDialColdRetry = 5 * time.Minute

// fleetDialColdRetryMax bounds the cold tier's jitter window.
const fleetDialColdRetryMax = 10 * time.Minute

// fleetLiveCheckInterval is how often an active serve session re-checks the registry
// so a `fleet rm` sends an authenticated revoked promptly, not on the next inbound
// frame (review B5b).
const fleetLiveCheckInterval = time.Second

// fleet journal directions.
const (
	fleetDirIn  = "in"
	fleetDirOut = "out"
	fleetDirAck = "ack" // an outbound cid the peer durably acked (WAL prune marker)
)

// fleetJournalEntry is the FROZEN durable journal shape (B4): a versioned,
// monotonically-sequenced record wrapping the COMPLETE validated wire frame
// (including from{}), replayable by L2/L3 verbatim.
type fleetJournalEntry struct {
	V     int             `json:"v"`
	Seq   uint64          `json:"seq"`
	Dir   string          `json:"dir"`
	At    string          `json:"at"`
	Frame json.RawMessage `json:"frame"`
}

// FleetStore is the fleet registry. It is stateless between calls — every
// operation loads fresh from disk under a filesystem lock — so a box process and
// a CLI process share one authoritative view.
type FleetStore struct {
	dir      string
	path     string
	lockPath string
}

// OpenFleetStore returns the registry handle under <stateDir>/fleet/. It does
// not read or write anything (the file is loaded fresh per operation).
func OpenFleetStore(stateDir string) (*FleetStore, error) {
	if stateDir == "" {
		return nil, fmt.Errorf("fleet store requires a state dir")
	}
	dir := filepath.Join(stateDir, fleetDirName)
	return &FleetStore{
		dir:      dir,
		path:     filepath.Join(dir, fleetStateFile),
		lockPath: filepath.Join(dir, fleetLockFile),
	}, nil
}

func fleetNow() string { return time.Now().UTC().Format(time.RFC3339Nano) }

// newEdgeID mints an 8-char immutable edge id (6 random bytes → base64url).
func newEdgeID() (string, error) { return randomBase64URL(6) }

// loadFleetState reads and structurally validates fleet.json. A missing file is
// an empty registry; a read or decode failure is an ERROR (never silently
// retained stale state — B1). A structurally invalid entry is an error so a
// corrupt registry is never mutated on top of.
func loadFleetState(path string) (fleetState, error) {
	st := fleetState{Edges: map[string]FleetEdge{}}
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return st, nil
		}
		return fleetState{}, err
	}
	if err := json.Unmarshal(data, &st); err != nil {
		return fleetState{}, fmt.Errorf("decode fleet state: %w", err)
	}
	if st.Edges == nil {
		st.Edges = map[string]FleetEdge{}
	}
	if err := validateFleetState(st); err != nil {
		return fleetState{}, err
	}
	return st, nil
}

// validateFleetState checks structural invariants on load (should-fix 3):
// map-key==edge_id, direction enum, envelope true, room/URL shape, and no
// duplicate active room or alias.
func validateFleetState(st fleetState) error {
	rooms := map[string]string{}
	aliases := map[string]string{}
	for key, e := range st.Edges {
		if e.EdgeID != key {
			return fmt.Errorf("fleet edge %q: edge_id %q != map key", key, e.EdgeID)
		}
		if !fleetEdgeIDRE.MatchString(e.EdgeID) {
			return fmt.Errorf("fleet edge %q: malformed edge_id", key)
		}
		if e.Direction != FleetServe && e.Direction != FleetDial {
			return fmt.Errorf("fleet edge %q: invalid direction %q", key, e.Direction)
		}
		if !e.Envelope {
			return fmt.Errorf("fleet edge %q: envelope must be true", key)
		}
		if !roomIDRE.MatchString(e.Room) {
			return fmt.Errorf("fleet edge %q: malformed room id", key)
		}
		if _, err := normalizeRendezvous(e.RelayURL); err != nil {
			return fmt.Errorf("fleet edge %q: bad relay_url: %w", key, err)
		}
		if e.Removed() {
			continue
		}
		if prev, dup := rooms[e.Room]; dup {
			return fmt.Errorf("fleet edges %q and %q share room %s", prev, key, e.Room)
		}
		rooms[e.Room] = key
		if e.Alias != "" {
			if prev, dup := aliases[e.Alias]; dup {
				return fmt.Errorf("fleet edges %q and %q share alias %q", prev, key, e.Alias)
			}
			aliases[e.Alias] = key
		}
	}
	return nil
}

func saveFleetState(path string, st fleetState) error {
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return atomicWrite0600(path, data)
}

// withLock runs fn holding a flock(LOCK_EX) on fleet.json.lock — the
// cross-process guard shared by the box and every CLI subcommand.
func (s *FleetStore) withLock(fn func() error) error {
	if err := os.MkdirAll(s.dir, 0o700); err != nil {
		return err
	}
	lf, err := os.OpenFile(s.lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return err
	}
	defer lf.Close()
	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX); err != nil {
		return fmt.Errorf("locking %s: %w", s.lockPath, err)
	}
	defer func() { _ = syscall.Flock(int(lf.Fd()), syscall.LOCK_UN) }()
	return fn()
}

// mutate is the flock-guarded read-modify-write: load fresh → fn → save. Every
// registry mutation goes through it so a concurrent process never loses a write.
func (s *FleetStore) mutate(fn func(st *fleetState) error) error {
	return s.withLock(func() error {
		st, err := loadFleetState(s.path)
		if err != nil {
			return err
		}
		if err := fn(&st); err != nil {
			return err
		}
		return saveFleetState(s.path, st)
	})
}

// Link mints a fleet room DIRECTLY into fleet.json (serve side, §2). It reuses
// the mintRoom recipe (22-char room id + 32-byte secret + e=1) but NEVER
// MintLinkMode, so relay-state.json is byte-untouched. It fails at the 16-edge
// cap and on an alias collision. The edge dir is STAGED before the registry is
// published (B6), so a failure can never leave an active ghost edge.
func (s *FleetStore) Link(base, alias string) (FleetEdge, string, error) {
	if err := validateAlias(alias); err != nil {
		return FleetEdge{}, "", err
	}
	alias = strings.TrimSpace(alias)
	r, secret, err := mintRoom(base, alias, true)
	if err != nil {
		return FleetEdge{}, "", err
	}
	normalized, err := normalizeRendezvous(r.URL)
	if err != nil {
		return FleetEdge{}, "", err
	}
	origin, err := rendezvousOrigin(normalized)
	if err != nil {
		return FleetEdge{}, "", err
	}
	var out FleetEdge
	err = s.mutate(func(st *fleetState) error {
		if err := s.checkCapAndAliasLocked(st, alias, ""); err != nil {
			return err
		}
		edgeID, err := uniqueEdgeID(st)
		if err != nil {
			return err
		}
		edge := FleetEdge{
			EdgeID: edgeID, Alias: alias, Direction: FleetServe,
			Room: r.ID, RelayURL: normalized, RelayOrigin: origin,
			Secret: secret, Envelope: true, AddedAt: fleetNow(),
		}
		// Stage the edge dir (journal.jsonl + state.json) BEFORE publishing the
		// registry entry, so a stage failure aborts with no ghost edge (B6).
		if err := s.stageEdgeDir(edge); err != nil {
			return err
		}
		st.Edges[edgeID] = edge
		out = edge
		return nil
	})
	if err != nil {
		return FleetEdge{}, "", err
	}
	return out, secret, nil
}

// JoinOptions carries the operator's relay-origin allowlist for a dial-side join
// (B7 SSRF gate). AllowedOrigins is the default set (the box's configured
// rendezvous); AllowRelay is an explicit operator override from `--allow-relay`.
type JoinOptions struct {
	AllowedOrigins []string
	AllowRelay     string
}

// Join parses a fleet pair URI (strict, §2), requires p=fleet + e=1, enforces
// the relay-origin allowlist (B7), generates a stable device id flt-<edgeID>,
// and persists the NORMALIZED creds in the edge dir with a registry entry
// direction=dial. It does NOT dial the peer — dialing is Lane L3.
func (s *FleetStore) Join(uri, alias string, opts JoinOptions) (FleetEdge, error) {
	if err := validateAlias(alias); err != nil {
		return FleetEdge{}, err
	}
	alias = strings.TrimSpace(alias)
	p, err := ParsePairURI(uri)
	if err != nil {
		return FleetEdge{}, err
	}
	if err := p.validateFleet(); err != nil {
		return FleetEdge{}, err
	}
	normalized, err := normalizeRendezvous(p.URL)
	if err != nil {
		return FleetEdge{}, err
	}
	origin, err := rendezvousOrigin(normalized)
	if err != nil {
		return FleetEdge{}, err
	}
	if err := allowRelayOrigin(origin, opts); err != nil {
		return FleetEdge{}, err
	}
	var out FleetEdge
	err = s.mutate(func(st *fleetState) error {
		if err := s.checkCapAndAliasLocked(st, alias, ""); err != nil {
			return err
		}
		edgeID, err := uniqueEdgeID(st)
		if err != nil {
			return err
		}
		creds := &DeviceCreds{
			DeviceID: "flt-" + edgeID, Room: p.Room, RelayURL: normalized,
			Secret: p.Secret, Envelope: true,
		}
		edge := FleetEdge{
			EdgeID: edgeID, Alias: alias, Direction: FleetDial,
			Room: p.Room, RelayURL: normalized, RelayOrigin: origin,
			DeviceCreds: creds, Envelope: true, AddedAt: fleetNow(),
		}
		if err := s.stageEdgeDir(edge); err != nil {
			return err
		}
		st.Edges[edgeID] = edge
		out = edge
		return nil
	})
	if err != nil {
		return FleetEdge{}, err
	}
	return out, nil
}

// Remove tombstones an edge (§2 rm), zeroing its creds while retaining the edge
// dir (journal history). Credential zeroing PATCHES only the secret fields — it
// never resets the cursor or rewrites state.json wholesale (B6). A state-patch
// failure is returned (visible/retriable), not swallowed.
func (s *FleetStore) Remove(arg string) (FleetEdge, error) {
	var out FleetEdge
	err := s.mutate(func(st *fleetState) error {
		id, err := resolveEdge(st, arg)
		if err != nil {
			return err
		}
		e := st.Edges[id]
		if e.Removed() {
			out = e
			return nil
		}
		e.Tombstone = &FleetTombstone{At: fleetNow(), Reason: "removed"}
		e.Secret = ""
		if e.DeviceCreds != nil {
			e.DeviceCreds.Secret = ""
		}
		// F1: a removal drops any orchestrator grant with the creds, so a re-paired
		// alias can never inherit authority from the edge it replaced.
		e.Authority = nil
		// Patch the retained state.json creds in place (never reset the cursor).
		if err := s.patchEdgeStateSecretLocked(id); err != nil {
			return fmt.Errorf("zeroing edge %s creds: %w", id, err)
		}
		st.Edges[id] = e
		out = e
		return nil
	})
	if err != nil {
		return FleetEdge{}, err
	}
	return out, nil
}

// Rename changes an edge's display alias (§1: alias renameable, edge_id
// immutable). It enforces the alias rules + active-uniqueness.
func (s *FleetStore) Rename(arg, alias string) (FleetEdge, error) {
	if err := validateAlias(alias); err != nil {
		return FleetEdge{}, err
	}
	alias = strings.TrimSpace(alias)
	var out FleetEdge
	err := s.mutate(func(st *fleetState) error {
		id, err := resolveEdge(st, arg)
		if err != nil {
			return err
		}
		if err := s.checkAliasLocked(st, alias, id); err != nil {
			return err
		}
		e := st.Edges[id]
		e.Alias = alias
		st.Edges[id] = e
		out = e
		return nil
	})
	if err != nil {
		return FleetEdge{}, err
	}
	return out, nil
}

// TouchLastSeen records a peer contact time (best-effort; used by the fleet
// handler). It is flock-guarded like every mutation so it cannot race a
// concurrent CLI rm and resurrect a tombstone (B1/B2): a tombstoned or missing
// edge is left untouched.
func (s *FleetStore) TouchLastSeen(edgeID string) {
	_ = s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			return nil
		}
		e.LastSeenAt = fleetNow()
		st.Edges[edgeID] = e
		return nil
	})
}

// Edges returns every registry entry (including tombstoned) sorted by edge_id.
func (s *FleetStore) Edges() ([]FleetEdge, error) {
	st, err := loadFleetState(s.path)
	if err != nil {
		return nil, err
	}
	out := make([]FleetEdge, 0, len(st.Edges))
	for _, e := range st.Edges {
		out = append(out, e)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].EdgeID < out[j].EdgeID })
	return out, nil
}

// LiveEdge loads ONE non-tombstoned edge fresh from disk. It is the authoritative
// per-frame check the fleet session uses (B2): a corrupt registry, a missing
// edge, or a tombstone all return ok=false (fail-closed → the session
// terminates).
func (s *FleetStore) LiveEdge(edgeID string) (FleetEdge, bool) {
	st, err := loadFleetState(s.path)
	if err != nil {
		return FleetEdge{}, false
	}
	e, ok := st.Edges[edgeID]
	if !ok || e.Removed() {
		return FleetEdge{}, false
	}
	return e, true
}

// ServedFleetRooms returns the serve-direction, non-tombstoned, STRUCTURALLY
// VALID edges the fleet room manager can serve (§3.1, should-fix 3), sorted by
// room id. An invalid registry surfaces as an error so the caller logs instead
// of silently serving a malformed edge.
func (s *FleetStore) ServedFleetRooms() ([]FleetEdge, error) {
	st, err := loadFleetState(s.path)
	if err != nil {
		return nil, err
	}
	var out []FleetEdge
	for id, e := range st.Edges {
		if e.Direction != FleetServe || e.Removed() {
			continue
		}
		if verr := validateServeEdge(id, e); verr != nil {
			return out, fmt.Errorf("skipping invalid serve edge %s: %w", id, verr)
		}
		out = append(out, e)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Room < out[j].Room })
	return out, nil
}

// roomRecordFor synthesizes the RoomRecord the connector/codec expects from a
// serve edge. Fleet rooms are always envelope rooms with a stored secret.
func (e FleetEdge) roomRecordFor() RoomRecord {
	return RoomRecord{ID: e.Room, URL: e.RelayURL, Name: e.Alias, Envelope: true, Secret: e.Secret}
}

// AppendJournalFrame appends one FROZEN journal entry wrapping a complete wire
// frame, fsyncing BEFORE returning so acceptance is durable (B4). The monotonic
// seq is derived from the journal tail under the flock. A write/sync failure is
// returned so the caller rejects the frame (never shrugged).
func (s *FleetStore) AppendJournalFrame(edgeID, dir string, frame json.RawMessage) (uint64, error) {
	var seq uint64
	err := s.withLock(func() error {
		var err error
		seq, err = s.appendJournalLocked(edgeID, dir, frame)
		return err
	})
	if err != nil {
		return 0, err
	}
	return seq, nil
}

// appendJournalLocked is the journal-append body without the flock — the caller
// must hold withLock. It lets a single flock transaction (EnqueueOutboundTx)
// revalidate + journal + queue atomically, since syscall.Flock is not reentrant
// across the fresh fd withLock opens each call. It fsyncs the frame BEFORE
// returning (B4) and, on first create, the containing directory (M7).
func (s *FleetStore) appendJournalLocked(edgeID, dir string, frame json.RawMessage) (uint64, error) {
	last, err := s.lastJournalSeqLocked(edgeID)
	if err != nil {
		return 0, err
	}
	return s.appendJournalAtLocked(edgeID, dir, frame, last)
}

// lastJournalSeqLocked returns the highest seq across BOTH journal generations
// (L4, §6), so a fresh live file after a rotation still continues the monotonic
// sequence from the .1 backup. Caller holds withLock.
func (s *FleetStore) lastJournalSeqLocked(edgeID string) (uint64, error) {
	lines, err := s.readJournalLines(edgeID)
	if err != nil {
		return 0, err
	}
	var max uint64
	for _, line := range lines {
		var e struct {
			Seq uint64 `json:"seq"`
		}
		if json.Unmarshal(line, &e) == nil && e.Seq > max {
			max = e.Seq
		}
	}
	return max, nil
}

// appendJournalAtLocked appends with a PRECOMPUTED previous seq (caller holds the
// lock and already scanned the journal), avoiding a redundant whole-file read on the
// hot inbound path (CommitInbound). It fsyncs the frame before returning (B4) and
// the containing dir on first create (M7). Before appending it rotates the live
// journal to .1 if it has reached fleetJournalRotateBytes (L4, §6) — seq stays
// monotonic across the rotation because the caller's precomputed prevSeq already
// spans both generations.
func (s *FleetStore) appendJournalAtLocked(edgeID, dir string, frame json.RawMessage, prevSeq uint64) (uint64, error) {
	edgeDir := filepath.Join(s.dir, edgeID)
	if err := os.MkdirAll(edgeDir, 0o700); err != nil {
		return 0, err
	}
	journalPath := filepath.Join(edgeDir, fleetJournalFile)
	if err := s.rotateJournalIfNeededLocked(edgeDir, journalPath); err != nil {
		return 0, err
	}
	_, statErr := os.Stat(journalPath)
	created := os.IsNotExist(statErr)
	seq := prevSeq + 1
	entry := fleetJournalEntry{V: fleetJournalVersion, Seq: seq, Dir: dir, At: fleetNow(), Frame: frame}
	line, err := json.Marshal(entry)
	if err != nil {
		return 0, err
	}
	f, err := os.OpenFile(journalPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return 0, err
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		f.Close()
		return 0, err
	}
	if err := f.Sync(); err != nil {
		f.Close()
		return 0, err
	}
	if err := f.Close(); err != nil {
		return 0, err
	}
	if created {
		fsyncDir(edgeDir)
	}
	return seq, nil
}

// rotateJournalIfNeededLocked rotates the live journal to journal.jsonl.1 when it
// has reached the rotation threshold (L4, §6). Caller holds withLock. A
// missing/small journal is a no-op.
//
// EXACTLY ONE archived generation is kept and it is NEVER overwritten: if a prior
// .1 already exists, the rotation is a no-op. This is the L4-sol BLOCKER-2 fix. The
// old code did os.Rename(journal, journal+".1") unconditionally, so a SECOND
// rotation clobbered the first .1 and silently destroyed that generation's WAL —
// old unacked outbound frames (lost from PendingOutbound recovery), inbound CIDs
// (allowing re-commit + reinjection), undelivered inbound records (lost from
// startup replay), and resume/late-ACK/cursor history. Refusing to clobber bounds
// the WAL to one archived generation and can never lose data. The live journal may
// then regrow unbounded past the threshold (acceptable this round; FOLLOW-UP:
// multi-generation compaction that ACK-prunes drained frames before rotating).
//
// The rename is followed by a directory fsync so the rotation survives a crash, and
// unlike the pre-blocker code that fsync's failure is PROPAGATED, not swallowed: the
// rename's durability is part of the rotation's correctness (a crash could otherwise
// resurrect the pre-rotation name or lose the .1 link), so a failed sync fails the
// append rather than leaving a half-durable rotation.
func (s *FleetStore) rotateJournalIfNeededLocked(edgeDir, journalPath string) error {
	fi, err := os.Stat(journalPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	threshold := int64(fleetJournalRotateBytes)
	if fleetJournalRotateBytesOverride > 0 {
		threshold = fleetJournalRotateBytesOverride // test seam only
	}
	if fi.Size() < threshold {
		return nil
	}
	rotated := journalPath + ".1"
	// Never clobber an existing archived generation (BLOCKER 2).
	if _, serr := os.Stat(rotated); serr == nil {
		return nil // a prior .1 is still present; skip rather than destroy its WAL
	} else if !os.IsNotExist(serr) {
		return serr
	}
	if err := os.Rename(journalPath, rotated); err != nil {
		return err
	}
	return fsyncDirErr(edgeDir)
}

// fleetJournalGenPaths returns an edge's journal generations OLDEST-FIRST (the .1
// backup, then the live file), so a reader concatenating them sees entries in
// append order across a rotation (L4, §6).
func (s *FleetStore) fleetJournalGenPaths(edgeID string) []string {
	base := filepath.Join(s.dir, edgeID, fleetJournalFile)
	return []string{base + ".1", base}
}

// readJournalLines reads every journal line across both generations (oldest-first),
// so inbound dedup, the outbound WAL, cursor reconciliation, and startup replay all
// see one rotation's worth of history (L4, §6). A missing generation is skipped.
func (s *FleetStore) readJournalLines(edgeID string) ([][]byte, error) {
	var out [][]byte
	for _, p := range s.fleetJournalGenPaths(edgeID) {
		data, err := os.ReadFile(p)
		if err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return nil, err
		}
		out = append(out, splitNonEmptyLines(data)...)
	}
	return out, nil
}

// EnqueueOutboundTx is the outbound path for fleet_send (review B4): under a SINGLE
// flock it revalidates the edge is still live (fail-closed against a concurrent rm —
// no queuing against a tombstone) and journals the frozen wire frame dir=out. The
// journal IS the outbound WAL — there is no second state write to strand a frame in,
// so a crash the instant after the journal fsync still recovers the frame (it is an
// unacked dir=out record, which PendingOutbound derives on the next attach). dropped
// reports whether the derived pending queue is now over the cap (oldest excluded
// from the drain set, logged by the caller).
func (s *FleetStore) EnqueueOutboundTx(edgeID, cid string, frame json.RawMessage) (seq uint64, dropped bool, err error) {
	err = s.withLock(func() error {
		st, lerr := loadFleetState(s.path)
		if lerr != nil {
			return lerr
		}
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			return fmt.Errorf("fleet edge %s is not live", edgeID)
		}
		sc, scerr := s.scanEdgeJournalLocked(edgeID)
		if scerr != nil {
			return scerr
		}
		sq, jerr := s.appendJournalAtLocked(edgeID, fleetDirOut, frame, sc.maxSeq)
		if jerr != nil {
			return jerr
		}
		seq = sq
		// Derived unacked count after this append (single scan; no re-read).
		unacked := 1
		for _, o := range sc.out {
			if !sc.acked[o.CID] {
				unacked++
			}
		}
		if unacked > fleetPendingCap {
			dropped = true
		}
		// L4 sent counter: bump this hour's outbound tally under the same flock. The
		// frame is ALREADY durably journaled (the WAL commit above) and will deliver on
		// the next attach/recovery; the counter is observability only. A failure here
		// must NOT fail the enqueue (SHOULD-FIX 3): returning an error would make
		// FleetSend report failure on a send that is durably pending, so the operator
		// retries and duplicates the logical send. Log the counter-write error and
		// return success once the frame is journaled.
		if est, eerr := s.loadEdgeStateLocked(edgeID); eerr != nil {
			fmt.Fprintf(os.Stderr, "hotline: fleet sent-counter load failed edge=%s (frame is durably enqueued, delivering): %v\n", edgeID, eerr)
		} else {
			est.bumpActivity(time.Now(), 1, 0, 0)
			if serr := s.saveEdgeStateLocked(edgeID, est); serr != nil {
				fmt.Fprintf(os.Stderr, "hotline: fleet sent-counter save failed edge=%s (frame is durably enqueued, delivering): %v\n", edgeID, serr)
			}
		}
		return nil
	})
	return seq, dropped, err
}

// ResolveEdge maps a CLI/tool arg (alias, edge_id, or unique edge_id prefix) to
// exactly one live edge, loading fresh under the lock. It is the fleet_send
// address resolver (§4). Zero/ambiguous/tombstoned all error.
func (s *FleetStore) ResolveEdge(arg string) (FleetEdge, error) {
	var out FleetEdge
	err := s.withLock(func() error {
		st, err := loadFleetState(s.path)
		if err != nil {
			return err
		}
		id, err := resolveEdge(&st, arg)
		if err != nil {
			return err
		}
		e := st.Edges[id]
		if e.Removed() {
			return fmt.Errorf("fleet edge %q was removed", arg)
		}
		out = e
		return nil
	})
	if err != nil {
		return FleetEdge{}, err
	}
	return out, nil
}

// loadEdgeStateLocked reads an edge's state.json (caller holds withLock). A
// missing file yields a zero state with cursor "0" (a freshly staged edge).
func (s *FleetStore) loadEdgeStateLocked(edgeID string) (fleetEdgeState, error) {
	path := filepath.Join(s.dir, edgeID, fleetEdgeStateFile)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return fleetEdgeState{Cursor: "0"}, nil
		}
		return fleetEdgeState{}, err
	}
	var st fleetEdgeState
	if err := json.Unmarshal(data, &st); err != nil {
		return fleetEdgeState{}, fmt.Errorf("decode edge state %s: %w", edgeID, err)
	}
	return st, nil
}

func (s *FleetStore) saveEdgeStateLocked(edgeID string, st fleetEdgeState) error {
	edgeDir := filepath.Join(s.dir, edgeID)
	if err := os.MkdirAll(edgeDir, 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	return atomicWrite0600(filepath.Join(edgeDir, fleetEdgeStateFile), append(data, '\n'))
}

// mutateEdgeState is the flock-guarded read-modify-write over one edge's
// state.json (the pending queue + ack watermarks + counters). Every L2 edge-state
// mutation goes through it so the box's serving path and a CLI reader never race.
func (s *FleetStore) mutateEdgeState(edgeID string, fn func(*fleetEdgeState) error) error {
	return s.withLock(func() error {
		st, err := s.loadEdgeStateLocked(edgeID)
		if err != nil {
			return err
		}
		if err := fn(&st); err != nil {
			return err
		}
		return s.saveEdgeStateLocked(edgeID, st)
	})
}

// EdgeState returns a copy of an edge's runtime state.json (introspection/tests).
func (s *FleetStore) EdgeState(edgeID string) (fleetEdgeState, error) {
	var out fleetEdgeState
	err := s.withLock(func() error {
		st, err := s.loadEdgeStateLocked(edgeID)
		if err != nil {
			return err
		}
		out = st
		return nil
	})
	return out, err
}

// PendingOutbound returns the live outbound queue (oldest-first) DERIVED from the
// durable journal (review B4): every dir=out frame whose cid has no dir=ack marker,
// in journal order, capped to the newest fleetPendingCap (oldest excluded from the
// drain set). The journal is the single source of truth — a crash between the
// outbound fsync and any other write can never strand a frame.
func (s *FleetStore) PendingOutbound(edgeID string) ([]fleetPendingOut, error) {
	var out []fleetPendingOut
	err := s.withLock(func() error {
		p, perr := s.pendingFromJournalLocked(edgeID)
		out = p
		return perr
	})
	return out, err
}

// PendingDepth returns the live unacked-outbound depth for an edge (L4 fleet ls /
// fleet_state), derived from the durable journal WAL — the accurate figure the
// legacy state.json Pending cache no longer holds.
func (s *FleetStore) PendingDepth(edgeID string) (int, error) {
	p, err := s.PendingOutbound(edgeID)
	return len(p), err
}

// EdgeActivity returns an edge's rolling 24h sent/recv/dropped fleet_msg counts
// (L4, §6), computed fresh from the persisted per-hour buckets.
func (s *FleetStore) EdgeActivity(edgeID string) (sent24h, recv24h, dropped24h uint64, err error) {
	st, e := s.EdgeState(edgeID)
	if e != nil {
		return 0, 0, 0, e
	}
	sent24h, recv24h, dropped24h = st.windowCounts(time.Now())
	return sent24h, recv24h, dropped24h, nil
}

// pendingFromJournalLocked derives the unacked outbound queue from the journal WAL
// (caller holds withLock). dir=out frames minus dir=ack markers, journal order,
// newest-fleetPendingCap.
func (s *FleetStore) pendingFromJournalLocked(edgeID string) ([]fleetPendingOut, error) {
	sc, err := s.scanEdgeJournalLocked(edgeID)
	if err != nil {
		return nil, err
	}
	var out []fleetPendingOut
	for _, o := range sc.out {
		if sc.acked[o.CID] {
			continue
		}
		out = append(out, o)
	}
	if len(out) > fleetPendingCap {
		out = out[len(out)-fleetPendingCap:]
	}
	return out, nil
}

// RecordPeerAckCID durably records that the peer acked an outbound cid (review B4):
// it appends a dir=ack WAL marker (fsync'd) so the drained state survives a crash —
// PendingOutbound never resurrects an acked frame. found reports whether an unacked
// dir=out record with that cid existed; remaining is the post-ack pending depth.
// Idempotent: a duplicate ack for an already-acked cid writes no second marker.
func (s *FleetStore) RecordPeerAckCID(edgeID, cid string) (found bool, remaining int, err error) {
	err = s.withLock(func() error {
		sc, serr := s.scanEdgeJournalLocked(edgeID)
		if serr != nil {
			return serr
		}
		if sc.acked[cid] {
			// already acked — recompute remaining, write nothing.
			for _, o := range sc.out {
				if !sc.acked[o.CID] {
					remaining++
				}
			}
			return nil
		}
		for _, o := range sc.out {
			if o.CID == cid {
				found = true
			}
		}
		if found {
			marker, _ := json.Marshal(map[string]any{"t": "fleet_ack", "cid": cid})
			if _, aerr := s.appendJournalLocked(edgeID, fleetDirAck, marker); aerr != nil {
				return aerr
			}
		}
		// remaining = unacked out entries after this ack.
		for _, o := range sc.out {
			if o.CID == cid || sc.acked[o.CID] {
				continue
			}
			remaining++
		}
		return nil
	})
	return found, remaining, err
}

// MarkInboundDelivered records that an inbound cid was injected to the agent
// (H2), so startup replay never re-injects it. Idempotent; FIFO-bounded.
func (s *FleetStore) MarkInboundDelivered(edgeID, cid string) error {
	return s.mutateEdgeState(edgeID, func(st *fleetEdgeState) error {
		for _, c := range st.InboundDelivered {
			if c == cid {
				return nil
			}
		}
		st.InboundDelivered = append(st.InboundDelivered, cid)
		if len(st.InboundDelivered) > fleetInboundDedupCap {
			st.InboundDelivered = st.InboundDelivered[len(st.InboundDelivered)-fleetInboundDedupCap:]
		}
		return nil
	})
}

// InboundDelivered reports whether an inbound cid was already injected.
func (s *FleetStore) InboundDelivered(edgeID, cid string) (bool, error) {
	st, err := s.EdgeState(edgeID)
	if err != nil {
		return false, err
	}
	for _, c := range st.InboundDelivered {
		if c == cid {
			return true, nil
		}
	}
	return false, nil
}

// IncDroppedInbound bumps the persisted inbound-drop counter (rate cap, §5).
func (s *FleetStore) IncDroppedInbound(edgeID string) error {
	return s.mutateEdgeState(edgeID, func(st *fleetEdgeState) error {
		st.DroppedInbound++
		// L4 dropped_24h: bump this hour's rate-drop tally in the same state write.
		st.bumpActivity(time.Now(), 0, 0, 1)
		return nil
	})
}

// SetPeerCaps stores the peer's box-attested capabilities manifest on the edge state
// (caps-design §3, latest-wins) with a local received_at. The manifest is already
// clamped by the caller (storePeerCaps). A tombstoned/missing edge simply writes its
// retained state dir — caps only ever arrive on a live session, so this is benign.
func (s *FleetStore) SetPeerCaps(edgeID string, c FleetCaps) error {
	return s.mutateEdgeState(edgeID, func(st *fleetEdgeState) error {
		st.PeerCaps = &FleetPeerCaps{Caps: c, ReceivedAt: fleetNow()}
		return nil
	})
}

// PeerCaps returns the stored peer manifest for an edge (caps-design §3), or nil when
// none has been received. Raw — NOT pin-checked; prefer BoundPeerCaps for any surface
// that renders under box-attested language.
func (s *FleetStore) PeerCaps(edgeID string) (*FleetPeerCaps, error) {
	st, err := s.EdgeState(edgeID)
	if err != nil {
		return nil, err
	}
	return st.PeerCaps, nil
}

// BoundPeerCaps is the ONE pin-aware accessor for the CLI + MCP surfaces (review B4): it
// returns the stored manifest ONLY when it is bound to the edge's pinned peer key AND the
// edge carries no persisted KeyFPMismatch (a mismatch overrides attestation until the
// edge is removed). Otherwise nil — so no surface ever renders an unbound or mismatched
// manifest under box-attested language. edge carries the authoritative pin (registry).
func (s *FleetStore) BoundPeerCaps(edge FleetEdge) (*FleetPeerCaps, error) {
	st, err := s.EdgeState(edge.EdgeID)
	if err != nil {
		return nil, err
	}
	if st.PeerCaps == nil || st.KeyFPMismatch {
		return nil, nil
	}
	if edge.PeerKeyFP == "" || st.PeerCaps.Caps.KeyFP == "" || st.PeerCaps.Caps.KeyFP != edge.PeerKeyFP {
		return nil, nil
	}
	return st.PeerCaps, nil
}

// FlagKeyFPMismatch persists the M5 identity-mismatch flag on the edge state.
func (s *FleetStore) FlagKeyFPMismatch(edgeID string) error {
	return s.mutateEdgeState(edgeID, func(st *fleetEdgeState) error {
		st.KeyFPMismatch = true
		return nil
	})
}

// PinPeerKeyFP is the M5 trust-on-first-use pin: it sets the edge's PeerKeyFP in
// the registry the FIRST time a peer presents one, and reports a mismatch when a
// later, DIFFERENT fp arrives (the stored pin is never overwritten). pinned=true
// means this call set the pin; mismatch=true means fp differed from the pin.
func (s *FleetStore) PinPeerKeyFP(edgeID, fp string) (pinned, mismatch bool, err error) {
	if fp == "" {
		return false, false, nil
	}
	err = s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			return nil
		}
		switch {
		case e.PeerKeyFP == "":
			e.PeerKeyFP = fp
			st.Edges[edgeID] = e
			pinned = true
		case e.PeerKeyFP != fp:
			mismatch = true
		}
		return nil
	})
	return pinned, mismatch, err
}

// journalScan is the parsed view of one edge's journal, computed in a single read
// (review B1/B3/B4): the durable source of truth for inbound dedup + cursor and for
// the outbound pending WAL.
type journalScan struct {
	maxSeq  uint64            // highest seq across ALL dirs (monotonic append base)
	inTail  uint64            // highest dir=in seq (the inbound cursor)
	inCIDs  map[string]bool   // every dir=in cid ever committed (complete dedup history)
	inOrder []string          // dir=in cids in journal order (for resume advertisement)
	out     []fleetPendingOut // dir=out records in journal order
	acked   map[string]bool   // dir=ack cid markers (outbound acked)
}

// scanEdgeJournalLocked reads and parses an edge's journal once (caller holds
// withLock), across both generations (L4 rotation, §6). A missing journal is an
// empty scan.
func (s *FleetStore) scanEdgeJournalLocked(edgeID string) (journalScan, error) {
	sc := journalScan{inCIDs: map[string]bool{}, acked: map[string]bool{}}
	lines, err := s.readJournalLines(edgeID)
	if err != nil {
		return journalScan{}, err
	}
	for _, line := range lines {
		var e fleetJournalEntry
		if json.Unmarshal(line, &e) != nil {
			continue
		}
		if e.Seq > sc.maxSeq {
			sc.maxSeq = e.Seq
		}
		var f struct {
			CID string `json:"cid"`
		}
		_ = json.Unmarshal(e.Frame, &f)
		switch e.Dir {
		case fleetDirIn:
			if e.Seq > sc.inTail {
				sc.inTail = e.Seq
			}
			if f.CID != "" {
				sc.inCIDs[f.CID] = true
				sc.inOrder = append(sc.inOrder, f.CID)
			}
		case fleetDirOut:
			sc.out = append(sc.out, fleetPendingOut{Seq: e.Seq, CID: f.CID, Frame: e.Frame, At: e.At})
		case fleetDirAck:
			if f.CID != "" {
				sc.acked[f.CID] = true
			}
		}
	}
	return sc, nil
}

// CommitInbound is the SINGLE durable inbound transaction (review B1/B3), shared by
// BOTH the serve handler and the dialer. Under ONE flock it: (1) revalidates the
// edge is live (a concurrent rm lands INSIDE the tx, never after), (2) checks the
// journal-derived cid index — the COMPLETE inbound history, not a bounded ring — so
// a redelivered or replayed cid is a durable no-op, (3) for a NEW cid, consults the
// rate cap, (4) appends+fsyncs the dir=in journal entry, and (5) advances the cursor
// in the SAME state write. The ACK is the caller's job and is sent ONLY when
// committed || dup (both mean "durably have it"); a not-live or error result sends
// NO ack, so the peer redelivers. rateAllow is invoked only for a genuinely new cid.
type inboundCommit struct {
	Seq         uint64
	Committed   bool // newly journaled this call
	Duplicate   bool // already in the durable journal (re-ack, do not re-journal)
	Live        bool // edge still live (not tombstoned/removed)
	RateDropped bool // new cid refused by the rate cap (no journal, no ack)
}

func (s *FleetStore) CommitInbound(edgeID, cid string, frame json.RawMessage, rateAllow func() bool) (inboundCommit, error) {
	var r inboundCommit
	err := s.withLock(func() error {
		st, lerr := loadFleetState(s.path)
		if lerr != nil {
			return lerr
		}
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			r.Live = false
			return nil
		}
		r.Live = true
		sc, serr := s.scanEdgeJournalLocked(edgeID)
		if serr != nil {
			return serr
		}
		if sc.inCIDs[cid] {
			r.Duplicate = true
			return nil
		}
		if rateAllow != nil && !rateAllow() {
			r.RateDropped = true
			return nil
		}
		seq, aerr := s.appendJournalAtLocked(edgeID, fleetDirIn, frame, sc.maxSeq)
		if aerr != nil {
			return aerr
		}
		r.Seq = seq
		r.Committed = true
		est, eerr := s.loadEdgeStateLocked(edgeID)
		if eerr != nil {
			return eerr
		}
		if seq > parseCursor(est.Cursor) {
			est.Cursor = fmt.Sprint(seq)
		}
		// L4 recv counter: a genuinely-new committed inbound bumps this hour's tally
		// in the SAME state write as the cursor advance (no extra flock, no dup on a
		// re-ack — RateDropped/Duplicate return before here).
		est.bumpActivity(time.Now(), 0, 1, 0)
		return s.saveEdgeStateLocked(edgeID, est)
	})
	return r, err
}

// ReconcileInboundCursor realigns the persisted Cursor with the durable journal's
// inbound tail at edge load (review B3 — handles divergence in EITHER direction).
// The journal is the dedup + cursor authority, so a torn state.json (cursor ahead OR
// behind the journal) is repaired by setting Cursor = inbound tail and bumping
// Generation. resynced reports whether a realignment occurred.
func (s *FleetStore) ReconcileInboundCursor(edgeID string) (generation uint64, resynced bool, err error) {
	err = s.withLock(func() error {
		sc, serr := s.scanEdgeJournalLocked(edgeID)
		if serr != nil {
			return serr
		}
		est, eerr := s.loadEdgeStateLocked(edgeID)
		if eerr != nil {
			return eerr
		}
		if parseCursor(est.Cursor) != sc.inTail {
			est.Cursor = fmt.Sprint(sc.inTail)
			est.Generation++
			resynced = true
			if werr := s.saveEdgeStateLocked(edgeID, est); werr != nil {
				return werr
			}
		}
		generation = est.Generation
		return nil
	})
	return generation, resynced, err
}

// InboundSeen reports whether an inbound cid is in the durable journal (test/support).
func (s *FleetStore) InboundSeen(edgeID, cid string) (bool, error) {
	var seen bool
	err := s.withLock(func() error {
		sc, serr := s.scanEdgeJournalLocked(edgeID)
		if serr != nil {
			return serr
		}
		seen = sc.inCIDs[cid]
		return nil
	})
	return seen, err
}

// RecentInboundCIDs returns the last n inbound-committed cids (journal order) for a
// fleet_resume advertisement (review B2). Bounded so the frame stays compact.
func (s *FleetStore) RecentInboundCIDs(edgeID string, n int) ([]string, error) {
	var out []string
	err := s.withLock(func() error {
		sc, serr := s.scanEdgeJournalLocked(edgeID)
		if serr != nil {
			return serr
		}
		out = ringTail(sc.inOrder, n)
		return nil
	})
	return out, err
}

// SetPeerBoxName records the peer's self-reported box display name at the INITIAL
// key pin only (review SF2). It is control-stripped and clamped; a tombstoned/
// missing edge or a name change on an already-pinned edge is a no-op.
func (s *FleetStore) SetPeerBoxName(edgeID, name string) error {
	name = clampIdent(strings.TrimSpace(stripFleetControl(name)))
	return s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() || name == "" || e.PeerBoxName != "" {
			return nil
		}
		e.PeerBoxName = name
		st.Edges[edgeID] = e
		return nil
	})
}

// MarkDialEdgeDead tombstones a dial edge the peer has revoked, whose room is gone,
// or whose creds are unrecoverable (review B5/SF3): the dialer stops dialing and the
// operator sees it dead in `fleet ls`. It mirrors Remove (tombstone + zero creds +
// patch state secret, cursor preserved) but records the specific reason. A no-op on
// an already-dead or missing edge. Local-only — the peer holds the room.
func (s *FleetStore) MarkDialEdgeDead(edgeID, reason string) error {
	return s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			return nil
		}
		e.Tombstone = &FleetTombstone{At: fleetNow(), Reason: reason}
		e.Secret = ""
		if e.DeviceCreds != nil {
			e.DeviceCreds.Secret = ""
		}
		// F1: drop any orchestrator grant with the creds (same rule as Remove).
		e.Authority = nil
		if err := s.patchEdgeStateSecretLocked(edgeID); err != nil {
			return fmt.Errorf("zeroing dead edge %s creds: %w", edgeID, err)
		}
		st.Edges[edgeID] = e
		return nil
	})
}

// MarkEdgeUnreachable records the F2 recoverable dead-mark on a live edge: the dialer
// has failed `attempts` consecutive handshakes on a known-good edge and has dropped to
// the cold-retry tier. It does NOT tombstone and does NOT zero creds — that is the
// whole point — so `unreachable` stays categorically distinct from the operator's
// `removed` and the peer's `revoked`. Re-marking an already-unreachable edge refreshes
// the attempt count and keeps the original Since.
func (s *FleetStore) MarkEdgeUnreachable(edgeID string, attempts int) error {
	return s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Removed() {
			return nil
		}
		now := fleetNow()
		if e.Unreachable == nil {
			e.Unreachable = &FleetUnreachable{Since: now}
		}
		e.Unreachable.Attempts = attempts
		e.Unreachable.LastAt = now
		st.Edges[edgeID] = e
		return nil
	})
}

// ClearEdgeUnreachable revives an edge on a successful handshake (F2). cleared reports
// whether a dead-mark was actually present, so the caller logs the revival once.
func (s *FleetStore) ClearEdgeUnreachable(edgeID string) (cleared bool, err error) {
	err = s.mutate(func(st *fleetState) error {
		e, ok := st.Edges[edgeID]
		if !ok || e.Unreachable == nil {
			return nil
		}
		e.Unreachable = nil
		st.Edges[edgeID] = e
		cleared = true
		return nil
	})
	return cleared, err
}

// DialEdges returns the dial-direction, non-tombstoned edges the dial manager
// drives (L3), sorted by edge id.
func (s *FleetStore) DialEdges() ([]FleetEdge, error) {
	st, err := loadFleetState(s.path)
	if err != nil {
		return nil, err
	}
	var out []FleetEdge
	for _, e := range st.Edges {
		if e.Direction != FleetDial || e.Removed() {
			continue
		}
		out = append(out, e)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].EdgeID < out[j].EdgeID })
	return out, nil
}

// parseCursor reads a decimal-string cursor, treating "" / non-numeric as 0.
func parseCursor(c string) uint64 {
	var n uint64
	if _, err := fmt.Sscan(c, &n); err != nil {
		return 0
	}
	return n
}

// ringTail returns the last cap elements of s (a fresh slice), or all of s.
func ringTail(s []string, cap int) []string {
	if len(s) > cap {
		s = s[len(s)-cap:]
	}
	return append([]string(nil), s...)
}

// JournalEntries returns the parsed journal entries for an edge (introspection +
// startup replay), across both rotation generations oldest-first (L4, §6).
func (s *FleetStore) JournalEntries(edgeID string) ([]fleetJournalEntry, error) {
	var lines [][]byte
	err := s.withLock(func() error {
		l, lerr := s.readJournalLines(edgeID)
		lines = l
		return lerr
	})
	if err != nil {
		return nil, err
	}
	var out []fleetJournalEntry
	for _, line := range lines {
		var e fleetJournalEntry
		if json.Unmarshal(line, &e) != nil {
			continue
		}
		out = append(out, e)
	}
	return out, nil
}

func splitNonEmptyLines(data []byte) [][]byte {
	var out [][]byte
	start := 0
	for i := 0; i < len(data); i++ {
		if data[i] == '\n' {
			if i > start {
				out = append(out, data[start:i])
			}
			start = i + 1
		}
	}
	if start < len(data) {
		out = append(out, data[start:])
	}
	return out
}

// checkCapAndAliasLocked enforces the edge cap + alias rules for a NEW edge.
func (s *FleetStore) checkCapAndAliasLocked(st *fleetState, alias, selfID string) error {
	if activeEdgeCount(st) >= fleetMaxEdges {
		return fmt.Errorf("fleet edge cap reached (%d/%d): remove an edge with `hotline fleet rm <peer>`", activeEdgeCount(st), fleetMaxEdges)
	}
	return s.checkAliasLocked(st, alias, selfID)
}

// checkAliasLocked rejects an alias that collides with an edge id or an existing
// active alias (should-fix 1).
func (s *FleetStore) checkAliasLocked(st *fleetState, alias, selfID string) error {
	if _, isID := st.Edges[alias]; isID {
		return fmt.Errorf("alias %q collides with an edge id", alias)
	}
	for id, e := range st.Edges {
		if id == selfID || e.Removed() {
			continue
		}
		if e.Alias == alias {
			return fmt.Errorf("alias %q is already in use by edge %s", alias, id)
		}
	}
	return nil
}

// activeEdgeCount counts non-tombstoned edges (the set the cap governs).
func activeEdgeCount(st *fleetState) int {
	n := 0
	for _, e := range st.Edges {
		if !e.Removed() {
			n++
		}
	}
	return n
}

// uniqueEdgeID mints an 8-char edge id not already present in the registry.
func uniqueEdgeID(st *fleetState) (string, error) {
	for attempt := 0; attempt < 8; attempt++ {
		id, err := newEdgeID()
		if err != nil {
			return "", err
		}
		if _, exists := st.Edges[id]; !exists {
			return id, nil
		}
	}
	return "", fmt.Errorf("could not mint a unique edge id")
}

// resolveEdge maps a CLI arg to exactly one edge_id: an exact/unique-prefix
// edge_id match, or an exact ACTIVE alias match. Zero or ambiguous is an error.
func resolveEdge(st *fleetState, arg string) (string, error) {
	if arg == "" {
		return "", fmt.Errorf("specify a peer alias or edge id")
	}
	if _, ok := st.Edges[arg]; ok {
		return arg, nil
	}
	var idMatches, aliasMatches []string
	for id, e := range st.Edges {
		if strings.HasPrefix(id, arg) {
			idMatches = append(idMatches, id)
		}
		if e.Alias == arg && !e.Removed() {
			aliasMatches = append(aliasMatches, id)
		}
	}
	switch {
	case len(idMatches)+len(aliasMatches) == 0:
		return "", fmt.Errorf("no fleet edge matching %q", arg)
	case len(aliasMatches) == 1 && len(idMatches) == 0:
		return aliasMatches[0], nil
	case len(idMatches) == 1 && len(aliasMatches) == 0:
		return idMatches[0], nil
	case len(idMatches) == 1 && len(aliasMatches) == 1 && idMatches[0] == aliasMatches[0]:
		return idMatches[0], nil
	default:
		return "", fmt.Errorf("%q is ambiguous: use a longer edge id", arg)
	}
}

// validateAlias enforces the alias charset/length rules (should-fix 1).
func validateAlias(alias string) error {
	a := strings.TrimSpace(alias)
	if a == "" {
		return fmt.Errorf("alias must not be empty")
	}
	if utf8.RuneCountInString(a) > fleetMaxAliasRunes {
		return fmt.Errorf("alias must be %d characters or fewer", fleetMaxAliasRunes)
	}
	for _, r := range a {
		if r < 0x20 || r == 0x7f {
			return fmt.Errorf("alias must not contain control characters")
		}
	}
	return nil
}

// validateServeEdge checks a serve edge is safe to serve (should-fix 3).
func validateServeEdge(id string, e FleetEdge) error {
	if e.EdgeID != id {
		return fmt.Errorf("edge_id mismatch")
	}
	if !fleetEdgeIDRE.MatchString(e.EdgeID) {
		return fmt.Errorf("malformed edge_id")
	}
	if e.Direction != FleetServe {
		return fmt.Errorf("not a serve edge")
	}
	if !e.Envelope {
		return fmt.Errorf("envelope must be true")
	}
	if !roomIDRE.MatchString(e.Room) {
		return fmt.Errorf("malformed room id")
	}
	if _, err := normalizeRendezvous(e.RelayURL); err != nil {
		return fmt.Errorf("bad relay_url: %w", err)
	}
	if e.Secret == "" {
		return fmt.Errorf("serve edge has no secret")
	}
	if _, err := decodePairSecret(e.Secret); err != nil {
		return fmt.Errorf("bad secret: %w", err)
	}
	return nil
}

// stageEdgeDir creates <dir>/<edgeID>/ with an empty journal.jsonl and a
// state.json (§1). Called BEFORE the registry entry is published (B6).
func (s *FleetStore) stageEdgeDir(e FleetEdge) error {
	edgeDir := filepath.Join(s.dir, e.EdgeID)
	if err := os.MkdirAll(edgeDir, 0o700); err != nil {
		return err
	}
	journal := filepath.Join(edgeDir, fleetJournalFile)
	if _, err := os.Stat(journal); os.IsNotExist(err) {
		f, err := os.OpenFile(journal, os.O_CREATE|os.O_WRONLY, 0o600)
		if err != nil {
			return err
		}
		_ = f.Close()
	}
	st := fleetEdgeState{DeviceCreds: e.DeviceCreds, Cursor: "0"}
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	return atomicWrite0600(filepath.Join(edgeDir, fleetEdgeStateFile), append(data, '\n'))
}

// patchEdgeStateSecretLocked zeroes ONLY the device_creds.secret in an edge's
// state.json, preserving the cursor and every other field (B6). A missing
// state.json is not an error (a serve edge has no dial creds to zero).
func (s *FleetStore) patchEdgeStateSecretLocked(edgeID string) error {
	path := filepath.Join(s.dir, edgeID, fleetEdgeStateFile)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	var st fleetEdgeState
	if err := json.Unmarshal(data, &st); err != nil {
		return err
	}
	if st.DeviceCreds != nil {
		st.DeviceCreds.Secret = ""
	}
	// L5: a removal zeroes the stored peer caps with the creds (caps-design §5).
	st.PeerCaps = nil
	out, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	return atomicWrite0600(path, append(out, '\n'))
}

// rendezvousOrigin extracts scheme://host from a normalized rendezvous URL.
func rendezvousOrigin(normalized string) (string, error) {
	u, err := parseStrictURL(normalized)
	if err != nil {
		return "", err
	}
	return u.Scheme + "://" + u.Host, nil
}

// allowRelayOrigin is the B7 SSRF-by-storage gate: a dial edge's relay origin
// must be one the operator approved (its configured rendezvous), unless an
// explicit --allow-relay override names it.
func allowRelayOrigin(origin string, opts JoinOptions) error {
	for _, a := range opts.AllowedOrigins {
		if a != "" && a == origin {
			return nil
		}
	}
	if strings.TrimSpace(opts.AllowRelay) != "" && opts.AllowRelay == origin {
		return nil
	}
	return fmt.Errorf("relay origin %s is not the box's configured rendezvous; re-run with `--allow-relay %s` to trust it explicitly", origin, origin)
}

// atomicWrite0600 writes data to path via a temp file + rename, mode 0600 —
// the same durability shape as RelayStore.saveLocked.
func atomicWrite0600(path string, data []byte) error {
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, ".fleet-*")
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
	if err := os.Rename(name, path); err != nil {
		return err
	}
	if err := os.Chmod(path, 0o600); err != nil {
		return err
	}
	// M7: fsync the containing directory so the rename (the durable-visibility
	// event) survives a crash, not just the file's data blocks.
	fsyncDir(dir)
	return nil
}

// fsyncDir best-effort fsyncs a directory so a create/rename inside it is durable.
// Swallows errors (a dir that cannot be opened for sync is not worth failing a
// write over, matching the store's other best-effort durability touches). Callers
// for whom the directory sync is correctness-critical (journal rotation) use
// fsyncDirErr instead.
func fsyncDir(dir string) {
	_ = fsyncDirErr(dir)
}

// fsyncDirErr fsyncs a directory and RETURNS any failure, so a correctness-critical
// caller (rotateJournalIfNeededLocked) can propagate it instead of silently leaving
// a half-durable rename (BLOCKER 2).
func fsyncDirErr(dir string) error {
	d, err := os.Open(dir)
	if err != nil {
		return err
	}
	if serr := d.Sync(); serr != nil {
		d.Close()
		return serr
	}
	return d.Close()
}
