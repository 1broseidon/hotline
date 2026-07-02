package discord

import (
	"strings"
	"testing"
)

func TestChunkShortPassthrough(t *testing.T) {
	got := Chunk("hello", 2000, "newline")
	if len(got) != 1 || got[0] != "hello" {
		t.Fatalf("got %v", got)
	}
}

func TestChunkSplitsAt2000(t *testing.T) {
	s := strings.Repeat("a", 2500)
	got := Chunk(s, 5000, "hard") // limit above cap must clamp to 2000
	if len(got) != 2 {
		t.Fatalf("want 2 chunks, got %d", len(got))
	}
	if len([]rune(got[0])) != 2000 || len([]rune(got[1])) != 500 {
		t.Fatalf("chunk sizes %d/%d", len([]rune(got[0])), len([]rune(got[1])))
	}
}

func TestChunkPrefersNewline(t *testing.T) {
	s := strings.Repeat("x", 1500) + "\n\n" + strings.Repeat("y", 1500)
	got := Chunk(s, 2000, "newline")
	if len(got) != 2 {
		t.Fatalf("want 2 chunks, got %d", len(got))
	}
	if !strings.HasSuffix(got[0], "x") || !strings.HasPrefix(got[1], "y") {
		t.Fatalf("split not at paragraph break: %q | %q", got[0][len(got[0])-5:], got[1][:5])
	}
}

func TestChunkMultibyteSafe(t *testing.T) {
	s := strings.Repeat("é", 2100)
	for _, c := range Chunk(s, 2000, "hard") {
		for _, r := range c {
			if r != 'é' {
				t.Fatalf("rune corrupted: %q", r)
			}
		}
	}
}

func TestSafeName(t *testing.T) {
	if got := SafeName("a<b>[c]\r\n;d"); got != "a_b__c____d" {
		t.Fatalf("got %q", got)
	}
}

func TestBubbleDelayClamped(t *testing.T) {
	if d := bubbleDelay("x"); d != bubbleMinDelay {
		t.Fatalf("short bubble delay %v", d)
	}
	if d := bubbleDelay(strings.Repeat("x", 10000)); d != bubbleMaxDelay {
		t.Fatalf("long bubble delay %v", d)
	}
}
