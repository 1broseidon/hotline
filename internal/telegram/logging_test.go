package telegram

import (
	"testing"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

func TestInboundKind(t *testing.T) {
	cases := []struct {
		meta map[string]string
		want string
	}{
		{map[string]string{"kind": "reaction"}, "reaction"},
		{map[string]string{"kind": "button"}, "button"},
		{map[string]string{"image_path": "/inbox/p.jpg"}, "photo"},
		{map[string]string{"attachment_kind": "voice"}, "voice"},
		{map[string]string{}, "text"},
		// explicit kind wins over media hints
		{map[string]string{"kind": "reaction", "image_path": "/x"}, "reaction"},
	}
	for _, c := range cases {
		if got := inboundKind(c.meta); got != c.want {
			t.Errorf("inboundKind(%v) = %q, want %q", c.meta, got, c.want)
		}
	}
}

func TestOutboundText(t *testing.T) {
	// Bubbles join by newline, blanks dropped.
	got := outboundText(mcpchan.ReplyInput{Bubbles: []string{"one", "  ", "two"}})
	if got != "one\ntwo" {
		t.Errorf("bubbles = %q, want \"one\\ntwo\"", got)
	}
	// Falls back to text when no bubbles.
	if got := outboundText(mcpchan.ReplyInput{Text: "solo"}); got != "solo" {
		t.Errorf("text = %q", got)
	}
	// Files are noted, appended after any text.
	got = outboundText(mcpchan.ReplyInput{Text: "see this", Files: []string{"/abs/report.pdf"}})
	if got != "see this\n[file: report.pdf]" {
		t.Errorf("with file = %q", got)
	}
	// File-only reply still records the attachment.
	if got := outboundText(mcpchan.ReplyInput{Files: []string{"/x/a.png"}}); got != "[file: a.png]" {
		t.Errorf("file-only = %q", got)
	}
}
