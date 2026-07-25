package app

import (
	"strings"
	"testing"
	"unicode/utf8"
)

func TestPushPreview(t *testing.T) {
	cases := []struct {
		name    string
		payload string
		want    string
	}{
		{"plain text", `{"t":"msg","id":"a-1","text":"hey there"}`, "hey there"},
		// E6/E10: element messages carry synthesized text — the preview IS the
		// text, no special casing.
		{"element message previews its synthesized text", `{"t":"msg","id":"a-1","text":"header pass: running","elements":[{"el":"job","id":"el-b1","fallback":"header pass: running"}]}`, "header pass: running"},
		{"nothing → default", `{"t":"msg","id":"a-1","text":""}`, "New message"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := pushPreview([]byte(tc.payload)); got != tc.want {
				t.Errorf("pushPreview = %q, want %q", got, tc.want)
			}
		})
	}
}

func TestPushPreviewTruncates(t *testing.T) {
	long := strings.Repeat("x", 400)
	payload := `{"t":"msg","id":"a-1","text":"` + long + `"}`
	got := pushPreview([]byte(payload))
	if n := utf8.RuneCountInString(got); n > pushBodyMax+1 { // +1 for the ellipsis rune
		t.Errorf("preview rune count %d exceeds pushBodyMax %d (+ellipsis)", n, pushBodyMax)
	}
	if !strings.HasSuffix(got, "…") {
		t.Errorf("long preview should be truncated with an ellipsis: %q", got)
	}
}

func TestBoundedNotificationTruncatesTitleAndBodyRuneSafe(t *testing.T) {
	got := boundedNotification(pushIntent{
		Title: "  " + strings.Repeat("界", pushTitleMax+20) + "  ",
		Body:  "  " + strings.Repeat("🎉", pushBodyMax+20) + "  ",
	})
	if !utf8.ValidString(got.Title) || !utf8.ValidString(got.Body) {
		t.Fatalf("bounded notification split UTF-8: %+v", got)
	}
	if n := utf8.RuneCountInString(got.Title); n != pushTitleMax+1 {
		t.Fatalf("title rune count = %d, want %d including ellipsis", n, pushTitleMax+1)
	}
	if n := utf8.RuneCountInString(got.Body); n != pushBodyMax+1 {
		t.Fatalf("body rune count = %d, want %d including ellipsis", n, pushBodyMax+1)
	}
	if !strings.HasSuffix(got.Title, "…") || !strings.HasSuffix(got.Body, "…") {
		t.Fatalf("bounded notification missing ellipsis: %+v", got)
	}
}

// TestPushEligibleRule pins ERRATA E10 at the discriminator itself.
func TestPushEligibleRule(t *testing.T) {
	synthEdit := `{"t":"edit","seq":5,"id":"a-4","text":"job x: done","elements":[{"el":"job","id":"el-1","fallback":"job x: done"}]}`
	cases := []struct {
		name    string
		payload string
		want    bool
	}{
		{"plain msg", `{"t":"msg","seq":1,"id":"a-1","text":"hi"}`, true},
		{"element-only msg (synthesized text)", `{"t":"msg","seq":2,"id":"a-2","text":"job x: running","elements":[{"el":"job","id":"el-1","fallback":"job x: running"}]}`, true},
		{"react", `{"t":"react","seq":3,"msg_id":"a-1","emoji":"👍"}`, true},
		{"text edit", `{"t":"edit","seq":4,"id":"a-1","text":"fresh words"}`, true},
		{"empty-text edit", `{"t":"edit","seq":5,"id":"a-1","text":""}`, false},
		{"element-only edit (synthesized text)", synthEdit, false},
		{"edit with real text AND elements", `{"t":"edit","seq":6,"id":"a-4","text":"custom words","elements":[{"el":"job","id":"el-1","fallback":"job x: done"}]}`, true},
		{"typing", `{"t":"typing","seq":7,"on":true}`, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := pushEligible([]byte(tc.payload)); got != tc.want {
				t.Errorf("pushEligible = %v, want %v", got, tc.want)
			}
		})
	}
}
