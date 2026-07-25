package app

import (
	"sync/atomic"
	"testing"
	"time"
)

// TestWakeGateRapidBurstCapped is the S-2 regression: 8 durable items arriving
// in a rapid burst on one away device must yield at most 6 wake POSTs (the
// core's per-device cap), so the box never spends its shared per-IP budget on
// wakes the core would reject.
func TestWakeGateRapidBurstCapped(t *testing.T) {
	var nowSec int64 = 1_000_000
	g := newWakeGate()
	g.clock = func() time.Time { return time.Unix(atomic.LoadInt64(&nowSec), 0) }

	allowed := 0
	for i := 0; i < 8; i++ { // same instant: a rapid burst
		if ok, _ := g.allow("dev-1"); ok {
			allowed++
		}
	}
	if allowed > wakeMaxPerMinute {
		t.Fatalf("rapid burst posted %d wakes, want <= %d", allowed, wakeMaxPerMinute)
	}
	// A same-second burst is additionally clamped by the 10s min gap: exactly one
	// wake should get through before the core would reject the rest.
	if allowed != 1 {
		t.Fatalf("same-second burst want exactly 1 wake through, got %d", allowed)
	}
}

// TestWakeGateSixPerMinute proves the rolling-minute cap: spacing wakes past the
// 10s min gap, at most 6 pass within any 60s window.
func TestWakeGateSixPerMinute(t *testing.T) {
	var nowSec int64 = 1_000_000
	g := newWakeGate()
	g.clock = func() time.Time { return time.Unix(atomic.LoadInt64(&nowSec), 0) }

	allowed := 0
	// Hammer once per second for a full 60s window: the 10s min-gap + 6/min cap
	// together admit at most 6 wakes in the window (t=0,10,20,30,40,50).
	for i := 0; i < 60; i++ {
		if ok, _ := g.allow("dev-1"); ok {
			allowed++
		}
		atomic.AddInt64(&nowSec, 1)
	}
	if allowed != wakeMaxPerMinute {
		t.Fatalf("want %d wakes admitted across a 60s window, got %d", wakeMaxPerMinute, allowed)
	}
}

// TestWakeGateSuppressionLogOncePerWindow proves the debug line fires at most
// once per rolling 60s window per device, not once per dropped item.
func TestWakeGateSuppressionLogOncePerWindow(t *testing.T) {
	var nowSec int64 = 1_000_000
	g := newWakeGate()
	g.clock = func() time.Time { return time.Unix(atomic.LoadInt64(&nowSec), 0) }

	logs := 0
	for i := 0; i < 8; i++ { // same instant: first passes, 7 suppressed
		if ok, logNow := g.allow("dev-1"); !ok && logNow {
			logs++
		}
	}
	if logs != 1 {
		t.Fatalf("want exactly one suppression log for the burst, got %d", logs)
	}
}

// TestWakeGatePerDeviceIsolation proves one device's budget does not affect
// another's (the gate keys on device id).
func TestWakeGatePerDeviceIsolation(t *testing.T) {
	var nowSec int64 = 1_000_000
	g := newWakeGate()
	g.clock = func() time.Time { return time.Unix(atomic.LoadInt64(&nowSec), 0) }

	if ok, _ := g.allow("dev-a"); !ok {
		t.Fatal("first wake for dev-a must pass")
	}
	if ok, _ := g.allow("dev-b"); !ok {
		t.Fatal("first wake for dev-b must pass despite dev-a's tick")
	}
}
