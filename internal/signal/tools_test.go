package signal

import (
	"context"
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

func TestReplyBubblesPacedWithTyping(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		// paced is the default BubbleMode
	})

	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID:  "+15550002222",
		Bubbles: []string{"one", "two", "three"},
	})
	if isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if got := len(d.callsFor("send")); got != 3 {
		t.Fatalf("sends %d", got)
	}
	// A typing indicator precedes every bubble after the first.
	if got := len(d.callsFor("sendTyping")); got != 2 {
		t.Fatalf("sendTyping calls %d, want 2", got)
	}
	if !strings.Contains(out, "3 parts") {
		t.Fatalf("out %q", out)
	}
}

func TestReplyInstantSkipsTyping(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		a.BubbleMode = "instant"
	})
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID: "+15550002222", Bubbles: []string{"one", "two"},
	}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if got := len(d.callsFor("sendTyping")); got != 0 {
		t.Fatalf("sendTyping calls %d, want 0", got)
	}
}

func TestReplyRefusesUnpairedChat(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "+15550007777", Text: "hi"})
	if !isErr || !strings.Contains(out, "not allowlisted") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
	if len(d.calls()) != 0 {
		t.Fatalf("calls %v", d.calls())
	}
}

func TestReplyAllowsConfiguredGroup(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.Groups = map[string]access.GroupPolicy{"group:g9==": {}}
	})
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "group:g9==", Text: "hi"}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if d.callsFor("send")[0].Params["groupId"] != "g9==" {
		t.Fatal("group send misaddressed")
	}
}

func TestReplyFilesSentAsAttachments(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
		a.BubbleMode = "instant"
	})
	f := filepath.Join(t.TempDir(), "pic.png")
	if err := os.WriteFile(f, []byte("png"), 0o600); err != nil {
		t.Fatal(err)
	}
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID: "+15550002222", Text: "here", Files: []string{f},
	}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	sends := d.callsFor("send")
	if len(sends) != 2 {
		t.Fatalf("sends %v", sends)
	}
	atts, ok := sends[1].Params["attachments"].([]any)
	if !ok || len(atts) != 1 || atts[0] != f {
		t.Fatalf("attachments %v", sends[1].Params["attachments"])
	}
}

func TestReplyRefusesStateFiles(t *testing.T) {
	_, tools, _, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID: "+15550002222", Text: "leak", Files: []string{tools.Cfg.AccessFile},
	})
	if !isErr || !strings.Contains(out, "refusing to send channel state") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}

func TestReactParsesInboundMessageID(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	out, isErr := tools.React(context.Background(), mcpchan.ReactInput{
		ChatID: "+15550002222", MessageID: "1700000000123:+15550002222", Emoji: "🔥",
	})
	if isErr {
		t.Fatalf("react errored: %s", out)
	}
	p := d.callsFor("sendReaction")[0].Params
	if p["targetAuthor"] != "+15550002222" || p["targetTimestamp"] != float64(1700000000123) {
		t.Fatalf("params %v", p)
	}

	// A bare timestamp addresses one of our own messages.
	if out, isErr := tools.React(context.Background(), mcpchan.ReactInput{
		ChatID: "+15550002222", MessageID: "1700000000555", Emoji: "👀",
	}); isErr {
		t.Fatalf("react errored: %s", out)
	}
	p = d.callsFor("sendReaction")[1].Params
	if p["targetAuthor"] != testAccount {
		t.Fatalf("own-message author %v", p["targetAuthor"])
	}
}

func TestEditMessageUsesEditTimestamp(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{
		ChatID: "+15550002222", MessageID: "1700000000555", Text: "fixed",
	})
	if isErr {
		t.Fatalf("edit errored: %s", out)
	}
	p := d.callsFor("send")[0].Params
	if p["editTimestamp"] != float64(1700000000555) || p["message"] != "fixed" {
		t.Fatalf("params %v", p)
	}
	if !strings.Contains(out, "edited (id: ") {
		t.Fatalf("out %q", out)
	}

	// Refuses to edit someone else's message.
	out, isErr = tools.EditMessage(context.Background(), mcpchan.EditInput{
		ChatID: "+15550002222", MessageID: "1700000000555:+15550002222", Text: "nope",
	})
	if !isErr || !strings.Contains(out, "messages the bot sent") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}

func TestDownloadAttachment(t *testing.T) {
	_, tools, d, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"+15550002222"}
	})
	d.Results["getAttachment"] = base64.StdEncoding.EncodeToString([]byte("FILEBYTES"))

	path, isErr := tools.DownloadAttachment(context.Background(), mcpchan.DownloadInput{
		FileID: "att9|+15550002222",
	})
	if isErr {
		t.Fatalf("download errored: %s", path)
	}
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "FILEBYTES" {
		t.Fatalf("saved %q err %v", data, err)
	}
	p := d.callsFor("getAttachment")[0].Params
	if p["id"] != "att9" {
		t.Fatalf("params %v", p)
	}
	rec, _ := p["recipient"].([]any)
	if len(rec) != 1 || rec[0] != "+15550002222" {
		t.Fatalf("recipient %v", p["recipient"])
	}
}

func TestToolsWithoutAccountReportCleanly(t *testing.T) {
	tools := NewTools(nil, nil, nil, newOptionStore())
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "x", Text: "hi"}); !isErr || !strings.Contains(out, "no signal account") {
		t.Fatalf("out %q", out)
	}
	if out, isErr := tools.React(context.Background(), mcpchan.ReactInput{}); !isErr || !strings.Contains(out, "no signal account") {
		t.Fatalf("out %q", out)
	}
	if out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{}); !isErr || !strings.Contains(out, "no signal account") {
		t.Fatalf("out %q", out)
	}
	if out, isErr := tools.DownloadAttachment(context.Background(), mcpchan.DownloadInput{}); !isErr || !strings.Contains(out, "no signal account") {
		t.Fatalf("out %q", out)
	}
}
