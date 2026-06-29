package telegram

import (
	"context"
	"errors"
	"fmt"
	"os"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

const (
	// LongPollSecs is the getUpdates long-poll timeout.
	LongPollSecs = 50
	// Max409 is how many consecutive 409 Conflict errors we tolerate before
	// giving up (another poller holds the token).
	Max409 = 8
	// MaxBackoff caps the reconnect backoff.
	MaxBackoff = 15 * time.Second
)

var allowedUpdates = []string{"message", "edited_message", "callback_query", "message_reaction"}

// ErrTokenContended is returned by Poll when it gives up after Max409
// consecutive 409 Conflicts — another poller holds the bot token. The caller
// routes this through the unified shutdown path so the force-exit safety net is
// armed and the give-up is logged through the single funnel.
var ErrTokenContended = errors.New("409 Conflict persists — another poller holds the bot token")

// Poll runs the resilient long-poll loop until ctx is cancelled. Updates are
// processed serially in update_id order; the offset advances before dispatch so
// a panicking update is never re-fetched. Handler panics are recovered.
//
// It returns nil when ctx is cancelled (a graceful, externally-driven stop) and
// ErrTokenContended when it gives up on persistent 409 Conflicts.
func Poll(ctx context.Context, bot *gotgbot.Bot, dispatch func(ctx context.Context, u *gotgbot.Update)) error {
	// Register commands once at startup (best-effort).
	if err := SetCommands(bot); err != nil {
		fmt.Fprintf(os.Stderr, "tele-go: setMyCommands failed: %v\n", err)
	}
	if bot.User.Username != "" {
		fmt.Fprintf(os.Stderr, "tele-go: polling as @%s\n", bot.User.Username)
	}

	var offset int64
	attempt := 0
	conflicts := 0

	for {
		if ctx.Err() != nil {
			return nil
		}

		updates, err := bot.GetUpdatesWithContext(ctx, &gotgbot.GetUpdatesOpts{
			Offset:         offset,
			Timeout:        LongPollSecs,
			AllowedUpdates: allowedUpdates,
			RequestOpts:    &gotgbot.RequestOpts{Timeout: (LongPollSecs + 5) * time.Second},
		})
		if err != nil {
			if ctx.Err() != nil || errors.Is(err, context.Canceled) {
				return nil
			}

			var tgErr *gotgbot.TelegramError
			switch {
			case errors.As(err, &tgErr) && tgErr.Code == 409:
				conflicts++
				if conflicts >= Max409 {
					fmt.Fprintf(os.Stderr, "tele-go: 409 Conflict persists after %d attempts — another poller holds the bot token. Exiting poll loop.\n", conflicts)
					return ErrTokenContended
				}
			case errors.As(err, &tgErr) && tgErr.Code == 429 && tgErr.ResponseParams != nil && tgErr.ResponseParams.RetryAfter > 0:
				wait := time.Duration(tgErr.ResponseParams.RetryAfter) * time.Second
				fmt.Fprintf(os.Stderr, "tele-go: 429 rate limited, retrying in %s\n", wait)
				if sleepCtx(ctx, wait) {
					return nil
				}
				continue
			default:
				conflicts = 0
			}

			attempt++
			delay := time.Duration(attempt) * time.Second
			if delay > MaxBackoff {
				delay = MaxBackoff
			}
			fmt.Fprintf(os.Stderr, "tele-go: polling error: %v, retrying in %s\n", err, delay)
			if sleepCtx(ctx, delay) {
				return nil
			}
			continue
		}

		// Success — reset error counters.
		attempt = 0
		conflicts = 0

		for i := range updates {
			u := &updates[i]
			// Advance the offset BEFORE dispatch so a failed update is never
			// re-fetched.
			offset = u.UpdateId + 1
			dispatchSafely(ctx, dispatch, u)
		}
	}
}

// dispatchSafely runs dispatch with panic recovery so one bad update can't kill
// the poll loop.
func dispatchSafely(ctx context.Context, dispatch func(ctx context.Context, u *gotgbot.Update), u *gotgbot.Update) {
	defer func() {
		if r := recover(); r != nil {
			fmt.Fprintf(os.Stderr, "tele-go: recovered from handler panic: %v\n", r)
		}
	}()
	dispatch(ctx, u)
}

// sleepCtx sleeps for d or until ctx is done. It returns true if ctx was
// cancelled (caller should exit).
func sleepCtx(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return true
	case <-t.C:
		return false
	}
}
