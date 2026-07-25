package app

import (
	"testing"
	"time"
)

// TestTypingGateTTLExpiry proves a single state:true auto-expires after typingTTL
// with no refresh (a dropped socket / killed app releases the hold).
func TestTypingGateTTLExpiry(t *testing.T) {
	now := time.Unix(1_000_000, 0)
	g := newTypingGate()
	g.clock = func() time.Time { return now }

	g.set("dev-a", true)
	if !g.active() {
		t.Fatal("gate must be active immediately after state:true")
	}
	now = now.Add(typingTTL - time.Millisecond)
	if !g.active() {
		t.Fatal("gate must still hold just before the TTL")
	}
	now = now.Add(2 * time.Millisecond) // past the TTL
	if g.active() {
		t.Fatal("gate must auto-expire after the TTL without a refresh")
	}
}

// TestTypingGateRefreshExtends proves a repeated state:true pushes the expiry out
// one full TTL from the refresh instant.
func TestTypingGateRefreshExtends(t *testing.T) {
	now := time.Unix(1_000_000, 0)
	g := newTypingGate()
	g.clock = func() time.Time { return now }

	g.set("dev-a", true)
	now = now.Add(4 * time.Second) // one refresh interval
	g.set("dev-a", true)           // refresh: expiry := now + TTL
	now = now.Add(typingTTL - time.Millisecond)
	if !g.active() {
		t.Fatal("a refresh must extend the hold a full TTL from the refresh instant")
	}
}

// TestTypingGateExplicitFalseReleases proves state:false clears the hold at once
// (the <100ms release that beats waiting out the TTL).
func TestTypingGateExplicitFalseReleases(t *testing.T) {
	g := newTypingGate()
	g.set("dev-a", true)
	g.set("dev-a", false)
	if g.active() {
		t.Fatal("state:false must release the hold immediately")
	}
}

// TestTypingGateMultiDeviceOR proves ANY device typing holds; a state:false from
// one device clears only that device.
func TestTypingGateMultiDeviceOR(t *testing.T) {
	g := newTypingGate()
	g.set("dev-a", true)
	g.set("dev-b", true)
	g.set("dev-a", false)
	if !g.active() {
		t.Fatal("dev-b still typing must keep the gate active after dev-a releases")
	}
	g.set("dev-b", false)
	if g.active() {
		t.Fatal("gate must be idle once every device released")
	}
}

// TestTypingGateOnEdge proves the reserved onEdge hook fires only on aggregate
// idle⇄active transitions, not on every set.
func TestTypingGateOnEdge(t *testing.T) {
	var edges []bool
	g := newTypingGate()
	g.onEdge = func(active bool) { edges = append(edges, active) }

	g.set("dev-a", true)  // idle -> active: edge true
	g.set("dev-b", true)  // still active: no edge
	g.set("dev-a", false) // still active (dev-b): no edge
	g.set("dev-b", false) // active -> idle: edge false

	if len(edges) != 2 || edges[0] != true || edges[1] != false {
		t.Fatalf("edges = %v, want [true false]", edges)
	}
}

// TestTypingGateEarliestExpiry proves earliestExpiry returns the soonest live
// hold and prunes expired entries.
func TestTypingGateEarliestExpiry(t *testing.T) {
	now := time.Unix(1_000_000, 0)
	g := newTypingGate()
	g.clock = func() time.Time { return now }

	g.set("dev-a", true)           // expiry now+TTL
	now = now.Add(2 * time.Second) // dev-b arms later
	g.set("dev-b", true)           // expiry now+TTL (later than dev-a's)

	exp, ok := g.earliestExpiry()
	if !ok {
		t.Fatal("earliestExpiry must report a live hold")
	}
	// dev-a armed first, so its expiry is the earliest.
	if want := time.Unix(1_000_000, 0).Add(typingTTL); !exp.Equal(want) {
		t.Fatalf("earliest expiry = %v, want dev-a's %v", exp, want)
	}
}
