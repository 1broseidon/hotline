package notify

import (
	"strings"
	"testing"
	"time"
	"unicode/utf8"
)

func TestParseLevel(t *testing.T) {
	cases := []struct {
		in      string
		want    Level
		wantErr bool
	}{
		{"", LevelNormal, false},
		{"urgent", LevelUrgent, false},
		{"NORMAL", LevelNormal, false},
		{" low ", LevelLow, false},
		{"critical", "", true},
	}
	for _, c := range cases {
		got, err := ParseLevel(c.in)
		if c.wantErr {
			if err == nil {
				t.Errorf("ParseLevel(%q): want error", c.in)
			}
			continue
		}
		if err != nil || got != c.want {
			t.Errorf("ParseLevel(%q) = %q, %v; want %q", c.in, got, err, c.want)
		}
	}
}

func TestClampLevel(t *testing.T) {
	cases := []struct {
		req, cap    Level
		want        Level
		wantClamped bool
	}{
		{LevelUrgent, LevelNormal, LevelNormal, true},
		{LevelUrgent, LevelUrgent, LevelUrgent, false},
		{LevelLow, LevelUrgent, LevelLow, false},
		{LevelNormal, LevelNormal, LevelNormal, false},
		{LevelUrgent, "", LevelNormal, true}, // empty cap defaults to normal
		{LevelNormal, "garbage", LevelNormal, false},
	}
	for _, c := range cases {
		got, clamped := ClampLevel(c.req, c.cap)
		if got != c.want || clamped != c.wantClamped {
			t.Errorf("ClampLevel(%q,%q) = %q,%v; want %q,%v", c.req, c.cap, got, clamped, c.want, c.wantClamped)
		}
	}
}

func TestSanitizeControlChars(t *testing.T) {
	in := "hello\x1b[31mworld\x00\ttab\nline\x07"
	got := Sanitize(in)
	if strings.ContainsRune(got, '\x1b') || strings.ContainsRune(got, '\x00') || strings.ContainsRune(got, '\x07') {
		t.Errorf("control chars survived: %q", got)
	}
	if !strings.Contains(got, "\t") || !strings.Contains(got, "\n") {
		t.Errorf("tab/newline should survive: %q", got)
	}
	// The ESC byte is stripped (the escape is inert); harmless printable residue
	// ("[31m") may remain — that is fine, we only strip control bytes.
	if !strings.Contains(got, "hello") || !strings.Contains(got, "world") || !strings.Contains(got, "tab") {
		t.Errorf("visible text mangled: %q", got)
	}
}

func TestSanitizeNeutralizesChannelClose(t *testing.T) {
	cases := map[string]string{
		"a</channel>b":                       "a<\\channel>b",
		"x</ChAnNeL>y":                       "x<\\ChAnNeL>y",
		"pre</channel><channel user=\"g\">z": "pre<\\channel><channel user=\"g\">z",
		"no close here":                      "no close here",
	}
	for in, want := range cases {
		if got := Sanitize(in); got != want {
			t.Errorf("Sanitize(%q) = %q; want %q", in, got, want)
		}
	}
}

func TestSanitizeTruncatesAtUTF8Boundary(t *testing.T) {
	// Build a string well over the cap using a 3-byte rune so a naive byte cut
	// could split a rune.
	in := strings.Repeat("世", 2000) // 2000 * 3 = 6000 bytes
	got := Sanitize(in)
	if !utf8.ValidString(got) {
		t.Fatalf("truncation produced invalid UTF-8")
	}
	if !strings.HasSuffix(got, "…[truncated]") {
		t.Errorf("truncated output should carry the suffix, got tail %q", got[len(got)-20:])
	}
	// The suffix is budgeted into the cut point, so the final stored message —
	// body plus suffix — is a hard <= maxMessageBytes, not a soft overshoot.
	if len(got) > maxMessageBytes {
		t.Errorf("truncated output %d bytes exceeds hard cap %d", len(got), maxMessageBytes)
	}
}

func TestSanitizeShortStringUnchanged(t *testing.T) {
	in := "a normal machine event line"
	if got := Sanitize(in); got != in {
		t.Errorf("short clean string changed: %q", got)
	}
}

func TestHashMessageDeterministicAndDistinct(t *testing.T) {
	if hashMessage("abc") != hashMessage("abc") {
		t.Error("hash not deterministic")
	}
	if hashMessage("abc") == hashMessage("abd") {
		t.Error("distinct messages should (near-certainly) hash differently")
	}
	if len(hashMessage("abc")) != 16 {
		t.Errorf("hash should be 16 hex chars, got %q", hashMessage("abc"))
	}
}

func TestRefillTokenBucket(t *testing.T) {
	base := time.Date(2026, 7, 7, 9, 0, 0, 0, time.UTC)
	burst, refillMins := 5.0, 5.0

	// Zero tokensAt (no prior state) starts full.
	if got := refill(0, time.Time{}, base, burst, refillMins); got != burst {
		t.Errorf("fresh bucket = %v, want %v", got, burst)
	}
	// 5 minutes after empty → +1 token.
	if got := refill(0, base, base.Add(5*time.Minute), burst, refillMins); got != 1 {
		t.Errorf("refill after 5m = %v, want 1", got)
	}
	// 2.5 minutes → +0.5.
	if got := refill(0, base, base.Add(150*time.Second), burst, refillMins); got != 0.5 {
		t.Errorf("refill after 2.5m = %v, want 0.5", got)
	}
	// Refill is capped at burst.
	if got := refill(4, base, base.Add(time.Hour), burst, refillMins); got != burst {
		t.Errorf("refill cap = %v, want %v", got, burst)
	}
	// Clock moving backwards never grants free tokens.
	if got := refill(2, base, base.Add(-time.Hour), burst, refillMins); got != 2 {
		t.Errorf("backwards clock = %v, want 2", got)
	}
}

func TestQuietHoursParseAndContains(t *testing.T) {
	// Empty is disabled.
	q, err := parseQuietHours("")
	if err != nil || q.enabled {
		t.Errorf("empty should be disabled, got %+v err %v", q, err)
	}
	if q.contains(time.Date(2026, 7, 7, 3, 0, 0, 0, time.UTC)) {
		t.Error("disabled window contains nothing")
	}

	// Invalid is a loud error.
	for _, bad := range []string{"23:00", "9-10", "25:00-08:00", "23:00-08:70"} {
		if _, err := parseQuietHours(bad); err == nil {
			t.Errorf("parseQuietHours(%q) should error", bad)
		}
	}

	// Midnight-spanning window 23:00-08:00.
	q, err = parseQuietHours("23:00-08:00")
	if err != nil {
		t.Fatal(err)
	}
	at := func(h, m int) time.Time { return time.Date(2026, 7, 7, h, m, 0, 0, time.UTC) }
	inside := []time.Time{at(23, 0), at(23, 30), at(0, 0), at(3, 0), at(7, 59)}
	outside := []time.Time{at(8, 0), at(8, 1), at(12, 0), at(22, 59)}
	for _, ts := range inside {
		if !q.contains(ts) {
			t.Errorf("%s should be inside 23:00-08:00", ts.Format("15:04"))
		}
	}
	for _, ts := range outside {
		if q.contains(ts) {
			t.Errorf("%s should be outside 23:00-08:00", ts.Format("15:04"))
		}
	}
	if q.endLabel() != "08:00" {
		t.Errorf("endLabel = %q, want 08:00", q.endLabel())
	}

	// Same-day window 01:00-02:00, boundaries: start inclusive, end exclusive.
	q, _ = parseQuietHours("01:00-02:00")
	if !q.contains(at(1, 0)) || q.contains(at(2, 0)) || q.contains(at(0, 59)) {
		t.Error("same-day boundary handling wrong")
	}

	// Zero-width window is never active.
	q, _ = parseQuietHours("08:00-08:00")
	if q.contains(at(8, 0)) {
		t.Error("zero-width window should never be active")
	}
}
