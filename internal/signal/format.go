package signal

import (
	"regexp"
	"strings"
	"time"
	"unicode/utf8"
)

// MaxChunkLimit is the per-message character cap we enforce. Signal clients
// render roughly 2000 characters inline and turn longer bodies into "long
// text" attachments, so we chunk at 2000 like the discord adapter.
const MaxChunkLimit = 2000

// unsafeNameRe matches characters that would let a sender-controlled name
// break out of the <channel> meta block.
var unsafeNameRe = regexp.MustCompile(`[<>\[\]\r\n;]`)

// SafeName replaces delimiter characters in a sender-controlled name so it
// can't forge or escape the inbound notification meta.
func SafeName(s string) string {
	if s == "" {
		return ""
	}
	return unsafeNameRe.ReplaceAllString(s, "_")
}

// Chunk splits text into pieces no longer than limit runes, clamped to
// 1..MaxChunkLimit. In "newline" mode the split prefers the last paragraph
// break, then the last line break, then the last space within the window,
// falling back to a hard cut. Operates on runes so multibyte text isn't split
// mid-character. Mirrors the telegram/discord splitters.
func Chunk(s string, limit int, mode string) []string {
	if limit < 1 {
		limit = 1
	}
	if limit > MaxChunkLimit {
		limit = MaxChunkLimit
	}

	runes := []rune(s)
	if len(runes) <= limit {
		return []string{s}
	}

	var out []string
	for len(runes) > limit {
		cut := limit
		if mode == "newline" {
			window := string(runes[:limit])
			if p := lastIndexRune(window, "\n\n"); p > limit/2 {
				cut = p
			} else if l := lastIndexRune(window, "\n"); l > limit/2 {
				cut = l
			} else if sp := lastIndexRune(window, " "); sp > 0 {
				cut = sp
			}
		}
		out = append(out, string(runes[:cut]))
		runes = runes[cut:]
		for len(runes) > 0 && runes[0] == '\n' {
			runes = runes[1:]
		}
	}
	if len(runes) > 0 {
		out = append(out, string(runes))
	}
	return out
}

// lastIndexRune returns the rune index (not byte index) of the last occurrence
// of sep in s, or -1.
func lastIndexRune(s, sep string) int {
	b := strings.LastIndex(s, sep)
	if b < 0 {
		return -1
	}
	return len([]rune(s[:b]))
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

// Bubble pacing — same feel as the telegram/discord adapters.
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

func nonEmpty(s, fallback string) string {
	if s == "" {
		return fallback
	}
	return s
}
