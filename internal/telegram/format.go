package telegram

import (
	"html"
	"regexp"
	"strings"
)

// MaxChunkLimit is Telegram's hard per-message character cap.
const MaxChunkLimit = 4096

// PhotoExts are the extensions sent inline as photos; everything else goes as
// a document.
var PhotoExts = map[string]bool{
	".jpg":  true,
	".jpeg": true,
	".png":  true,
	".gif":  true,
	".webp": true,
}

// unsafeNameRe matches characters that would let an uploader-controlled
// filename break out of the <channel> meta block.
var unsafeNameRe = regexp.MustCompile(`[<>\[\]\r\n;]`)

// SafeName replaces delimiter characters in an uploader-controlled name so it
// can't forge or escape the inbound notification meta.
func SafeName(s string) string {
	if s == "" {
		return ""
	}
	return unsafeNameRe.ReplaceAllString(s, "_")
}

// Chunk splits text into pieces no longer than limit runes. limit is clamped
// to 1..MaxChunkLimit. In "newline" mode the split prefers the last paragraph
// break, then the last line break, then the last space within the window,
// falling back to a hard cut. Operates on runes so multibyte text isn't split
// mid-character.
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
		// Drop leading newlines left at the boundary.
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
// of sep in s, or -1. Used so cut offsets line up with the rune slice.
func lastIndexRune(s, sep string) int {
	b := strings.LastIndex(s, sep)
	if b < 0 {
		return -1
	}
	return len([]rune(s[:b]))
}

// EscapeHTML escapes text for Telegram's HTML parse mode.
func EscapeHTML(s string) string {
	return html.EscapeString(s)
}

var markdownV2Specials = "_*[]()~`>#+-=|{}.!"

// EscapeMarkdownV2 backslash-escapes every MarkdownV2 special character.
func EscapeMarkdownV2(s string) string {
	var b strings.Builder
	b.Grow(len(s) * 2)
	for _, r := range s {
		if strings.ContainsRune(markdownV2Specials, r) {
			b.WriteByte('\\')
		}
		b.WriteRune(r)
	}
	return b.String()
}

// parseModeFor maps a tool "format" value to the Telegram parse_mode string.
// An empty string means no parse mode (plain text).
func parseModeFor(format string) string {
	switch strings.ToLower(format) {
	case "markdownv2":
		return "MarkdownV2"
	case "html":
		return "HTML"
	default:
		return ""
	}
}
