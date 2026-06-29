package telegram

import (
	"strings"
	"time"
	"unicode/utf8"
)

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

// Bubble pacing constants. A multi-bubble reply is delivered with a typing
// indicator and a per-bubble pause scaled to the bubble's length, so a burst
// lands with a human texting cadence instead of all at once.
const (
	// bubblePerCharMs simulates typing speed (~36 wpm-ish at 6 chars/word).
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
