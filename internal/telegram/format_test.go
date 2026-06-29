package telegram

import (
	"strings"
	"testing"
)

func TestChunkShort(t *testing.T) {
	got := Chunk("hello", 4096, "length")
	if len(got) != 1 || got[0] != "hello" {
		t.Fatalf("got %v", got)
	}
}

func TestChunkLength(t *testing.T) {
	s := strings.Repeat("a", 10)
	got := Chunk(s, 4, "length")
	if len(got) != 3 {
		t.Fatalf("expected 3 chunks, got %d: %v", len(got), got)
	}
	for _, c := range got {
		if len([]rune(c)) > 4 {
			t.Fatalf("chunk too long: %q", c)
		}
	}
	if strings.Join(got, "") != s {
		t.Fatalf("reassembly mismatch: %q", strings.Join(got, ""))
	}
}

func TestChunkNewlinePrefersBoundary(t *testing.T) {
	s := "para one here\n\npara two is longer than the window allows"
	got := Chunk(s, 20, "newline")
	if got[0] != "para one here" {
		t.Fatalf("expected split at paragraph boundary, got first chunk %q", got[0])
	}
}

func TestChunkBoundaryExact(t *testing.T) {
	s := strings.Repeat("x", 4)
	got := Chunk(s, 4, "length")
	if len(got) != 1 {
		t.Fatalf("exactly-at-limit should be one chunk, got %d", len(got))
	}
}

func TestChunkMultibyte(t *testing.T) {
	// 6 multibyte runes, limit 2 -> 3 chunks, no rune split.
	s := "αβγδεζ"
	got := Chunk(s, 2, "length")
	if len(got) != 3 {
		t.Fatalf("expected 3 chunks, got %d", len(got))
	}
	if strings.Join(got, "") != s {
		t.Fatal("multibyte reassembly mismatch")
	}
}

func TestChunkClamp(t *testing.T) {
	s := strings.Repeat("a", 5000)
	got := Chunk(s, 100000, "length") // clamp to 4096
	if len([]rune(got[0])) > MaxChunkLimit {
		t.Fatalf("clamp failed, first chunk %d runes", len([]rune(got[0])))
	}
}

func TestSafeName(t *testing.T) {
	cases := map[string]string{
		"clean.png":      "clean.png",
		"a<b>c":          "a_b_c",
		"x[y]z":          "x_y_z",
		"line1\r\nline2": "line1__line2",
		"semi;colon":     "semi_colon",
		"":               "",
	}
	for in, want := range cases {
		if got := SafeName(in); got != want {
			t.Fatalf("SafeName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestEscapeMarkdownV2(t *testing.T) {
	got := EscapeMarkdownV2("a_b*c.")
	want := `a\_b\*c\.`
	if got != want {
		t.Fatalf("EscapeMarkdownV2 = %q, want %q", got, want)
	}
}

func TestEscapeHTML(t *testing.T) {
	got := EscapeHTML("<b>&\"")
	want := "&lt;b&gt;&amp;&#34;"
	if got != want {
		t.Fatalf("EscapeHTML = %q, want %q", got, want)
	}
}

func TestParseModeFor(t *testing.T) {
	cases := map[string]string{
		"markdownv2": "MarkdownV2",
		"MarkdownV2": "MarkdownV2",
		"html":       "HTML",
		"text":       "",
		"":           "",
		"bogus":      "",
	}
	for in, want := range cases {
		if got := parseModeFor(in); got != want {
			t.Fatalf("parseModeFor(%q) = %q, want %q", in, got, want)
		}
	}
}
