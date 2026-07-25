package app

import (
	"fmt"
	"time"
)

// This file is milestone F1 — ORCHESTRATOR AUTHORITY (fleet-workers design §A1,
// orchestrator-authority seed). It is the whole of the authority model:
//
//	GRANT, NEVER CLAIM. Authority is a property of the RECEIVING box's own registry
//	entry for an edge, written ONLY by the operator through `hotline fleet grant`.
//	No inbound frame, no MCP tool, no peer message can create, extend, or refresh
//	it — nothing on the wire is ever read into FleetEdge.Authority, and the fleet_msg
//	journal frame is rebuilt from validated fields only (deliverInbound), so an
//	unmodeled "authority" field on a peer frame is discarded before it is stored.
//
//	BOUND TO THE KEY, NOT THE NAME. A grant records the peer's pinned box-key
//	fingerprint (PeerKeyFP, the M5 trust-on-first-use pin). The alias is renameable
//	and therefore never load-bearing: if the pin changes, or a KeyFPMismatch is
//	persisted, the grant stops applying (it is not deleted — it simply never matches).
//
//	TYPED + STRUCTURALLY BOUNDED. Directive framing applies ONLY to the DOWN kinds
//	(fleetDirectiveKinds). `restart` is structurally absent from the fleet kind enum
//	(fleetKinds) — there is no code path that can express it as a directive, so a
//	fully compromised orchestrator cannot even phrase the order, let alone have it
//	obeyed. Operator powers (pairing, access, cap escalation) have no kind either.
//
//	NON-TRANSITIVE. Authority lives on ONE edge record on ONE box. Nothing in the
//	wire shape carries it, so it cannot relay: a grant on edge A confers nothing on
//	edge B, and no frame content can extend it. Only the operator grants.
//
//	ONE-WAY. Frames this box SENDS are plain fleet_msg — a grant on an inbound edge
//	changes nothing about the outbound path, so a granted worker cannot steer its hub.
//
//	EXPIRES. An optional TTL (`--ttl`) stamps ExpiresAt; past it the grant silently
//	demotes to normal untrusted-peer framing on the very next frame (no cleanup job
//	required — expiry is evaluated per delivery).
//
//	REVOCABLE NEXT FRAME. injectInbound re-reads the edge from disk before framing a
//	directive-kind frame, so `hotline fleet revoke` (a different process) takes effect
//	on the next inbound frame, not on the next session.

// FleetAuthority is the operator-granted orchestrator authority on ONE edge of the
// RECEIVING box's registry (§A1). Its presence flips inbound framing for the typed
// down-kinds only. KeyFP is the peer box-key fingerprint the grant is bound to,
// copied from the edge's pin at grant time — the grant applies only while the edge's
// live pin still equals it.
type FleetAuthority struct {
	KeyFP     string `json:"key_fp"`
	GrantedAt string `json:"granted_at"`
	// ExpiresAt is the RFC3339 TTL horizon. Empty means no expiry (revoke-only).
	ExpiresAt string `json:"expires_at,omitempty"`
}

// fleetDirectiveKinds is the DOWN vocabulary — the closed set of kinds that may be
// framed as an orchestrator directive on a granted edge (§A1/§A4):
//
//	task       = assign  (the design's ruling: `task` on a granted edge IS `assign`;
//	                      no separate kind was added, so old peers stay compatible)
//	cancel     = wind the work down gracefully, send a final result
//	status_req = report your state
//
// Everything else — brief / result / ack (accept) / refuse / ping, the UP vocabulary —
// is never a directive, even on a granted edge: it gets today's untrusted-peer marker
// verbatim. `restart` is absent from fleetKinds entirely and therefore unreachable here.
var fleetDirectiveKinds = map[string]bool{"task": true, "cancel": true, "status_req": true}

// fleetAuthorityReason is the audit word for why a frame was or was not framed as a
// directive. It rides the fleet.log line so the operator can read the decision.
type fleetAuthorityReason string

const (
	authGranted      fleetAuthorityReason = "granted"
	authNoGrant      fleetAuthorityReason = "no_grant"
	authNotDirective fleetAuthorityReason = "kind_not_directive"
	authExpired      fleetAuthorityReason = "expired"
	authKeyUnbound   fleetAuthorityReason = "key_fp_unbound"
	authKeyMismatch  fleetAuthorityReason = "key_mismatch"
	authRemoved      fleetAuthorityReason = "edge_removed"
)

// fleetAuthorityFor is the ONE authority decision, evaluated per delivered frame.
// It is deliberately total and fail-closed: every path that is not an exact,
// unexpired, key-bound, mismatch-free grant on a down-kind returns false.
//
// mismatch is the caller's live M5 verdict (this frame's fp differed from the pin, OR
// the edge carries a persisted KeyFPMismatch) — a mismatch ALWAYS wins over a grant,
// the same rule attestation follows. fromKeyFP is the fingerprint the frame itself
// presented ("" when the peer sent none); when present it must equal the grant's
// binding as well, so a grant can never be exercised under a different key.
func fleetAuthorityFor(edge FleetEdge, kind, fromKeyFP string, mismatch bool, now time.Time) (bool, fleetAuthorityReason) {
	a := edge.Authority
	if a == nil {
		return false, authNoGrant
	}
	if edge.Removed() {
		return false, authRemoved
	}
	if mismatch {
		return false, authKeyMismatch
	}
	if a.KeyFP == "" || edge.PeerKeyFP == "" || a.KeyFP != edge.PeerKeyFP {
		return false, authKeyUnbound
	}
	if fromKeyFP != "" && fromKeyFP != a.KeyFP {
		return false, authKeyUnbound
	}
	if a.expired(now) {
		return false, authExpired
	}
	if !fleetDirectiveKinds[kind] {
		return false, authNotDirective
	}
	return true, authGranted
}

// expired reports whether a grant's TTL has passed. An UNPARSABLE ExpiresAt counts as
// expired (fail-closed: a corrupt horizon must never read as unlimited authority).
func (a *FleetAuthority) expired(now time.Time) bool {
	if a == nil {
		return true
	}
	if a.ExpiresAt == "" {
		return false
	}
	t, err := time.Parse(time.RFC3339, a.ExpiresAt)
	if err != nil {
		return true
	}
	return !now.Before(t)
}

// AuthorityStatus renders the operator-facing state of an edge's grant for `fleet ls`
// and the fleet tool: "" (no grant), "granted", "granted,expires_in=…", or "expired".
// Display only — never the framing decision (that is fleetAuthorityFor).
func (e FleetEdge) AuthorityStatus(now time.Time) string {
	a := e.Authority
	if a == nil {
		return ""
	}
	if a.expired(now) {
		return "expired"
	}
	if a.KeyFP == "" || e.PeerKeyFP == "" || a.KeyFP != e.PeerKeyFP {
		return "unbound"
	}
	if a.ExpiresAt == "" {
		return "granted"
	}
	t, err := time.Parse(time.RFC3339, a.ExpiresAt)
	if err != nil {
		return "expired"
	}
	return "granted,expires_in=" + t.Sub(now).Round(time.Minute).String()
}

// HasAuthority reports whether a live, key-bound, unexpired grant exists (the boolean
// the operator surfaces carry). Not the framing gate — kind gating lives in
// fleetAuthorityFor.
func (e FleetEdge) HasAuthority(now time.Time) bool {
	ok, _ := fleetAuthorityFor(e, "task", "", false, now)
	return ok
}

// GrantAuthority is the OPERATOR-ONLY write path for orchestrator authority (§A1) —
// reachable from `hotline fleet grant` and nowhere else. It binds the grant to the
// edge's CURRENT pinned peer key, so a grant can never be minted for an unidentified
// peer, and refuses outright on a tombstoned edge or one carrying a key mismatch.
// ttl <= 0 means no expiry (revoke-only).
func (s *FleetStore) GrantAuthority(arg string, ttl time.Duration) (FleetEdge, error) {
	var out FleetEdge
	err := s.mutate(func(st *fleetState) error {
		id, err := resolveEdge(st, arg)
		if err != nil {
			return err
		}
		e := st.Edges[id]
		if e.Removed() {
			return fmt.Errorf("fleet edge %s (%s) is removed: re-pair before granting authority", id, e.Alias)
		}
		if e.PeerKeyFP == "" {
			return fmt.Errorf("fleet edge %s (%s) has no pinned peer key yet: authority binds to the peer's box key, which is pinned on the first connected session — connect the edge once, then grant", id, e.Alias)
		}
		// A persisted mismatch means we do not know who is on the other end. Refuse to
		// mint a grant at all rather than mint one that silently never applies.
		if est, eerr := s.loadEdgeStateLocked(id); eerr == nil && est.KeyFPMismatch {
			return fmt.Errorf("fleet edge %s (%s) has a KEY MISMATCH: the peer's box key changed since pairing — `hotline fleet rm` and re-pair before granting authority", id, e.Alias)
		}
		a := &FleetAuthority{KeyFP: e.PeerKeyFP, GrantedAt: fleetNow()}
		if ttl > 0 {
			a.ExpiresAt = time.Now().UTC().Add(ttl).Format(time.RFC3339)
		}
		e.Authority = a
		st.Edges[id] = e
		out = e
		return nil
	})
	if err != nil {
		return FleetEdge{}, err
	}
	return out, nil
}

// RevokeAuthority drops an edge's grant (§A1: revocation is a registry write and takes
// effect on the NEXT inbound frame, because injectInbound re-reads the edge per
// directive-kind delivery). had reports whether a grant was actually present, so the
// CLI can say so plainly. Works on a tombstoned edge too — clearing a stale grant is
// always allowed.
func (s *FleetStore) RevokeAuthority(arg string) (edge FleetEdge, had bool, err error) {
	err = s.mutate(func(st *fleetState) error {
		id, rerr := resolveEdge(st, arg)
		if rerr != nil {
			return rerr
		}
		e := st.Edges[id]
		had = e.Authority != nil
		e.Authority = nil
		st.Edges[id] = e
		edge = e
		return nil
	})
	if err != nil {
		return FleetEdge{}, false, err
	}
	return edge, had, nil
}

// orNone renders an empty audit field as "none" so a fleet.log line never reads as a
// truncated key=value pair.
func orNone(s string) string {
	if s == "" {
		return "none"
	}
	return s
}

// FleetAudit appends one operator-audit line to <stateDir>/fleet.log from a CLI
// process — the same append-only log the running box writes session lines to, so
// grant/revoke land in the operator's single fleet timeline. Best-effort and nil-safe.
func FleetAudit(stateDir, format string, args ...any) {
	openFleetLogger(stateDir).logf(format, args...)
}
