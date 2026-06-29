package telegram

import (
	"context"
	"testing"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

// TestDispatchSafelyRecoversPanic confirms a panicking dispatch cannot crash
// the poll loop: dispatchSafely must swallow the panic and return normally.
func TestDispatchSafelyRecoversPanic(t *testing.T) {
	done := make(chan struct{})
	go func() {
		defer close(done)
		dispatchSafely(context.Background(), func(context.Context, *gotgbot.Update) {
			panic("boom")
		}, &gotgbot.Update{})
	}()
	select {
	case <-done:
		// returned without propagating the panic
	case <-time.After(2 * time.Second):
		t.Fatal("dispatchSafely did not return after a panicking handler")
	}
}

// TestDispatchSafelyRunsHandler confirms the normal path actually invokes the
// dispatch function with the supplied update.
func TestDispatchSafelyRunsHandler(t *testing.T) {
	var gotID int64
	dispatchSafely(context.Background(), func(_ context.Context, u *gotgbot.Update) {
		gotID = u.UpdateId
	}, &gotgbot.Update{UpdateId: 77})
	if gotID != 77 {
		t.Fatalf("dispatch not invoked with update; gotID=%d", gotID)
	}
}

// TestSleepCtxTimerElapses: when the timer fires before cancellation, sleepCtx
// returns false (caller should keep going).
func TestSleepCtxTimerElapses(t *testing.T) {
	if cancelled := sleepCtx(context.Background(), 10*time.Millisecond); cancelled {
		t.Fatal("expected false (timer elapsed), got true")
	}
}

// TestSleepCtxCancelled: an already-cancelled context returns true immediately.
func TestSleepCtxCancelled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	start := time.Now()
	if cancelled := sleepCtx(ctx, 10*time.Second); !cancelled {
		t.Fatal("expected true (ctx cancelled), got false")
	}
	if time.Since(start) > time.Second {
		t.Fatal("sleepCtx should return promptly on a cancelled ctx, not wait out the duration")
	}
}
