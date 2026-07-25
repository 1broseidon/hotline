package app

import (
	"context"
	"strings"
	"time"
	"unicode/utf8"
)

// nonBlank returns s if it is non-empty, else fallback.
func nonBlank(s, fallback string) string {
	if strings.TrimSpace(s) == "" {
		return fallback
	}
	return s
}

// countNonBlank returns how many entries are not empty-after-trimming.
func countNonBlank(xs []string) int {
	n := 0
	for _, x := range xs {
		if strings.TrimSpace(x) != "" {
			n++
		}
	}
	return n
}

// nonBlankItems returns the entries that are not empty-after-trimming, trimmed
// only of surrounding whitespace? No — original bubbles are preserved verbatim;
// only fully-blank entries are dropped (matching the signal/telegram loop).
func nonBlankItems(xs []string) []string {
	out := make([]string, 0, len(xs))
	for _, x := range xs {
		if strings.TrimSpace(x) != "" {
			out = append(out, x)
		}
	}
	return out
}

// truncate clips s to at most n runes, appending an ellipsis when it had to cut.
// Rune-aware so a multibyte character is never split. Used to bound the quoted
// reply-context text handed to the harness.
func truncate(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	rs := []rune(s)
	return string(rs[:n]) + "…"
}

// maxButtons caps how many options one reply can carry. Matches the other
// adapters.
const maxButtons = 12

// sanitizeButtons trims labels, drops blanks, and caps the count.
func sanitizeButtons(in []string) []string {
	out := make([]string, 0, len(in))
	for _, b := range in {
		if b = strings.TrimSpace(b); b == "" {
			continue
		}
		out = append(out, b)
		if len(out) == maxButtons {
			break
		}
	}
	return out
}

// Bubble pacing — lifted from the signal adapter so app bubbles arrive with the
// same human texting cadence.
const (
	bubblePerCharMs = 28
	bubbleMinDelay  = 350 * time.Millisecond
	bubbleMaxDelay  = 2200 * time.Millisecond
)

// bubbleDelay returns how long to "type" a bubble before sending it, scaled to
// its rune length and clamped to [bubbleMinDelay, bubbleMaxDelay].
func bubbleDelay(s string) time.Duration {
	d := time.Duration(utf8.RuneCountInString(s)*bubblePerCharMs) * time.Millisecond
	if d < bubbleMinDelay {
		return bubbleMinDelay
	}
	if d > bubbleMaxDelay {
		return bubbleMaxDelay
	}
	return d
}

// sleepCtx sleeps for d or until ctx is done, reporting whether it was cut short
// by cancellation.
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
