package notify

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

// refill returns the token count after refilling a bucket from tokensAt to now,
// capped at burst. A zero tokensAt (a source with no prior state) starts full.
func refill(tokens float64, tokensAt, now time.Time, burst, refillMins float64) float64 {
	if tokensAt.IsZero() {
		return burst
	}
	elapsed := now.Sub(tokensAt).Minutes()
	if elapsed < 0 {
		elapsed = 0 // clock went backwards; never grant free tokens
	}
	t := tokens + elapsed/refillMins
	if t > burst {
		t = burst
	}
	return t
}

// quietHours is a parsed "HH:MM-HH:MM" window evaluated in the local zone.
// Midnight-spanning ("23:00-08:00") is supported. An empty spec disables it.
type quietHours struct {
	enabled          bool
	startMin, endMin int // minutes since local midnight
}

// parseQuietHours parses the "HH:MM-HH:MM" registry field. Empty is disabled
// (not an error); a malformed value is a loud error the operator must fix.
func parseQuietHours(s string) (quietHours, error) {
	s = strings.TrimSpace(s)
	if s == "" {
		return quietHours{enabled: false}, nil
	}
	a, b, ok := strings.Cut(s, "-")
	if !ok {
		return quietHours{}, fmt.Errorf("quietHours %q: want HH:MM-HH:MM", s)
	}
	sh, sm, err := parseHHMM(a)
	if err != nil {
		return quietHours{}, fmt.Errorf("quietHours %q: %w", s, err)
	}
	eh, em, err := parseHHMM(b)
	if err != nil {
		return quietHours{}, fmt.Errorf("quietHours %q: %w", s, err)
	}
	return quietHours{enabled: true, startMin: sh*60 + sm, endMin: eh*60 + em}, nil
}

// parseHHMM parses a 24-hour "HH:MM" (leading zero optional).
func parseHHMM(s string) (h, m int, err error) {
	hs, ms, ok := strings.Cut(strings.TrimSpace(s), ":")
	if !ok {
		return 0, 0, fmt.Errorf("time %q: want HH:MM", s)
	}
	h, err = strconv.Atoi(strings.TrimSpace(hs))
	if err != nil || h < 0 || h > 23 {
		return 0, 0, fmt.Errorf("time %q: hour out of range", s)
	}
	m, err = strconv.Atoi(strings.TrimSpace(ms))
	if err != nil || m < 0 || m > 59 {
		return 0, 0, fmt.Errorf("time %q: minute out of range", s)
	}
	return h, m, nil
}

// contains reports whether t (in its own local wall clock) is inside the window.
// A zero-width window (start == end) is treated as never active.
func (q quietHours) contains(t time.Time) bool {
	if !q.enabled || q.startMin == q.endMin {
		return false
	}
	cur := t.Hour()*60 + t.Minute()
	if q.startMin < q.endMin {
		return cur >= q.startMin && cur < q.endMin
	}
	// Midnight-spanning: [start, 24:00) ∪ [00:00, end).
	return cur >= q.startMin || cur < q.endMin
}

// endLabel renders the window's end as "HH:MM" (the "queued until …" time).
func (q quietHours) endLabel() string {
	return fmt.Sprintf("%02d:%02d", q.endMin/60, q.endMin%60)
}
