package app

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/supervise"
	"github.com/1broseidon/hotline/internal/transcript"
)

type Server struct {
	cfg *config.Config
	log *transcript.Logger

	// botName is the push-notification title for the direct/self-host gateway
	// path (push.go). It is seeded from cfg.BotName at construction and refreshed
	// on a device rename (FB21 set_name); botNameMu guards the rename write
	// against the concurrent push-path read.
	botNameMu sync.RWMutex
	botName   string

	store   *RelayStore
	mailbox *localMailbox
	outbox  *outbox
	blobs   *blobRegistry
	cid     *cidDeduper
	jobs    *jobRegistry
	elIndex *elementIndex
	// elementProj is the FB92 (msgID, elID) → latest-folded-element projection the
	// box reads to synthesize a resolution edit for an inbound /el action. Folded on
	// every durable msg/edit emit (foldFrame) and seeded once at startup from the
	// journal. Guarded by its own mutex; never scanned under deliveryMu.
	elementProj *elementProjection
	initErr     error

	deliveryMu sync.Mutex
	sinkMu     sync.RWMutex
	sink       provider.InboundSink

	// typing gate + inbound coalescer (typing-signal design phase 1). typing
	// holds delivery while any device asserts it is typing; inbound buffers a
	// texting burst into ONE SendChannel. Both are in-memory only and always
	// non-nil; inbound.enabled gates whether handleDeviceSend routes through the
	// coalescer (off ⇒ legacy synchronous delivery).
	typing  *typingGate
	inbound *inboundCoalescer

	// agent_state emitter (SPEC §1.2): throttled snapshot broadcaster.
	asMu       sync.Mutex
	asTimer    *time.Timer
	asLastSent time.Time
	asLastSnap []byte
	asThrottle time.Duration
	asClosed   bool

	// fleet_state emitter (a2a-design-v2 §6, Lane L4): the operator-facing fleet
	// snapshot broadcaster, a coarser sibling of the agent_state emitter (30s floor).
	fsMu       sync.Mutex
	fsTimer    *time.Timer
	fsLastSent time.Time
	fsLastSnap []byte
	fsThrottle time.Duration
	fsClosed   bool
	// fleetSweepEvery overrides the F2 liveness-sweep period (fleetliveness.go). Zero
	// means the fleetLivenessSweep default; tests shorten it.
	fleetSweepEvery time.Duration
	// fleet_state SEPARATE sequence domain (L4 post-mortem fix A2): fleet_state frames
	// carry a seq from THIS counter, never s.outbox.reserveTransient() — so a fleet
	// event can never perturb the shared operator outbox cursor / durable frame
	// sequencing. Guarded so a snapshot and a broadcast cannot interleave seq/publish.
	fsSeqMu sync.Mutex
	fsSeq   uint64
	// fleetStateCapRefs is the capability gate (L4 post-mortem fix A1): fleet_state is
	// emitted ONLY to operator devices that advertised fleet-state support in their
	// hello. No shipped client advertises it, so in production fleet_state stops being
	// emitted entirely. Refcounted by live session so concurrent reconnects are safe.
	fleetStateCapMu   sync.Mutex
	fleetStateCapRefs map[string]int

	// fleet caps emitter (caps-design-2026-07-23, Lane L5): resends the box-attested
	// capabilities manifest to caps-sendable peers on a fingerprint change, debounced
	// ≥30s. Its OWN lock — it touches only fleet sessions + fleet state, never the
	// operator outbox/transient path.
	capsMu          sync.Mutex
	capsTimer       *time.Timer
	capsLastSent    time.Time
	capsFingerprint string
	capsClosed      bool
	// capsOut* is the in-memory OUTGOING manifest cache (B2): built ONLY by the refresh
	// goroutine / MergeAgentInfo (off the hot path), read with ZERO filesystem access by
	// the serve+dial handshakes (currentCapsWireCached). Never build from disk inline on
	// session establishment.
	capsOutMu   sync.RWMutex
	capsOutWire []byte
	capsOutFP   string
	// capsAttest is the in-memory INCOMING attestation cache (B2): per-edge {pin,
	// mismatch, stored-caps key_fp + received_at}, so the inbound preamble/attestation
	// path reads cached values under the per-edge delivery lock with ZERO filesystem
	// access — it never reloads fleet.json/state.json per message. Seeded at startup,
	// updated in-line by every pin / mismatch-flag / caps-store the box performs.
	capsAttestMu sync.Mutex
	capsAttest   map[string]*edgeCapsAttest

	// startedAt is this box process's start (caps-design §1): the uptime/started_at
	// anchor, sender-computed so clock skew never fakes uptime. Set once in NewServer.
	startedAt time.Time
	// appVersion is the box binary's version/commit/date, threaded in from package
	// main via NewProvider so the caps builder can stamp bin{} (caps-design §1).
	appVersion AppVersion

	// Box-side harness/model identity for wire metadata (welcome §3.2 +
	// agent_state §1.2). Seeded at construction from the run child's config;
	// refined live by the harness_info notification (claude-sdk resolves its
	// model only when the SDK session initializes).
	agentInfoMu sync.RWMutex
	agentInfo   AgentInfo

	// Box-side model CATALOG (model catalog amendment 2026-07-20): the
	// selectable model list the harness enumerated, mirrored to devices on the
	// transient agent_catalog frame. Its own lock, not agentInfoMu: identity
	// restamps are frequent and this is not, so sharing a mutex would put the
	// hot path behind the cold one for no benefit. Empty = no catalog; the app
	// then renders its curated fallback (every pre-amendment box).
	agentCatalogMu sync.RWMutex
	agentCatalog   AgentCatalog

	// SDK hot-model apply (amendment 2026-07-19): model-only set_sdk_config
	// requests forwarded to the harness and awaiting its sdk_apply_result,
	// keyed by rid. Own small mutex — no interaction with the existing lock
	// order. sdkApplyForward is bound pre-Run by the claude-sdk harness
	// wiring (cmd/hotline runInjectedHarness, same posture as the agentInfo
	// seed); nil means the hot path is unavailable and model-only requests
	// keep the restart path (fixture SC9). sdkPendingTimeout is a test seam;
	// 0 = sdkApplyPendingTimeout.
	// boxRoot is THIS box's state root — where its model/effort knobs live
	// (sol review #10). Empty means the default box, which resolves against the
	// shared base root exactly as it always did. Seeded at construction by the
	// caller, which is the layer that resolves the box.
	boxRoot string

	sdkMu             sync.Mutex
	sdkPending        map[string]*sdkPending
	sdkApplyForward   func(ctx context.Context, rid string, model, effort *string) error
	sdkPendingTimeout time.Duration
	// sdkSettled remembers the outcome of every rid this box already answered,
	// so a client that REPLAYS a control (every app build whose set_sdk_config
	// rode the never-settling pending outbox) is re-answered instead of
	// re-applied. Bounded by sdkSettledCap/sdkSettledTTL. Guarded by sdkMu.
	sdkSettled map[string]sdkSettled

	pushEndpoint    string
	pushBearer      string
	pushClient      *http.Client
	gatewayEndpoint string
	pushSigner      *pushSigner

	// liveActivitySender is a direct APNs ActivityKit transport. It is separate
	// from push.go/Expo and nil when the all-or-nothing APNs config is absent or
	// invalid. Its Enqueue method never performs network I/O synchronously.
	liveActivitySender liveActivitySender

	// Core mode (core-v1 SPEC §5): when coreMode is set the box registers rooms,
	// forwards device tokens, and sends wake hints through coreClient instead of
	// calling Expo directly. coreClient is nil (and every core branch is inert)
	// when HOTLINE_CORE_MODE is unset.
	coreMode   bool
	coreClient *coreClient
	regMu      sync.Mutex
	registered map[string]bool

	// wakeGate pre-throttles core-mode wake POSTs per device to the core's own
	// 6/min+10s contract, so a burst never burns the box's shared per-IP budget
	// on wakes the core would reject (round-2 review S-2).
	wakeGate *wakeGate

	// pushPreviewClear (HOTLINE_PUSH_PREVIEW=clear) opts the box owner into
	// readable push previews: the wake hint carries the message plaintext. Unset
	// ⇒ the wake hint is wire-identical to today's generic behavior.
	pushPreviewClear bool

	// unifiedChat (HOTLINE_UNIFIED_CHAT != 0, default on) stamps device_send
	// meta chat_id="app" so the harness sees ONE conversation across every
	// device (user/user_id stay the originating deviceID for provenance). Off
	// restores the legacy per-device chat_id.
	unifiedChat bool
	// readSync (HOTLINE_READ_SYNC != 0, default on) enables the read-state frame,
	// max-merge, fan, and post-drain snapshot. Off makes `read` inert (ignored
	// like any unknown type) and suppresses the snapshot.
	readSync bool

	// The read-acceptance watermark W is NO LONGER a monotone-max scalar (that
	// design could not represent interior holes: seq1 fans, seq2 is dropped by a
	// sibling, seq3 fans, and a CAS-max jumps the mark over the seq2 hole). It is now
	// computed live as W = MIN over active devices of each device's highest
	// CONTIGUOUS durable-delivered journal seq (mailboxRecord.CHead). A hole on any
	// device holds W below it even as higher seqs fan; W moves with the true min and
	// can stay put. See localMailbox.durableWatermark / recordDeliveredLocked.

	dial relayDialFunc

	onLinked               func(string)
	afterDeviceSendPersist func()
	// afterResyncSnapshot is a TEST-ONLY seam fired inside clearResyncFull after the
	// durable snapshot is captured and BEFORE holes are recorded / Full is cleared. It
	// lets a test reproduce the stale-snapshot schedule deterministically (drive a
	// concurrent emit that must be serialized behind deliveryMu). nil in production.
	afterResyncSnapshot func()

	// supervisorDir is HOTLINE_SUPERVISOR_DIR: the running `hotline up`
	// supervisor's control dir, set when the connector is a grandchild of the
	// supervisor. It comes from cfg.SupervisorDir (populated by config.LoadApp
	// at the CLI boundary), NEVER from the ambient env read here — reading it
	// here made a bare `go test ./internal/app/` file a real restart.request
	// into the live supervisor dir. Empty for an unsupervised `hotline run`,
	// and empty for the zero-value test Config. onRoomRotate is the
	// seam the connector calls when it observes the current room change out from
	// under it (a `relay new-link`, including one run as a separate CLI process
	// against the same state dir): the default asks the supervisor to recycle the
	// harness so a fresh harness+connector bind to the new room instead of the
	// running harness going deaf (soak bug 2026-07-14). Overridable in tests.
	supervisorDir string
	onRoomRotate  func(newRoomID string)

	// connLog persists connector-lifecycle events (relay /c dial/connect/
	// disconnect/backoff + WebSocket close codes) to <stateDir>/connector.log so a
	// device-bounce flap is diagnosable after the fact; the pre-existing os.Stderr
	// lines are unchanged. Nil-safe; see connlog.go.
	connLog *connLogger

	// fleetStore is the A2A (agent-to-agent) registry (a2a-design-v2 §1), a
	// transport disjoint from the operator relay path — it shares NO durable state
	// with store/mailbox/outbox. Nil when the state dir is empty (bare-Server
	// tests); the fleet room manager then no-ops. See fleetstore.go / fleetconn.go.
	fleetStore *FleetStore
	// fleetLog is the <stateDir>/fleet.log connLogger (§6): fleet session lifecycle
	// and per-frame metadata (never message text). Nil-safe; see fleetsession.go.
	fleetLog *connLogger

	// fleetSink is the Lane-L2 inbound-injection target for fleet_msg: a
	// source="fleet"-tagged InboundSink bound by FleetProvider.Start (the fleet
	// Provider registered in NewRouter). It is DISJOINT from s.sink (the app
	// channel) — a fleet turn never rides the app coalescer/mailbox (F10). Nil
	// until the fleet provider starts; a fleet_msg that arrives first is journaled
	// only (logged), never lost from the durable record.
	fleetSinkMu sync.RWMutex
	fleetSink   provider.InboundSink

	// fleetSessions maps an edge_id to its LIVE serve session, so fleet_send can
	// push over a connected peer (§4). The writer is the session's
	// envelope-sealing, mutex-guarded write func — safe to call from the tool
	// goroutine. Absent ⇒ the edge is offline and fleet_send queues to state.json.
	// A monotonic token disambiguates a reconnect that replaced the entry (func
	// values are not comparable, and their code pointers alias across sessions).
	fleetSessMu   sync.Mutex
	fleetSessions map[string]fleetLiveSession
	fleetSessSeq  uint64

	// fleetRate is the per-edge in-memory inbound rate limiter (§5, 60/5min). The
	// box is the sole serving process, so an in-memory sliding window is
	// authoritative; the DROP counter is persisted to edge state.json for the
	// operator. Keyed by edge_id. The window is IN-MEMORY only: a box restart
	// resets it (a peer cannot game this — a restart is not attacker-triggerable,
	// and the durable drop counter is preserved), which is accepted for L2.
	fleetRateMu sync.Mutex
	fleetRate   map[string]*fleetRateWindow

	// fleetEdgeLocks is the per-edge in-memory DELIVERY critical section (H4): one
	// mutex per edge_id serializing attach(register+drain), outbound enqueue+push,
	// and inbound inject+dedup so a message is never both live-pushed and drained,
	// and a live session's injection never races startup replay. Kept minimal —
	// the durable store flock still guards cross-process state; this only orders
	// the box's own concurrent goroutines per edge.
	fleetLockMu    sync.Mutex
	fleetEdgeLocks map[string]*sync.Mutex

	// fleetCapsRate is the per-edge in-memory fleet_caps accept floor (caps-design §2,
	// 1/30s): keyed by edge_id, holds the last-accepted time. In-memory only (a box
	// restart resets it — not attacker-triggerable).
	fleetCapsRateMu sync.Mutex
	fleetCapsRate   map[string]time.Time
}

// fleetRateWindow is a bounded sliding-window counter for one edge's inbound
// fleet_msg (§5). It holds at most fleetInboundRateN recent accept times.
type fleetRateWindow struct {
	times []time.Time
}

func NewServer(cfg *config.Config, log *transcript.Logger) *Server {
	s := &Server{cfg: cfg, log: log, botName: cfg.BotName, pushEndpoint: expoPushEndpoint, pushBearer: cfg.AppPushToken, gatewayEndpoint: cfg.AppPushEndpoint, pushClient: &http.Client{Timeout: pushTimeout}, dial: defaultRelayDial, supervisorDir: cfg.SupervisorDir, wakeGate: newWakeGate()}
	// Persistent connector-lifecycle log (connlog.go): the relay /c dial/connect/
	// disconnect/backoff events and close codes would otherwise vanish into the
	// supervised harness's tmux pane, leaving field flaps un-attributable.
	s.connLog = openConnLogger(cfg.StateDir)
	// Fleet (A2A) registry + log: a disjoint transport from the operator relay
	// path (a2a-design-v2 §1). A load failure disables fleet serving (logged)
	// rather than aborting the box, exactly like the optional push/core paths.
	s.fleetLog = openFleetLogger(cfg.StateDir)
	s.fleetSessions = map[string]fleetLiveSession{}
	s.fleetRate = map[string]*fleetRateWindow{}
	s.fleetEdgeLocks = map[string]*sync.Mutex{}
	s.fleetCapsRate = map[string]time.Time{}
	s.fleetStateCapRefs = map[string]int{}
	s.capsAttest = map[string]*edgeCapsAttest{}
	// Box process start (caps-design §1): the uptime anchor for the caps manifest.
	s.startedAt = time.Now()
	if cfg.StateDir != "" {
		if fs, ferr := OpenFleetStore(cfg.StateDir); ferr != nil {
			fmt.Fprintf(os.Stderr, "hotline: fleet registry load failed, fleet serving disabled: %v\n", ferr)
		} else {
			s.fleetStore = fs
			// B2: seed the in-memory INCOMING attestation cache at construction (a pure
			// state read — no key creation), so a restart with existing edges attests from
			// memory immediately. The OUTGOING manifest cache is primed by the refresh
			// goroutine in Run (it builds the box key, which must NOT be created at
			// construction on an inert box — TestCoreModeUnsetIsInert).
			s.seedCapsAttest()
		}
	}
	s.onRoomRotate = s.requestHarnessRecycle
	s.typing = newTypingGate()
	s.inbound = newInboundCoalescer(s.typing, s.flushInbound, cfg.InboundCoalesce)
	// HOTLINE_APP_COALESCE_WINDOW overrides BOTH the quiet window and the
	// complete-looking grace (deliberately unified); zero keeps the built-in
	// default. The HOTLINE_APP_COALESCE=0 kill switch is unaffected.
	if cfg.AppCoalesceWindow > 0 {
		s.inbound.window = cfg.AppCoalesceWindow
		s.inbound.grace = cfg.AppCoalesceWindow
	}
	var err error
	s.store, err = OpenRelayStore(cfg.StateDir)
	if err != nil {
		s.initErr = err
		return s
	}
	liveSender, liveErr := newLiveActivitySender(cfg, s.store)
	if liveErr != nil {
		fmt.Fprintf(os.Stderr, "hotline: live activities disabled: %v\n", liveErr)
	} else {
		s.liveActivitySender = liveSender
	}
	// Gateway mode (HOTLINE_PUSH_ENDPOINT set): construct the signer, loading or
	// generating the persistent P-256 key. A signer-init failure disables the
	// gateway push path (logged) rather than aborting the box; the legacy Expo
	// path stays available when the endpoint is unset.
	if s.gatewayEndpoint != "" {
		signer, serr := newPushSigner(filepath.Join(cfg.StateDir, pushSigningKeyFile), s.gatewayEndpoint, s.store, s.pushClient)
		if serr != nil {
			fmt.Fprintf(os.Stderr, "hotline: app push signer init failed, gateway push disabled: %v\n", serr)
		} else {
			s.pushSigner = signer
		}
	}
	// Core mode (HOTLINE_CORE_MODE=1): construct the signed core client, loading
	// or generating the box identity key (box-key.json, distinct from the push
	// signing key). A client-init failure disables the core path (logged) rather
	// than aborting the box. Unset ⇒ no core client, no core code path runs.
	s.coreMode = cfg.CoreMode
	s.pushPreviewClear = cfg.PushPreviewClear
	// Default-on kill switches: config stores the "off" sense so a zero-value
	// Config (the test idiom) keeps the shipped default-on behavior.
	s.unifiedChat = !cfg.UnifiedChatOff
	s.readSync = !cfg.ReadSyncOff
	s.registered = map[string]bool{}
	if s.coreMode {
		cc, cerr := newCoreClient(cfg.StateDir, cfg.CoreURL, s.pushClient)
		if cerr != nil {
			fmt.Fprintf(os.Stderr, "hotline: core client init failed, core mode disabled: %v\n", cerr)
		} else {
			s.coreClient = cc
		}
	}
	s.mailbox, err = newLocalMailbox(filepath.Join(cfg.StateDir, "mailboxes.json"))
	if err != nil {
		s.initErr = err
		return s
	}
	s.outbox = newPersistentOutbox(defaultOutboxCap, filepath.Join(cfg.StateDir, "outbox.jsonl"))
	// The read-acceptance watermark W is derived live from the per-device CHead
	// (persisted in mailboxes.json), so there is no scalar to seed at boot: a lagging
	// sibling's CHead is already below the missing seq, and W = min CHead reflects it
	// automatically. Only the persisted READ cursor still needs a boot clamp: keep it
	// no higher than the durable journal cursor so, if a prior read.j referenced a
	// seq that never durably persisted (degraded outbox persistence, or a crash that
	// left outbox.jsonl short), a future durable message can't inherit an
	// already-marked-read seq. Transients are never persisted to outbox.jsonl, so the
	// restored outbox cursor is exactly the highest durable seq.
	s.mailbox.clampReadTo(s.outbox.cursor())
	// Reconstruct crash/boot-gap holes so a durable seq that reached outbox.jsonl but
	// never fanned (or the whole undelivered range of an old CHead-less mailbox) can
	// never be leaped by a later fold (blocker #1). Boot-time, before any client.
	s.reconcileBootHoles()
	s.blobs = newBlobRegistry(cfg.StateDir)
	s.cid = newPersistentCIDDeduper(defaultCIDDedupCap, filepath.Join(cfg.StateDir, "cid-seen.jsonl"))
	s.jobs, err = newPersistentJobRegistry(jobRegistryStoragePath(cfg.StateDir))
	if err != nil {
		// Card persistence is best-effort like the outbox: normal app delivery
		// remains available even when its restart registry cannot be loaded.
		logJobRegistryFailure("load", err)
	}
	s.elIndex = newElementIndex()
	// FB92: seed the resolution projection from the full journal in one boot-time
	// pass (single-threaded, before any client, off the hot path). From here it is
	// kept live by foldFrame on every durable emit.
	s.elementProj = newElementProjection()
	s.elementProj.seed(s.outbox.framesAfter(0))
	s.asThrottle = agentStateThrottle
	s.fsThrottle = fleetStateThrottle
	// Anchor the fleet_state change-throttle at boot so the FIRST fleet event
	// schedules an async (timer-driven) broadcast rather than firing inline on the
	// fleet frame's call stack. This preserves the operator-isolation property (a
	// fleet_msg never synchronously advances the operator outbox transient seq —
	// fleet_state is operator awareness that trails on a ≥30s throttle), while
	// operators still get an immediate picture via the post-drain snapshot.
	s.fsLastSent = time.Now()
	s.mailbox.onPushIntent = s.maybePush
	s.mailbox.onCustomPushIntent = s.maybePushWithIntent
	// Only now are store, mailbox, outbox, registry, element identity, and the
	// optional APNs sender all ready to absorb the restart cancellation sweep.
	s.restoreJobCards()
	return s
}

// getBotName reads the current push-title identity under botNameMu.
func (s *Server) getBotName() string {
	s.botNameMu.RLock()
	defer s.botNameMu.RUnlock()
	return s.botName
}

// setBotName refreshes the push-title identity after a rename (FB21 set_name).
func (s *Server) setBotName(name string) {
	s.botNameMu.Lock()
	s.botName = name
	s.botNameMu.Unlock()
}

func (s *Server) bindSink(sink provider.InboundSink) {
	s.sinkMu.Lock()
	s.sink = sink
	s.sinkMu.Unlock()
}

func (s *Server) currentSink() provider.InboundSink {
	s.sinkMu.RLock()
	defer s.sinkMu.RUnlock()
	return s.sink
}

// requestHarnessRecycle is the default onRoomRotate: when the connector notices
// the current room rotated (a `relay new-link`), ask the supervisor to bounce
// the harness through the same restart.request control file the restart tool and
// SIGHUP use. This works even when new-link was run as a separate CLI process —
// the connector learns the new room from the on-disk relay state (CurrentRoom
// reloads it) and files the recycle here. The fresh harness (and the fresh
// connector it spawns) then bind cleanly to the new room instead of the running
// harness staying deaf on the old one. Inert (logged) when unsupervised.
func (s *Server) requestHarnessRecycle(newRoomID string) {
	if s.supervisorDir == "" {
		fmt.Fprintf(os.Stderr, "hotline: relay room rotated to %s but no supervisor to recycle the harness (unsupervised `hotline run`)\n", newRoomID)
		return
	}
	if err := supervise.RequestRestart(s.supervisorDir, "relay room rotated (new-link)"); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: relay room rotated to %s; requesting harness recycle failed: %v\n", newRoomID, err)
		return
	}
	fmt.Fprintf(os.Stderr, "hotline: relay room rotated to %s; requested harness recycle\n", newRoomID)
}

func (s *Server) Run(ctx context.Context, sink provider.InboundSink) error {
	if s.initErr != nil {
		return s.initErr
	}
	s.bindSink(sink)
	unobserve := s.registerStoreObservers()
	defer unobserve()
	defer s.stopAgentStateEmitter()
	defer s.stopFleetStateEmitter()
	defer s.stopFleetCapsEmitter()
	defer s.stopSDKPending()
	defer s.outbox.close()
	// Drain any burst caught mid-hold at shutdown, mirroring the telegram/discord/
	// signal coalescers. A flush with the sink already gone journals nothing new
	// (records were Appended at accept, un-MarkDelivered), so catch-up replays it.
	defer s.inbound.FlushAll(context.Background())
	// Serve fleet rooms alongside the operator rooms on a disjoint transport
	// (a2a-design-v2 §3.1). The two managers share only the Server (no durable
	// state), so fleet traffic can never reach an operator mailbox and vice versa.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	fleetDone := make(chan struct{})
	go func() {
		defer close(fleetDone)
		_ = s.runFleetRoomManager(ctx)
	}()
	// Dial side (Lane L3): drive every direction=dial edge's device-leg client on a
	// disjoint transport, picked up on the manager's next poll after `fleet join`.
	fleetDialDone := make(chan struct{})
	go func() {
		defer close(fleetDialDone)
		_ = s.runFleetDialManager(ctx)
	}()
	// Lane L5: refresh the box-attested caps manifest off the hot path and resend to
	// caps-sendable peers on a delta (caps-design §5). Disjoint from the operator path.
	fleetCapsDone := make(chan struct{})
	go func() {
		defer close(fleetCapsDone)
		s.runFleetCapsRefresh(ctx)
	}()
	// F2: watch for edges accumulating outbound into a void (a peer that wrote this
	// edge off while we keep queueing for it). Read-only; it alarms, never acts.
	fleetLivenessDone := make(chan struct{})
	go func() {
		defer close(fleetLivenessDone)
		s.runFleetLivenessSweep(ctx)
	}()
	err := s.runRoomManager(ctx)
	cancel()
	<-fleetDone
	<-fleetDialDone
	<-fleetCapsDone
	<-fleetLivenessDone
	return err
}

// unifiedChatID is the single stable chat_id the harness sees for the app channel
// when chat_id unification is on (§1.1). One conversation, many keyboards.
const unifiedChatID = "app"

func (s *Server) chatAllowed(chatID string) bool {
	// Unified mode: outbound turns carry chat_id="app" (never a device id), so
	// the gate resolves to "is any device active" — a reply is deliverable iff at
	// least one spoke can receive it (enqueueDurableLocked fans to all of them).
	if s.unifiedChat && chatID == unifiedChatID {
		return len(s.store.ActiveDevices()) > 0
	}
	d, ok := s.store.Device(chatID)
	return ok && d.State == DeviceActive
}

func (s *Server) emit(build func(seq uint64) []byte) uint64 {
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	return s.emitLocked(build)
}

// emitWithPushIntent attaches one internal notification intent to the same
// outbox append and terminal-frame fanout. The intent is not a wire field and is
// never recovered by backfill; each device's enqueue-time presence snapshot is
// the only push decision point.
func (s *Server) emitWithPushIntent(intent pushIntent, build func(seq uint64) []byte) uint64 {
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	return s.emitLockedWithPushIntent(build, &intent)
}

func (s *Server) emitLocked(build func(seq uint64) []byte) uint64 {
	return s.emitLockedWithPushIntent(build, nil)
}

func (s *Server) emitLockedWithPushIntent(build func(seq uint64) []byte, intent *pushIntent) uint64 {
	seq, data, createdAt := s.outbox.add(build)
	// FB92: fold every durable msg/edit into the resolution projection so its
	// latest-payload view tracks the outbox (a msg registers the message; an edit
	// merges into a known one; everything else is a no-op). Nil-guarded for
	// bare-Server unit tests that skip NewServer.
	if s.elementProj != nil {
		s.elementProj.foldFrame(data)
	}
	s.enqueueDurableWithPushIntentAtLocked(seq, data, createdAt, intent, false)
	return seq
}

func (s *Server) emitDeliveryLocked(key string, build func(seq uint64) []byte) uint64 {
	seq, data, createdAt := s.outbox.addDelivery(key, build)
	if s.afterDeviceSendPersist != nil {
		s.afterDeviceSendPersist()
	}
	s.enqueueDurableAtLocked(seq, data, createdAt)
	return seq
}

// DEPRECATED SHAPE — TEST-ONLY as of 395d46e. enqueueDurableLocked and
// enqueueDurableWithPushIntentLocked below have NO production callers: every
// live emit path goes through the *AtLocked variants, which carry the frame's
// original createdAt. These two stamp createdAt at enqueue time instead
// (stampIfMissing=true), so re-wiring either of them into an emit or replay path
// would silently re-date replayed frames — exactly the bug the *AtLocked split
// was made to fix. Do not "simplify" a call site back onto these; prefer
// enqueueDurableAtLocked / enqueueDurableWithPushIntentAtLocked. Kept only
// because readstate_test.go exercises the stamp-at-enqueue behaviour directly.
func (s *Server) enqueueDurableLocked(seq uint64, data []byte) {
	s.enqueueDurableWithPushIntentLocked(seq, data, nil)
}

func (s *Server) enqueueDurableAtLocked(seq uint64, data []byte, createdAt string) {
	s.enqueueDurableWithPushIntentAtLocked(seq, data, createdAt, nil, false)
}

func (s *Server) enqueueDurableWithPushIntentLocked(seq uint64, data []byte, intent *pushIntent) {
	s.enqueueDurableWithPushIntentAtLocked(seq, data, "", intent, true)
}

func (s *Server) enqueueDurableWithPushIntentAtLocked(seq uint64, data []byte, createdAt string, intent *pushIntent, stampIfMissing bool) {
	for _, d := range s.store.ActiveDevices() {
		deviceIntent := intent
		// FB44 preference selection belongs in this already-sorted fanout snapshot:
		// nil/true enables the successful-job intent, false suppresses it for this
		// device without changing the terminal edit delivered to its mailbox.
		if deviceIntent != nil && d.JobCompletionPush != nil && !*d.JobCompletionPush {
			deviceIntent = nil
		}
		var err error
		if stampIfMissing {
			_, err = s.mailbox.enqueueWithPushIntent(d.ID, journalString(seq), data, deviceIntent)
		} else {
			_, err = s.mailbox.enqueueWithPushIntentAt(d.ID, journalString(seq), data, createdAt, deviceIntent, false)
		}
		if err != nil {
			// Any failure (persistence error OR ErrMailboxFull ⇒ mailbox reset, the
			// device dropped item j) means this sibling does NOT hold seq. Record it as
			// a HOLE so the device's contiguous head (CHead) — and thus W = min CHead —
			// stays BELOW seq until the frame is actually backfilled (blocker #1). This
			// holds even if higher seqs later fan successfully: a monotone-max scalar
			// would have jumped over the hole, but CHead is frozen at the seq below the
			// hole by construction. A full mailbox is a normal, self-healing condition
			// (backfilled on the device's next attach), so only genuine errors log.
			if rerr := s.mailbox.recordHole(d.ID, seq); rerr != nil {
				fmt.Fprintf(os.Stderr, "hotline: app mailbox hole persist failed device=%s: %v\n", d.ID, rerr)
			}
			if !errorsIsMailboxFull(err) {
				fmt.Fprintf(os.Stderr, "hotline: app mailbox enqueue failed device=%s: %v\n", d.ID, err)
			}
		}
	}
	// No scalar high-water to advance: read-state acceptance reads W = min CHead
	// live. A successful enqueue above already folded seq into that device's CHead
	// (localMailbox.enqueue → recordDeliveredLocked); a failed one recorded a hole.
	// A reconciliation re-drive with an OLD seq is idempotent — recordDeliveredLocked
	// no-ops for a seq already contiguous, so W never regresses (SF-A) without any
	// explicit max.
}

func (s *Server) emitTransient(build func(seq uint64) []byte) uint64 {
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	seq := s.outbox.reserveTransient()
	data := build(seq)
	for _, d := range s.store.ActiveDevices() {
		s.mailbox.publishTransient(d.ID, data)
	}
	return seq
}

// emitTransientTo publishes a transient frame to a single device (used for the
// agent_state snapshot a freshly caught-up device receives). Like emitTransient
// it reserves a live-only seq and never touches the durable mailbox.
func (s *Server) emitTransientTo(deviceID string, build func(seq uint64) []byte) uint64 {
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	seq := s.outbox.reserveTransient()
	data := build(seq)
	s.mailbox.publishTransient(deviceID, data)
	return seq
}

// setRead applies an inbound read cursor (§4): it is a snapped DURABLE seq that
// acceptReadAtMost re-bounds to W (participating min-CHead) atomically before the
// max-merge, then — iff it advanced — fans the read transient to every active
// device (sender included, idempotent). Transient by construction: it reserves NO
// outbox seq (durable cursor churn would bloat every mailbox); an offline device
// converges via snapshotReadTo on its next attach instead.
func (s *Server) setRead(durable uint64) {
	advanced, err := s.mailbox.acceptReadAtMost(durable)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: read cursor persist failed: %v\n", err)
		return
	}
	if !advanced {
		return
	}
	data := readFrame(journalString(durable))
	for _, d := range s.store.ActiveDevices() {
		s.mailbox.publishTransient(d.ID, data)
	}
}

// snapshotReadTo hands a freshly caught-up device the current shared read cursor
// over the transient path (§4.1), alongside the agent_state snapshot. This is how
// an offline device converges without replaying intermediate cursor values —
// max-merge makes a single snapshot sufficient. No-op when read-sync is off or
// nothing has been read yet.
func (s *Server) snapshotReadTo(deviceID string) {
	if !s.readSync {
		return
	}
	j := s.mailbox.readCursor()
	if j == "" {
		return
	}
	s.mailbox.publishTransient(deviceID, readFrame(j))
}

func errorsIsMailboxFull(err error) bool { return err == ErrMailboxFull }

// durableWatermark is W: the highest durable journal seq every PARTICIPATING
// device (connected AND caught-up) has contiguously received (min CHead over
// participating devices). Read-state acceptance bounds read.j to this, so a read
// is never accepted for a seq a participating device is still missing (including
// one hidden behind an interior hole), while a disconnected/dead or still-
// reconciling device neither pins W low nor lets it leap.
func (s *Server) durableWatermark() uint64 {
	return s.mailbox.durableWatermark()
}

// durableHead is the highest DURABLE journal seq the box has ever issued — the
// authoritative "live durable head" against which a resync device's contiguous head
// is measured before it may rejoin W (WP-S1 fix6). It uses the ring-independent
// durable oracle (highestDurableSeqAtMost over the full issued range), so a durable
// frame emitted while a device was parked — which recorded no per-device hole — is
// still counted: markParticipating keeps such a device excluded until its CHead
// contiguously reaches this. 0 when the box holds no durable frame yet.
func (s *Server) durableHead() uint64 {
	head, ok := s.outbox.highestDurableSeqAtMost(s.outbox.cursor())
	if !ok {
		return 0
	}
	return head
}

// backfillDevice re-enqueues durable frames a device is missing above its
// contiguous head, so a reactivating/lagging device's recorded holes (and any
// reactivation gap where frames fanned while it was inactive) are actually filled
// and its CHead can advance — until then W stays below the hole. Idempotent:
// enqueue dedups frames the device already holds and no-ops deliveries already
// contiguous. Best-effort (a still-full mailbox re-records the hole and W simply
// waits). Runs on attach, off the deliveryMu path (only m.mu), so no lock
// inversion with enqueueDurableLocked.
// reconcileDevice is the unified boot/reconnect reconcile for a CONNECTING device:
// it reconstructs Holes for every durable seq the device is missing above its
// contiguous head (so a later fold can never leap a crash/reactivation gap), then
// backfills — re-delivering what it can. Anything still undeliverable stays a
// recorded Hole and holds W below it; the device does not count as caught up until
// it is filled. Runs off the deliveryMu path (mailbox m.mu only), so no lock
// inversion with enqueueDurableLocked.
func (s *Server) reconcileDevice(deviceID string) bool {
	chead := s.mailbox.contiguousHead(deviceID)
	var durable []uint64
	for _, jf := range s.outbox.framesAfter(chead) {
		if durableContent(jf.data) {
			durable = append(durable, jf.seq)
		}
	}
	s.mailbox.reconcileHoles(deviceID, durable)
	s.backfillDevice(deviceID)
	// Report whether the device is now caught up (drained, no outstanding holes) so a
	// resync device is only cleared back into W once it genuinely re-adopted and
	// caught up (BLOCKER 2). markParticipating re-checks this under the lock.
	return s.mailbox.caughtUp(deviceID)
}

// clearResyncFull atomically completes a resync device's gap-ack. It holds
// s.deliveryMu — the SAME lock every emit holds across outbox.add THEN
// enqueueDurableLocked — for the ENTIRE transition: capture the undelivered durable
// range from the outbox (framesAfter takes AND RELEASES the outbox lock), then hand
// it to reconcileHolesAndClearFull (which under m.mu records those holes and clears
// Full in one critical section). Holding deliveryMu across the whole thing closes the
// stale-snapshot race (WP-S1 fix8): because an emit cannot append+fan without
// deliveryMu, a concurrent emit of a frame above CHead is forced entirely BEFORE this
// snapshot (so the frame is in framesAfter → recorded as a Hole) or entirely AFTER
// Full is cleared and deliveryMu is released (so recordHole is NOT suppressed and the
// normal out-of-order path records the gap). No emit can append after the snapshot yet
// bounce-and-suppress under Full, which was the leap window.
//
// Lock order is preserved: deliveryMu → outbox → m.mu. framesAfter releases the outbox
// lock BEFORE reconcileHolesAndClearFull takes m.mu — the outbox lock is never held
// across m.mu — so this does not cycle with provision's m.mu→outbox path (provision
// never takes deliveryMu). The connector caller (handleHello, between gap-ack and
// reconcileDevice) runs off the deliveryMu path — device_send's deliveryMu hold is a
// separate, already-released critical section on the same goroutine — so re-acquiring
// this non-recursive mutex here does not self-deadlock.
func (s *Server) clearResyncFull(deviceID string) {
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	chead := s.mailbox.contiguousHead(deviceID)
	var durable []uint64
	for _, jf := range s.outbox.framesAfter(chead) {
		if durableContent(jf.data) {
			durable = append(durable, jf.seq)
		}
	}
	if s.afterResyncSnapshot != nil {
		s.afterResyncSnapshot()
	}
	s.mailbox.reconcileHolesAndClearFull(deviceID, durable)
}

// reconcileBootHoles reconstructs, for every persisted device, the Holes for
// durable seqs recorded in the append-only outbox.jsonl that the device never
// received (blocker #1: a crash between the outbox persist and the mailbox fan
// leaves a durable seq that would otherwise be silently leaped when a later seq
// folds; an old CHead-less mailbox reconstructs its whole undelivered range). This
// runs at boot, single-threaded, before any client connects. A device is anyway
// excluded from W until it reconnects and participates — this pass additionally
// prevents the fold-leap while it is offline (a fresh emit while disconnected must
// not fold past a gap). Delivery happens later, on the device's attach reconcile.
func (s *Server) reconcileBootHoles() {
	var durable []uint64
	for _, jf := range s.outbox.framesAfter(0) {
		if durableContent(jf.data) {
			durable = append(durable, jf.seq)
		}
	}
	if len(durable) == 0 {
		return
	}
	for _, id := range s.mailbox.deviceIDs() {
		s.mailbox.reconcileHoles(id, durable)
	}
}

func (s *Server) backfillDevice(deviceID string) {
	chead := s.mailbox.contiguousHead(deviceID)
	for _, jf := range s.outbox.framesAfter(chead) {
		if !durableContent(jf.data) {
			continue
		}
		if _, err := s.mailbox.enqueueAt(deviceID, journalString(jf.seq), jf.data, jf.createdAt); err != nil && !errorsIsMailboxFull(err) {
			fmt.Fprintf(os.Stderr, "hotline: app mailbox backfill failed device=%s seq=%d: %v\n", deviceID, jf.seq, err)
			return
		}
	}
}

func (s *Server) provisionMailbox(deviceID string) error {
	// The seed is a lazy loader: provision only reads/parses/copies the (untrimmed,
	// append-only) outbox journal when it actually creates a missing mailbox. A hot
	// reconnect for an already-resident mailbox (and repeated authenticated hellos
	// on one socket) short-circuits at the existence check and never touches disk —
	// keeping the core-mode-OFF path perf-identical to before B1.
	return s.mailbox.provision(deviceID, func() []journalFrame { return s.outbox.framesAfter(0) })
}

func validAttachmentName(name string) bool {
	name = strings.TrimSpace(name)
	return name != "" && utf8.RuneCountInString(name) <= 255 && name != "." && name != ".." &&
		!strings.ContainsAny(name, "/\\\x00\r\n") && filepath.Base(name) == name
}

func (s *Server) handleDeviceSend(ctx context.Context, deviceID string, frame deviceSendFrame) error {
	if len(frame.CID) < 16 || len(frame.CID) > 64 {
		return fmt.Errorf("device_send.cid must be 16..64 characters")
	}
	payloadType, ok := exactType(frame.Payload)
	if !ok {
		return fmt.Errorf("device_send.payload requires exact lowercase t")
	}
	key := deliveryKey(deviceID, frame.CID)
	s.deliveryMu.Lock()
	defer s.deliveryMu.Unlock()
	if s.cid.seen(key) {
		return nil
	}
	if seq, echo, createdAt, ok := s.outbox.delivery(key); ok {
		s.enqueueDurableAtLocked(seq, echo, createdAt)
		s.cid.add(key)
		return nil
	}

	var p struct {
		Text    string `json:"text"`
		Label   string `json:"label"`
		MsgID   string `json:"msg_id"`
		Emoji   string `json:"emoji"`
		ReplyTo string `json:"reply_to"`
		Xfer    string `json:"xfer"`
		Name    string `json:"name"`
		Mime    string `json:"mime"`
		Size    int64  `json:"size"`
	}
	if err := json.Unmarshal(frame.Payload, &p); err != nil {
		return fmt.Errorf("invalid device_send.payload")
	}
	content := p.Text
	harnessContent := content
	kind := "text"
	// chat_id unification (§1.1): the harness perceives ONE conversation across
	// every device (chat_id="app") while user/user_id keep the originating
	// deviceID for provenance. Gated: env-off restores the legacy per-device
	// chat_id=deviceID.
	chatID := deviceID
	if s.unifiedChat {
		chatID = unifiedChatID
	}
	meta := map[string]string{"chat_id": chatID, "user": deviceID, "user_id": deviceID, "ts": time.Now().UTC().Format(time.RFC3339)}
	var echoFile *fileRef
	switch payloadType {
	case "send":
		if strings.TrimSpace(content) == "" {
			return fmt.Errorf("send.text is required")
		}
		if p.ReplyTo != "" {
			meta["reply_to_message_id"] = p.ReplyTo
		}
		// Element-action bridge (§1.3): a recognized "/el {…}" send becomes a
		// structured element_action turn; any malformation falls open to the
		// plain message it already is. The raw serialization stays the canonical
		// echoed text; the harness gets the readable summary.
		if act, summary, value, ok := parseElementAction(content); ok {
			kind = "element_action"
			harnessContent = summary
			meta["kind"] = kind
			meta["element_msg"] = act.Msg
			meta["element_id"] = act.El
			meta["element_act"] = act.Act
			if value != "" {
				meta["element_value"] = value
			}
			// FB92: synthesize the confirming resolution edit BEFORE the message_id
			// prediction below (so the harness metadata still matches sent.id) and
			// BEFORE the keyed echo (so the echo fences it). deliveryMu is held; this
			// emits via emitLocked and is best-effort — it never fails the forward.
			s.synthesizeResolutionEditLocked(act)
		}
	case "tap":
		// harnessContent (what the bot receives) must also become the label —
		// it was seeded from p.Text above, which is empty for a tap, so without
		// this the bot + transcript got a BLANK button press while the app echo
		// (which uses `content`) showed the label correctly.
		content, harnessContent, kind = p.Label, p.Label, "button"
		meta["kind"] = kind
		if p.MsgID != "" {
			meta["reply_to_message_id"] = p.MsgID
		}
	case "react":
		// Same fix as tap: carry the emoji to the harness, not the empty p.Text.
		content, harnessContent, kind = p.Emoji, p.Emoji, "reaction"
		meta["kind"] = kind
		if p.MsgID != "" {
			meta["reply_to_message_id"] = p.MsgID
		}
	case "send.photo":
		rec, ok := s.blobs.resolve(p.Xfer)
		if !ok {
			return fmt.Errorf("send.photo references an incomplete blob")
		}
		if (p.Mime != "" && p.Mime != rec.Mime) || (p.Size != 0 && p.Size != rec.Size) {
			return fmt.Errorf("send.photo metadata does not match blob")
		}
		name := p.Name
		if !validAttachmentName(name) {
			name = "photo" + extensionForMime(rec.Mime)
		}
		content = nonBlank(content, "(photo)")
		meta["image_path"] = rec.Path
		echoFile = &fileRef{ID: rec.Xfer, Name: name, Mime: rec.Mime, Size: rec.Size, Xfer: rec.Xfer}
	case "send.attachment":
		rec, ok := s.blobs.resolve(p.Xfer)
		if !ok {
			return fmt.Errorf("send.attachment references an incomplete blob")
		}
		if !validAttachmentName(p.Name) || p.Mime == "" || p.Mime != rec.Mime || p.Size != rec.Size {
			return fmt.Errorf("send.attachment metadata does not match blob")
		}
		content = nonBlank(content, "(attachment)")
		kind = "attachment"
		meta["kind"] = kind
		meta["attachment_file_id"] = rec.Xfer
		meta["attachment_kind"] = "document"
		meta["attachment_name"] = p.Name
		echoFile = &fileRef{ID: rec.Xfer, Name: p.Name, Mime: rec.Mime, Size: rec.Size, Xfer: rec.Xfer}
	default:
		return fmt.Errorf("unsupported device_send payload type %q", payloadType)
	}
	if strings.TrimSpace(content) == "" {
		return fmt.Errorf("device_send payload is empty")
	}
	meta["message_id"] = fmt.Sprintf("u-%d", s.outbox.cursor()+1)
	// Delivery split (typing-signal design §3.4). When the coalescer is enabled the
	// accept path journals Append IMMEDIATELY (so a box crash mid-hold is healed by
	// catch-up) and defers delivery to the coalescer, which SendChannels the
	// coalesced burst and MarkDelivers each message at flush. The device echo below
	// still fires at accept, so the hold is invisible in the app. When disabled the
	// legacy synchronous path runs unchanged (SendChannel now, journal after).
	if s.inbound != nil && s.inbound.enabled {
		if s.log != nil {
			s.log.Append(transcript.Record{Dir: "in", ChatID: chatID, User: deviceID, UserID: deviceID, Kind: kind, MessageID: meta["message_id"], Text: harnessContent})
		}
	} else {
		sink := s.currentSink()
		if sink == nil {
			return fmt.Errorf("app inbound sink unavailable")
		}
		if err := sink.SendChannel(ctx, harnessContent, meta); err != nil {
			return err
		}
		if s.log != nil {
			s.log.Append(transcript.Record{Dir: "in", ChatID: chatID, User: deviceID, UserID: deviceID, Kind: kind, MessageID: meta["message_id"], Text: harnessContent})
			// SendChannel returned nil: the inbound reached a live harness. Journal the
			// delivered high-water so catch-up never replays this turn (B-1).
			s.log.MarkDelivered(meta["message_id"])
		}
	}
	if payloadType == "react" && p.MsgID != "" {
		s.emitDeliveryLocked(key, func(seq uint64) []byte { return reactFrame(seq, p.MsgID, p.Emoji, deviceID) })
	} else {
		s.emitDeliveryLocked(key, func(seq uint64) []byte {
			return sentFrame(seq, fmt.Sprintf("u-%d", seq), content, frame.CID, deviceID, kind, echoFile)
		})
	}
	s.cid.add(key)
	// Enqueue AFTER the accept is fully committed (echo emitted, CID recorded).
	// enqueue only buffers + arms a timer; the flush (SendChannel) runs off this
	// goroutine, so it never executes under deliveryMu.
	if s.inbound != nil && s.inbound.enabled {
		s.inbound.enqueue(ctx, harnessContent, meta)
	}
	return nil
}

// flushInbound is the coalescer's delivery callback: SendChannel the coalesced
// burst, then MarkDelivered each buffered message on success. It runs off
// deliveryMu (a timer, poke, or shutdown drain). A nil/failed sink logs and
// drops — the messages were Appended at accept, so catch-up replays them on the
// next harness start (strictly better than today's post-accept bad_frame).
func (s *Server) flushInbound(ctx context.Context, msgs []pendingMsg) {
	if len(msgs) == 0 {
		return
	}
	content, meta := coalesceApp(msgs)
	sink := s.currentSink()
	if sink == nil {
		fmt.Fprintf(os.Stderr, "hotline: app inbound sink unavailable, %d message(s) deferred to catch-up\n", len(msgs))
		return
	}
	if err := sink.SendChannel(ctx, content, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: app deliver inbound failed: %v\n", err)
		return
	}
	for _, m := range msgs {
		s.log.MarkDelivered(m.meta["message_id"])
	}
}

type journalFrame struct {
	seq       uint64
	data      []byte
	createdAt string
}

func (o *outbox) framesAfter(after uint64) []journalFrame {
	o.mu.Lock()
	defer o.mu.Unlock()
	var out []journalFrame
	if o.path == "" {
		for _, e := range o.buf {
			if e.seq > after {
				out = append(out, journalFrame{seq: e.seq, data: append([]byte(nil), e.data...), createdAt: e.createdAt})
			}
		}
		return out
	}
	data, err := os.ReadFile(o.path)
	if err != nil {
		return out
	}
	for _, line := range bytes.Split(data, []byte("\n")) {
		var p persistLine
		if json.Unmarshal(line, &p) == nil && p.Seq > after {
			out = append(out, journalFrame{seq: p.Seq, data: append([]byte(nil), p.Frame...), createdAt: p.CreatedAt})
		}
	}
	return out
}
