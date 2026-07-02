package discord

import (
	"strings"
	"testing"

	"github.com/bwmarrin/discordgo"
)

func TestDownloadToInboxRejectsNonCDNHosts(t *testing.T) {
	for _, u := range []string{
		"https://evil.example.com/x.png",
		"http://cdn.discordapp.com/x.png", // http, not https
		"https://cdn.discordapp.com.evil.com/x.png",
		"file:///etc/passwd",
		"https://169.254.169.254/latest/meta-data",
	} {
		if _, err := DownloadToInbox(t.TempDir(), u); err == nil {
			t.Errorf("accepted %q", u)
		} else if !strings.Contains(err.Error(), "refusing") && !strings.Contains(err.Error(), "invalid") {
			t.Errorf("unexpected error for %q: %v", u, err)
		}
	}
}

func TestExtractClassification(t *testing.T) {
	cases := []struct {
		att  discordgo.MessageAttachment
		kind string
	}{
		{discordgo.MessageAttachment{Filename: "a.png", ContentType: "image/png"}, "photo"},
		{discordgo.MessageAttachment{Filename: "a.mp3", ContentType: "audio/mpeg"}, "audio"},
		{discordgo.MessageAttachment{Filename: "v.ogg", ContentType: "audio/ogg", DurationSecs: 3.2, Waveform: "abc"}, "voice"},
		{discordgo.MessageAttachment{Filename: "a.mp4", ContentType: "video/mp4"}, "video"},
		{discordgo.MessageAttachment{Filename: "a.pdf", ContentType: "application/pdf"}, "document"},
		{discordgo.MessageAttachment{Filename: "a.bin"}, "document"},
	}
	for _, c := range cases {
		if got := extract(&c.att); got.Kind != c.kind {
			t.Errorf("%s: got kind %q, want %q", c.att.Filename, got.Kind, c.kind)
		}
	}
	if got := extract(&discordgo.MessageAttachment{Filename: "x<y>.png", ContentType: "image/png"}); got.Name != "x_y_.png" {
		t.Errorf("name not sanitized: %q", got.Name)
	}
}
