package telegram

import (
	"strings"
	"testing"
	"time"
)

func TestBubbleDelayClampsAndScales(t *testing.T) {
	// Empty / short bubbles clamp to the floor.
	if d := bubbleDelay(""); d != bubbleMinDelay {
		t.Errorf("bubbleDelay(\"\") = %v, want floor %v", d, bubbleMinDelay)
	}
	if d := bubbleDelay("hi"); d != bubbleMinDelay {
		t.Errorf("bubbleDelay short = %v, want floor %v", d, bubbleMinDelay)
	}

	// A very long bubble clamps to the ceiling.
	if d := bubbleDelay(strings.Repeat("x", 1000)); d != bubbleMaxDelay {
		t.Errorf("bubbleDelay long = %v, want ceil %v", d, bubbleMaxDelay)
	}

	// A mid-length bubble scales between the bounds and is rune- not byte-based:
	// 40 emoji (4 bytes each in UTF-8) must scale by rune count, not byte count.
	mid := bubbleDelay(strings.Repeat("😀", 40))
	if mid <= bubbleMinDelay || mid >= bubbleMaxDelay {
		t.Errorf("bubbleDelay mid = %v, want strictly between %v and %v", mid, bubbleMinDelay, bubbleMaxDelay)
	}
	want := time.Duration(40*bubblePerCharMs) * time.Millisecond
	if mid != want {
		t.Errorf("bubbleDelay rune-scaled = %v, want %v (byte-scaled would be 4x)", mid, want)
	}
}

func TestCountNonBlank(t *testing.T) {
	cases := []struct {
		in   []string
		want int
	}{
		{nil, 0},
		{[]string{}, 0},
		{[]string{"", "  ", "\n\t"}, 0},
		{[]string{"a", "", "b"}, 2},
		{[]string{" x "}, 1},
	}
	for _, c := range cases {
		if got := countNonBlank(c.in); got != c.want {
			t.Errorf("countNonBlank(%q) = %d, want %d", c.in, got, c.want)
		}
	}
}
