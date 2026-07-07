package supervise

import (
	"testing"
	"time"
)

func TestBackoffSequenceAndCap(t *testing.T) {
	b := &Backoff{Initial: 2 * time.Second, Max: 10 * time.Minute, Healthy: 5 * time.Minute}
	want := []time.Duration{
		2 * time.Second, 4 * time.Second, 8 * time.Second, 16 * time.Second,
		32 * time.Second, 64 * time.Second, 128 * time.Second, 256 * time.Second,
		512 * time.Second, 10 * time.Minute, 10 * time.Minute, // capped, stays capped
	}
	for i, w := range want {
		if got := b.Next(0); got != w {
			t.Fatalf("Next #%d = %v, want %v", i, got, w)
		}
	}
}

func TestBackoffHealthyUptimeResets(t *testing.T) {
	b := &Backoff{Initial: 2 * time.Second, Max: 10 * time.Minute, Healthy: 5 * time.Minute}
	b.Next(0)
	b.Next(0)
	if got := b.Next(0); got != 8*time.Second {
		t.Fatalf("third crash = %v, want 8s", got)
	}
	// A healthy run resets the sequence.
	if got := b.Next(6 * time.Minute); got != 2*time.Second {
		t.Errorf("after healthy run = %v, want 2s", got)
	}
	// An almost-healthy run does not.
	if got := b.Next(4 * time.Minute); got != 4*time.Second {
		t.Errorf("after short run = %v, want 4s", got)
	}
}

func TestBackoffReset(t *testing.T) {
	b := &Backoff{Initial: 2 * time.Second, Max: 10 * time.Minute, Healthy: 5 * time.Minute}
	b.Next(0)
	b.Next(0)
	b.Reset()
	if got := b.Next(0); got != 2*time.Second {
		t.Errorf("after Reset = %v, want 2s", got)
	}
}

// TestBackoffShiftOverflow guards the d<=0 overflow escape: an absurd number
// of consecutive failures must still return Max, never a negative delay.
func TestBackoffShiftOverflow(t *testing.T) {
	b := &Backoff{Initial: 2 * time.Second, Max: 10 * time.Minute, Healthy: 5 * time.Minute}
	b.n = 62
	if got := b.Next(0); got != 10*time.Minute {
		t.Errorf("overflowed shift = %v, want Max", got)
	}
}
