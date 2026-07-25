package app

// SDK-settings amendment 2026-07-19 (protocol/v2/SPEC.md, fixture
// sdk-config.json): the app sets the claude-sdk model/effort knobs remotely.
// The request rides the nested text-payload control family (set_name /
// set_push_preview / set_job_completion_push) as `set_sdk_config`; the box
// validates with the welcome-metadata caps, persists into the shared
// state-dir .env (config.UpdateSDKEnv — the mergedEnv source the supervisor's
// respawn re-resolution reads), answers the requesting device on the
// transient `sdk_config_result` frame, then bounces the harness through the
// existing supervisor restart control. Apply confirmation is the ordinary
// post-restart welcome/agent_state identity — no confirm frame exists.
//
// SDK hot-model amendment 2026-07-19 (same day, fixture SC6-SC9): a
// model-only request on a box whose harness wiring bound the hot forwarder
// skips the bounce entirely — the box forwards a sdk_apply notification to
// the harness, which applies Query.setModel to the LIVE session; the answer
// (sdk_apply_result) drives persistence (persist-after-ok) and the deferred
// ok+restart:false+hot:true result. Confirmation is the next agent_state
// snapshot carrying the restamped model — no welcome, the wire never drops.
//
// SDK hot-effort amendment 2026-07-19: effort hot-applies exactly like model.
// The harness applies it live via applyFlagSettings({effortLevel}) (symbolic)
// or setMaxThinkingTokens (raw budget) — the same mapping the boot Options use.
// The hot gate is now "forwarder bound && (model or effort present)"; the
// restart path remains ONLY the forwarder-unbound fallback (older binaries).

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"regexp"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/supervise"
)

// sdkApplyPendingTimeout bounds how long a forwarded hot apply may wait for
// the harness's sdk_apply_result before the box answers harness_unreachable
// (SDK hot-model amendment 2026-07-19). The client's hot budget is 10 s from
// the result receipt, so this must resolve first.
const sdkApplyPendingTimeout = 10 * time.Second

// sdkPending is one forwarded hot apply awaiting its sdk_apply_result. model
// and effort carry the SAME absent-vs-clear pointer semantics as the request
// (nil = field omitted, pointer-to-"" = explicit clear) so persist-after-ok
// writes exactly the fields the app changed.
type sdkPending struct {
	deviceID string
	model    *string
	effort   *string
	// persist is the harness's .env writer, captured at forward time so the
	// deferred result can never persist a pi apply into the claude-sdk env
	// family (or vice versa).
	persist func(model, effort *string) error
	timer   *time.Timer
}

// sdkSettled is the remembered outcome of a rid that already ran to completion.
//
// REPLAY DEFENCE (SDK control-rail fix). A set_sdk_config rides a device_send,
// and every shipped app before the control-rail fix put that device_send in its
// pending outbox — where it can never settle (we answer on the transient
// sdk_config_result, never with a mailbox `sent` echo) and is therefore
// re-flushed on EVERY reconnect. Such an app re-sends an hour-old model change
// each time its socket blips.
//
// The box cannot fix those clients, so it refuses to be fooled by them: a rid
// that already produced a result is answered from this cache, byte-identical,
// WITHOUT touching the live session or .env again. Replay becomes idempotent
// instead of a silent re-apply. It also closes the narrower same-session race
// where two devices mint the same rid.
type sdkSettled struct {
	frame func(seq uint64) []byte
	at    time.Time
}

// sdkSettledCap bounds the completed-rid cache. Knob changes are operator-paced
// (a handful per session), so this is generous; the oldest entry is evicted
// once it is full, and sdkSettledTTL expires entries a long-lived box would
// otherwise hold forever.
const (
	sdkSettledCap = 64
	sdkSettledTTL = 30 * time.Minute
)

// sdkRidRe bounds the request id used for result correlation (SC2): the app
// sends its makeCid() ULID, but any 1-64 chars of [A-Za-z0-9_-] route.
var sdkRidRe = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

// sdkModelRe is the box-authoritative model cap (SC2 = welcome-metadata WM2):
// ≤ 64 chars, alphanumeric lead, then a conservative single-line/.env/env-var
// safe charset. The class includes the square brackets that a real
// ModelInfo.value carries as its context-window suffix ("claude-opus-4-8[1m]"):
// the model catalog (catalog.ts) surfaces those values verbatim as selectable
// rows, so a tapped catalog id sends the bracketed string back here — refusing
// it would reject the box's own advertised menu before the harness's matchModel
// ever sees it. Brackets are .env/env-var safe (no quoting metacharacter).
var sdkModelRe = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:\[\]-]{0,63}$`)

// piModelRe is the same cap widened by "/" for the pi harness (pi hot-apply
// amendment 2026-07-20). Pi's model space is provider-scoped and its resolver
// canonicalizes to "provider/id" ("openai-codex/gpt-5.6-sol"), so the slash is
// part of a well-formed id there. Kept as a SEPARATE pattern rather than
// widening sdkModelRe so the claude-sdk acceptance surface stays byte-identical
// — no claude model id contains a slash, and a box should refuse what its own
// harness could never resolve. Both stay .env/env-var safe (single line, no
// quoting metacharacters).
var piModelRe = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:/-]{0,63}$`)

// harnessKnobs is the per-harness half of the model/effort control: what a
// valid value looks like, where the effective (boot) values are read from, and
// where a confirmed change is persisted. Everything else — the rid gate, the
// hot forwarding, persist-after-ok, the result frames — is harness-agnostic.
type harnessKnobs struct {
	modelRe     *regexp.Regexp
	modelHelp   string
	validEffort func(string) bool
	effortHelp  string
	// live reads the harness's configured model/effort from its own env family
	// (the values `up` re-exported at spawn). Only the no-op check uses it.
	live func() (model, effort string, err error)
	// persist writes a CONFIRMED change into the state-dir .env, with the same
	// nil-vs-pointer-to-"" semantics as the request.
	persist func(model, effort *string) error
}

// knobsFor maps a harness identity onto its knob family, scoped to THIS box's
// state root. A harness absent from this table takes no remote model/effort at
// all (SC4's unsupported_harness).
//
// boxRoot is what makes these settings genuinely per-box (sol review #10):
// reads and writes both resolve against <boxRoot>/.env under the shared base
// .env, so a named box's model change can no longer retarget every other box
// on the machine. The default box's root IS the base, so it is unchanged.
func knobsFor(harness, boxRoot string) (harnessKnobs, bool) {
	switch harness {
	case "claude-sdk":
		return harnessKnobs{
			modelRe:     sdkModelRe,
			modelHelp:   "model must be 1-64 chars: alphanumeric lead, then [A-Za-z0-9._:[]-]",
			validEffort: config.ValidSDKEffort,
			effortHelp:  "effort must be low|medium|high|xhigh|max or a positive integer (16 chars max)",
			live: func() (string, string, error) {
				cfg, err := config.LoadSDKForBox(boxRoot)
				return cfg.Model, cfg.Effort, err
			},
			persist: func(model, effort *string) error {
				return config.UpdateSDKEnvForBox(boxRoot, model, effort)
			},
		}, true
	case "pi":
		return harnessKnobs{
			modelRe:     piModelRe,
			modelHelp:   "model must be 1-64 chars: alphanumeric lead, then [A-Za-z0-9._:/-] (a pi id or provider/id)",
			validEffort: config.ValidPiThinking,
			effortHelp:  "effort must be one of off|minimal|low|medium|high|xhigh|max (pi takes a thinking level, not a token budget)",
			live: func() (string, string, error) {
				knob, err := config.LoadPiModelForBox(boxRoot)
				return knob.Model, knob.Thinking, err
			},
			persist: func(model, thinking *string) error {
				return config.UpdatePiEnvForBox(boxRoot, model, thinking)
			},
		}, true
	default:
		return harnessKnobs{}, false
	}
}

// sdkEffortMaxLen mirrors the app's AGENT_EFFORT_MAX_LEN (WM2): the symbolic
// names are ≤ 5 chars and a >16-digit maxThinkingTokens is nonsense.
const sdkEffortMaxLen = 16

// sdkConfigRequest is a parsed set_sdk_config control. Pointer = the field
// was present on the wire; a pointer to "" is an explicit clear-to-default.
// Model arrives trimmed and Effort lowercased+trimmed (the accepted post-trim
// values the result echoes).
type sdkConfigRequest struct {
	RID    string
	Model  *string
	Effort *string
}

// setSDKConfigFromDeviceSend recognizes a set_sdk_config control ride: a
// device_send whose "send" text payload is the serialized
// `{"t":"set_sdk_config","rid":…,"model":…,"effort":…}` line (same
// text-payload mechanism as set_name, since the app cannot emit a top-level
// websocket frame). isControl=true means the text IS a set_sdk_config object
// — the caller consumes it silently REGARDLESS of field validity, so a
// malformed ride never leaks raw JSON to the harness. Anything else returns
// isControl=false and flows on as a normal send.
func setSDKConfigFromDeviceSend(raw []byte) (req sdkConfigRequest, isControl bool) {
	var f deviceSendFrame
	if json.Unmarshal(raw, &f) != nil {
		return sdkConfigRequest{}, false
	}
	pt, ok := exactType(f.Payload)
	if !ok || pt != "send" {
		return sdkConfigRequest{}, false
	}
	var p struct {
		Text string `json:"text"`
	}
	if json.Unmarshal(f.Payload, &p) != nil {
		return sdkConfigRequest{}, false
	}
	return parseSetSDKConfig(p.Text)
}

// parseSetSDKConfig reports whether text is a set_sdk_config control object
// and returns its normalized request. isControl=true even when the fields are
// malformed (the probe matched, so the frame must be consumed); a request the
// caller cannot route (bad rid) simply carries the zero RID.
func parseSetSDKConfig(text string) (req sdkConfigRequest, isControl bool) {
	t := strings.TrimSpace(text)
	if !strings.HasPrefix(t, "{") {
		return sdkConfigRequest{}, false
	}
	var probe struct {
		T string `json:"t"`
	}
	if json.Unmarshal([]byte(t), &probe) != nil || probe.T != "set_sdk_config" {
		return sdkConfigRequest{}, false
	}
	// It IS a set_sdk_config control: consume silently from here on.
	var m struct {
		RID    string  `json:"rid"`
		Model  *string `json:"model"`
		Effort *string `json:"effort"`
	}
	if json.Unmarshal([]byte(t), &m) != nil {
		return sdkConfigRequest{}, true
	}
	req = sdkConfigRequest{RID: m.RID, Model: m.Model, Effort: m.Effort}
	if req.Model != nil {
		v := strings.TrimSpace(*req.Model)
		req.Model = &v
	}
	if req.Effort != nil {
		v := strings.ToLower(strings.TrimSpace(*req.Effort))
		req.Effort = &v
	}
	return req, true
}

// validateSDKConfig applies the SC2 rules to a normalized request under the
// requesting box's harness knobs. An empty code means valid. The effort rule
// delegates to the harness's own acceptance test (config.ValidSDKEffort /
// config.ValidPiThinking — the exact LoadSDK / Pi setThinkingLevel rules) so a
// value we accept here can never fail the harness's re-resolution.
func validateSDKConfig(req sdkConfigRequest, knobs harnessKnobs) (code, detail string) {
	if req.Model == nil && req.Effort == nil {
		return "empty_request", "set_sdk_config carried neither model nor effort"
	}
	if req.Model != nil && *req.Model != "" && !knobs.modelRe.MatchString(*req.Model) {
		return "invalid_model", knobs.modelHelp
	}
	if req.Effort != nil && *req.Effort != "" {
		if len(*req.Effort) > sdkEffortMaxLen || !knobs.validEffort(*req.Effort) {
			return "invalid_effort", knobs.effortHelp
		}
	}
	return "", ""
}

// handleSetSDKConfig runs a validated-or-refused set_sdk_config control to
// completion: refusal/validation errors answer immediately; a request on a
// hot-capable box (model and/or effort) routes to handleSetSDKConfigHot
// (deferred result); any other accepted change persists via config.UpdateSDKEnv,
// requests a supervisor restart, and answers ok+restart:true — the
// supervisor's 2 s poll guarantees the frame flushes long before the bounce
// drops the wire. All results ride emitTransientTo to the REQUESTING device
// only (SC3). Runs in the ws-handler before handleDeviceSend, holding no
// locks; emitTransientTo is the same primitive snapshotAgentStateTo already
// uses from handler context — no new lock ordering.
func (s *Server) handleSetSDKConfig(deviceID string, req sdkConfigRequest) {
	if !sdkRidRe.MatchString(req.RID) {
		// No way to correlate a result; drop silently (the buggy client times
		// out — this never happens to the shipped app, which sends a ULID).
		return
	}
	// Replay gate, BEFORE any validation or side effect: a rid we already
	// answered is re-answered from the cache, never re-applied. This is what
	// makes an outbox-replaying client (every app build before the control-rail
	// fix) harmless instead of a silent re-apply of an old model change.
	if replay, ok := s.settledSDKResult(req.RID); ok {
		s.emitTransientTo(deviceID, replay)
		return
	}
	fail := func(code, detail string) {
		s.answerSDKConfig(deviceID, req.RID, func(seq uint64) []byte {
			return sdkConfigResultFrame(seq, req.RID, false, nil, nil, false, false, false, code, detail)
		})
	}

	// SC4: only harnesses with a knob family take remote model/effort —
	// claude-sdk, and (pi hot-apply amendment 2026-07-20) pi. The identity is
	// seeded authoritatively from harnessMode in the run child (main.go); an
	// unknown/empty harness refuses too — the honest default.
	harness := s.currentAgentInfo().Harness
	knobs, supported := knobsFor(harness, s.boxRoot)
	if !supported {
		switch harness {
		case "claude":
			fail("unsupported_harness", "this box runs the claude TUI; model/effort can't be set remotely")
		case "":
			fail("unsupported_harness", "this box reports no harness identity; model/effort can only be set on claude-sdk or pi boxes")
		default:
			fail("unsupported_harness", fmt.Sprintf("this box runs harness %s; model/effort can only be set on claude-sdk or pi boxes", oneLineDetail(harness)))
		}
		return
	}

	if code, detail := validateSDKConfig(req, knobs); code != "" {
		fail(code, detail)
		return
	}

	// Hot path (SDK hot-model amendment + effort hot amendment 2026-07-19): any
	// request on a box whose harness wiring bound the hot forwarder applies
	// model AND/OR effort to the LIVE SDK session — no restart (operator
	// directive: restarting for a model/effort change is a no go). The restart
	// path below is now ONLY the fallback for a forwarder-unbound box
	// (telegram-only wiring, older binaries that never bound the forwarder —
	// fixture SC9); such a box keeps the verbatim persist→bounce→resume flow.
	if s.sdkApplyForward != nil && (req.Model != nil || req.Effort != nil) {
		s.handleSetSDKConfigHot(deviceID, req, knobs, fail)
		return
	}

	// No-op check against the EFFECTIVE running config: in the run child the
	// real env carries the values `up` re-exported at spawn, so the harness's
	// own loader here reflects what it is actually running with — exactly what
	// a no-op should compare against. A load error (junk .env) skips the check
	// and lets the persist below overwrite the junk.
	if liveModel, liveEffort, err := knobs.live(); err == nil {
		if (req.Model == nil || *req.Model == liveModel) && (req.Effort == nil || *req.Effort == liveEffort) {
			s.answerSDKConfig(deviceID, req.RID, func(seq uint64) []byte {
				return sdkConfigResultFrame(seq, req.RID, true, req.Model, req.Effort, false, false, false, "", "")
			})
			return
		}
	}

	if err := knobs.persist(req.Model, req.Effort); err != nil {
		fail("persist_failed", oneLineDetail(err.Error()))
		return
	}

	// Persisted. From here every outcome must say so: an unreachable
	// supervisor downgrades to restart_failed (saved; a manual restart
	// applies it), never a silent success.
	if s.supervisorDir == "" {
		fail("restart_failed", "settings saved; restart the box to apply")
		return
	}
	if err := supervise.RequestRestart(s.supervisorDir, fmt.Sprintf("sdk config change from app (rid %s)", req.RID)); err != nil {
		fail("restart_failed", "settings saved; restart the box to apply")
		return
	}
	s.answerSDKConfig(deviceID, req.RID, func(seq uint64) []byte {
		return sdkConfigResultFrame(seq, req.RID, true, req.Model, req.Effort, true, false, false, "", "")
	})
}

// handleSetSDKConfigHot runs the model/effort hot path (fixture SC6-SC8, effort
// hot amendment): no-op check against the LIVE identity, register a pending
// apply, forward model+effort to the harness over the sdk_apply notification,
// and defer the result to handleSDKApplyResult or the pending timer. Nothing
// touches .env here — persistence follows the harness's confirmation
// (persist-after-ok, SC7), so a bogus value can never enter .env via this path
// and a respawn can never revert a confirmed apply. Failures answer with an
// explicit error, never a silent restart fallback (the directive bans
// restarting for a model/effort change; every failure leaves a consistent
// state to retry from).
func (s *Server) handleSetSDKConfigHot(deviceID string, req sdkConfigRequest, knobs harnessKnobs, fail func(code, detail string)) {
	// No-op check against the LIVE identity, not LoadSDK: after a hot apply the
	// run child's real env still carries the values `up` exported at spawn, so
	// LoadSDK (real-env-wins) reports the BOOT config — a flip-back
	// (sonnet→opus after opus→sonnet, or high→xhigh→high) would be mis-answered
	// as a no-op and never applied. AgentInfo is restamped by harness_info after
	// every hot apply and is the live truth; the harness's own env loader is only
	// the fallback when a field was never reported.
	//
	// This holds identically for a pi box: the pi extension stamps harness_info
	// with the canonical "provider/id" model and the LIVE thinking level on every
	// child-ready, every hot apply, and every TUI model/thinking change, so
	// AgentInfo is populated and the LoadPiModel fallback rarely fires.
	info := s.currentAgentInfo()
	liveModel, liveEffort := info.Model, info.Effort
	// The config fallback fires ONLY for a field the harness has never reported.
	// A field the harness reported as CLEARED is empty on purpose, and falling
	// back would resurrect the boot value from the real env — which is exactly
	// the bug that made a re-select of a just-cleared model answer "already
	// effective" and never apply (hot-clear amendment).
	if (!info.ModelKnown && liveModel == "") || (!info.EffortKnown && liveEffort == "") {
		if cfgModel, cfgEffort, err := knobs.live(); err == nil {
			if !info.ModelKnown && liveModel == "" {
				liveModel = cfgModel
			}
			if !info.EffortKnown && liveEffort == "" {
				liveEffort = cfgEffort
			}
		}
	}
	if sdkModelNoop(req.Model, liveModel) && sdkEffortNoop(req.Effort, liveEffort) {
		s.answerSDKConfig(deviceID, req.RID, func(seq uint64) []byte {
			return sdkConfigResultFrame(seq, req.RID, true, req.Model, req.Effort, false, false, false, "", "")
		})
		return
	}

	// Register the pending apply BEFORE forwarding — the harness's result
	// notification races the forward call's return — and register it
	// atomically, under one critical section that also enforces the two
	// invariants a rid-keyed map cannot enforce on its own:
	//
	//   1. NO DUPLICATE RID. The old code let a second request with the same
	//      rid REPLACE the first: the first device was silently orphaned and
	//      the harness's answer — computed for request A — was applied and
	//      persisted for request B. A duplicate is now dropped; the original
	//      entry still owns the rid and still answers it.
	//   2. ONE APPLY AT A TIME. Two overlapping applies race in .env
	//      (concurrent read-modify-write can drop either model or effort) and
	//      in the live session. The rows disable while an apply is in flight,
	//      so this only ever fires for a second DEVICE, and it fires with a
	//      code that says exactly what happened rather than corrupting state.
	//
	// The timer is armed AFTER insertion, inside the same lock. Arming it first
	// (as before) allowed the expiry to run takeSDKPending against a map the
	// entry had not been added to yet — an apply that could never be answered.
	timeout := s.sdkPendingTimeout
	if timeout <= 0 {
		timeout = sdkApplyPendingTimeout
	}
	rid := req.RID
	pend := &sdkPending{deviceID: deviceID, model: req.Model, effort: req.Effort, persist: knobs.persist}
	s.sdkMu.Lock()
	if s.sdkPending == nil {
		s.sdkPending = make(map[string]*sdkPending)
	}
	if _, duplicate := s.sdkPending[rid]; duplicate {
		s.sdkMu.Unlock()
		// The in-flight original owns this rid and will answer the device.
		// Answering twice (or superseding) is what let result A settle request B.
		fmt.Fprintf(os.Stderr, "hotline: duplicate in-flight sdk apply rid %q dropped\n", oneLineDetail(rid))
		return
	}
	if len(s.sdkPending) > 0 {
		s.sdkMu.Unlock()
		fail("apply_in_progress", "another model/effort change is still being applied; try again in a moment")
		return
	}
	s.sdkPending[rid] = pend
	pend.timer = time.AfterFunc(timeout, func() { s.expireSDKPending(rid) })
	s.sdkMu.Unlock()

	if err := s.sdkApplyForward(context.Background(), rid, req.Model, req.Effort); err != nil {
		s.takeSDKPending(rid)
		fmt.Fprintf(os.Stderr, "hotline: sdk hot apply forward failed (rid %s): %v\n", rid, err)
		fail("harness_unreachable", "couldn't reach the agent; try again")
		return
	}
	// Deferred: handleSDKApplyResult or the timer answers the device.
}

// sdkModelNoop reports whether a model request is already effective against the
// live identity. nil = the field was omitted (unchanged, so a no-op). A clear
// ("") is a no-op only when nothing is configured (live ""); otherwise the same
// tolerance as the client's identityMatches: exact, or the live resolved id
// refining the requested alias (an alias resolving to the dated/tagged full id).
func sdkModelNoop(req *string, live string) bool {
	if req == nil {
		return true
	}
	m := *req
	if m == "" {
		return live == ""
	}
	return live == m || idExtendsAtBoundary(live, m)
}

// modelIDDelimiters are the characters that end a model-id component. Mirrors
// the harness's MODEL_ID_DELIMITERS (harness/claude-sdk/src/sdkapply.ts) and
// the app's (sdkSettings.ts); all three must agree, or the three layers
// disagree about what counts as the same model.
var modelIDDelimiters = map[byte]bool{
	'-': true, '_': true, '.': true, ':': true,
	'[': true, '(': true, '@': true, '/': true,
}

// idExtendsAtBoundary reports whether candidate extends requested at a
// component boundary — the alias tolerance that lets a bare "claude-opus-4-8"
// count as the live "claude-opus-4-8[1m]".
//
// Bare strings.HasPrefix was too generous for a NO-OP check specifically: a
// request of "c" against a live "claude-sonnet-4-6" was answered "already
// effective", so the apply never happened and the app waited for an identity
// that would never change. Requiring a delimiter keeps the alias cases and
// drops the rest.
func idExtendsAtBoundary(candidate, requested string) bool {
	if len(candidate) <= len(requested) || !strings.HasPrefix(candidate, requested) {
		return false
	}
	return modelIDDelimiters[candidate[len(requested)]]
}

// sdkEffortNoop mirrors sdkModelNoop for the effort knob. Effort has no
// dated/aliased variants, so the match is a case-insensitive equality (the live
// value may be un-normalized from the wire); a clear ("") is a no-op only when
// nothing is configured.
func sdkEffortNoop(req *string, live string) bool {
	if req == nil {
		return true
	}
	e := *req
	if e == "" {
		return live == ""
	}
	return strings.EqualFold(live, e)
}

// answerSDKConfig emits one terminal sdk_config_result to the requesting device
// AND remembers it under rid. Every path that answers a set_sdk_config goes
// through here, so a replay of that rid can be served from the cache instead of
// re-running the operation. build is a closure over the already-decided payload,
// so the replay differs from the original only in `seq` (which is per-emit by
// definition).
func (s *Server) answerSDKConfig(deviceID, rid string, build func(seq uint64) []byte) {
	s.rememberSDKResult(rid, build)
	s.emitTransientTo(deviceID, build)
}

// rememberSDKResult records a terminal outcome, evicting the oldest entry when
// the cache is full so a long-lived box's memory stays bounded.
func (s *Server) rememberSDKResult(rid string, build func(seq uint64) []byte) {
	now := time.Now()
	s.sdkMu.Lock()
	defer s.sdkMu.Unlock()
	if s.sdkSettled == nil {
		s.sdkSettled = make(map[string]sdkSettled, sdkSettledCap)
	}
	for k, v := range s.sdkSettled {
		if now.Sub(v.at) > sdkSettledTTL {
			delete(s.sdkSettled, k)
		}
	}
	for len(s.sdkSettled) >= sdkSettledCap {
		oldestKey, oldestAt := "", now
		for k, v := range s.sdkSettled {
			if oldestKey == "" || v.at.Before(oldestAt) {
				oldestKey, oldestAt = k, v.at
			}
		}
		delete(s.sdkSettled, oldestKey)
	}
	s.sdkSettled[rid] = sdkSettled{frame: build, at: now}
}

// settledSDKResult reports the remembered outcome for rid, if this box already
// answered it and the entry has not aged out.
func (s *Server) settledSDKResult(rid string) (func(seq uint64) []byte, bool) {
	s.sdkMu.Lock()
	defer s.sdkMu.Unlock()
	entry, ok := s.sdkSettled[rid]
	if !ok {
		return nil, false
	}
	if time.Since(entry.at) > sdkSettledTTL {
		delete(s.sdkSettled, rid)
		return nil, false
	}
	return entry.frame, true
}

// takeSDKPending removes and returns the pending apply for rid, stopping its
// timer. The single consume point — result handler, timer expiry, and the
// forward-failure unwind all race through here, so exactly one side answers.
func (s *Server) takeSDKPending(rid string) (*sdkPending, bool) {
	s.sdkMu.Lock()
	defer s.sdkMu.Unlock()
	p, ok := s.sdkPending[rid]
	if ok {
		delete(s.sdkPending, rid)
		p.timer.Stop()
	}
	return p, ok
}

// expireSDKPending answers a forwarded apply the harness never confirmed
// (timer callback). A late sdk_apply_result after this finds no pending entry
// and is dropped — late-ok never persists, keeping the SC7 invariant that
// .env only moves on an answered confirmation.
func (s *Server) expireSDKPending(rid string) {
	p, ok := s.takeSDKPending(rid)
	if !ok {
		return
	}
	s.answerSDKConfig(p.deviceID, rid, func(seq uint64) []byte {
		return sdkConfigResultFrame(seq, rid, false, nil, nil, false, false, false, "harness_unreachable", "the agent didn't confirm the change")
	})
}

// handleSDKApplyResult resolves a pending hot apply with the harness's answer
// (dispatched from the transport's interception goroutine via
// Provider.SDKApplyResultSink). Persist-after-ok is the point: .env is
// written only now, after the SDK confirmed the live session switched — so
// on every outcome either live model == persisted model, or the client got
// an explicit error naming which side is ahead.
func (s *Server) handleSDKApplyResult(p mcpchan.SDKApplyResultParams) {
	pend, ok := s.takeSDKPending(p.RID)
	if !ok {
		// Unknown or expired rid (the timer already answered): drop, log.
		fmt.Fprintf(os.Stderr, "hotline: sdk_apply_result for unknown/expired rid %q dropped\n", oneLineDetail(p.RID))
		return
	}
	fail := func(code, detail string) {
		s.answerSDKConfig(pend.deviceID, p.RID, func(seq uint64) []byte {
			return sdkConfigResultFrame(seq, p.RID, false, nil, nil, false, false, false, code, oneLineDetail(detail))
		})
	}
	if !p.OK {
		// The harness's closed set (unknown_model|apply_failed|no_session)
		// maps straight into the result frame; the session is untouched and
		// nothing was persisted.
		code := p.Code
		if code == "" {
			code = "apply_failed"
		}
		fail(code, p.Detail)
		return
	}
	// Persist (and echo) what the harness says actually LANDED, not merely what
	// was asked for (pi hot-apply amendment 2026-07-20). The harness's result
	// echoes a field only when the request carried it, so this never invents a
	// change; it only refines one:
	//   - pi canonicalizes "gpt-5.6-sol" → "openai-codex/gpt-5.6-sol", which is
	//     the form HOTLINE_PI_MODEL must hold for the next respawn to resolve it.
	//   - pi's setThinkingLevel CLAMPS to the model's capability, so a "max"
	//     request on a model that tops out at "high" must persist and report
	//     "high" — otherwise the client waits for an identity that never comes.
	// claude-sdk echoes the request verbatim, so its frames stay byte-identical.
	model, effort := pend.model, pend.effort
	if model != nil && p.Model != "" {
		applied := p.Model
		model = &applied
	}
	if effort != nil && p.Effort != "" {
		applied := p.Effort
		effort = &applied
	}
	if err := pend.persist(model, effort); err != nil {
		// Honest about the split state: the live session switched but the
		// knob didn't stick — a restart reverts it.
		fmt.Fprintf(os.Stderr, "hotline: sdk hot apply persist failed (rid %s): %v\n", p.RID, err)
		fail("persist_failed", "applied to the live session but not saved; a restart will revert it")
		return
	}
	s.answerSDKConfig(pend.deviceID, p.RID, func(seq uint64) []byte {
		return sdkConfigResultFrame(seq, p.RID, true, model, effort, false, true, p.Unverified, "", "")
	})
}

// stopSDKPending stops every pending hot-apply timer so a stopped server
// never answers again (mirrors stopAgentStateEmitter; deferred in Run).
func (s *Server) stopSDKPending() {
	s.sdkMu.Lock()
	defer s.sdkMu.Unlock()
	for rid, p := range s.sdkPending {
		p.timer.Stop()
		delete(s.sdkPending, rid)
	}
}

// oneLineDetail flattens s to a single line ≤ 200 chars for the result
// frame's optional detail field (SC3).
func oneLineDetail(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if len(s) > 200 {
		s = s[:200]
	}
	return s
}
