package app

import (
	"sync"
	"time"
)

// typingTTL is how long a single `state:true` frame holds the gate without a
// refresh. 6s = one app refresh interval (4s) plus one full missed-refresh
// margin, so a dropped socket or a killed app releases the hold in <=6s. See the
// typing-signal design (§2).
const typingTTL = 6 * time.Second

// typingGate holds per-device typing expiries for the app channel. In-memory
// only: a box restart clears it (correct — buffered inbound is healed by
// catch-up, and a fresh typing frame re-arms within one refresh interval). A
// typing frame is consumed silently at the inbound frame switch: it is never
// persisted, never a mailbox item, never content, and structurally incapable of
// a wake (it never touches the outbound mailbox fan).
//
// Multi-device rule: ANY device typing holds — active() is an OR over the live
// per-device expiries. A `state:false` from device X clears only X.
type typingGate struct {
	mu     sync.Mutex
	expiry map[string]time.Time // deviceID -> hold-until
	clock  func() time.Time     // injectable for tests; nil ⇒ time.Now

	// onEdge, when set, fires on active⇄idle transitions of the aggregate gate.
	// Reserved for the future pi-only steer upgrade (design §5); nil in v1 so the
	// hook point exists without any behavior attached.
	onEdge func(active bool)
}

func newTypingGate() *typingGate {
	return &typingGate{expiry: make(map[string]time.Time), clock: time.Now}
}

func (g *typingGate) now() time.Time {
	if g.clock != nil {
		return g.clock()
	}
	return time.Now()
}

// set records a device's typing state. true arms the device's hold to now+TTL;
// false deletes the device's entry (an explicit release, <100ms vs waiting out
// the TTL). It fires onEdge only when the aggregate active state flips.
func (g *typingGate) set(deviceID string, typing bool) {
	g.mu.Lock()
	was := g.activeLocked()
	if typing {
		g.expiry[deviceID] = g.now().Add(typingTTL)
	} else {
		delete(g.expiry, deviceID)
	}
	now := g.activeLocked()
	onEdge := g.onEdge
	g.mu.Unlock()

	if onEdge != nil && was != now {
		onEdge(now)
	}
}

// active reports whether any device's hold is live, lazily pruning expired
// entries.
func (g *typingGate) active() bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.activeLocked()
}

// earliestExpiry returns the soonest live hold-until across all devices, pruning
// expired entries. ok is false when no hold is live.
func (g *typingGate) earliestExpiry() (t time.Time, ok bool) {
	g.mu.Lock()
	defer g.mu.Unlock()
	now := g.now()
	for dev, exp := range g.expiry {
		if !exp.After(now) {
			delete(g.expiry, dev)
			continue
		}
		if !ok || exp.Before(t) {
			t, ok = exp, true
		}
	}
	return t, ok
}

// activeLocked prunes expired entries and reports whether any hold remains.
func (g *typingGate) activeLocked() bool {
	now := g.now()
	for dev, exp := range g.expiry {
		if !exp.After(now) {
			delete(g.expiry, dev)
		}
	}
	return len(g.expiry) > 0
}
