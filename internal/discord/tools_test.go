package discord

import (
	"context"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

// allowChat records chatID as an allowlisted user's DM channel so the outbound
// gate accepts it.
func allowChat(t *testing.T, tools *Tools, chatID, userID string) {
	t.Helper()
	recordDMChannel(tools.Cfg.StateDir, chatID, userID)
}

func TestReplyBubblesWithButtons(t *testing.T) {
	h, tools, fs, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"u1"}
		a.BubbleMode = "instant" // no pacing in tests
	})
	_ = h
	allowChat(t, tools, "c", "u1")

	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID:  "c",
		Bubbles: []string{"one", "", "two?"},
		Buttons: []string{"yes", "no"},
	})
	if isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if len(fs.Sent) != 2 {
		t.Fatalf("sent %d messages", len(fs.Sent))
	}
	if fs.Sent[0].Data.Content != "one" || fs.Sent[1].Data.Content != "two?" {
		t.Fatalf("contents %q %q", fs.Sent[0].Data.Content, fs.Sent[1].Data.Content)
	}
	// Buttons ride only on the last bubble.
	if len(fs.Sent[0].Data.Components) != 0 {
		t.Fatal("buttons on first bubble")
	}
	comps := fs.Sent[1].Data.Components
	if buttonLabel(comps, "act:0") != "yes" || buttonLabel(comps, "act:1") != "no" {
		t.Fatalf("components %+v", comps)
	}
	if !strings.Contains(out, "sent 2 parts") {
		t.Fatalf("summary %q", out)
	}
}

func TestReplyPacedBubblesSendTyping(t *testing.T) {
	_, tools, fs, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} }) // default paced
	allowChat(t, tools, "c", "u1")
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{
		ChatID:  "c",
		Bubbles: []string{"a", "b"},
	}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if len(fs.Typing) != 1 || fs.Typing[0] != "c" {
		t.Fatalf("typing calls %v", fs.Typing)
	}
}

func TestReplySplitsLongTextAt2000(t *testing.T) {
	_, tools, fs, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	allowChat(t, tools, "c", "u1")
	long := strings.Repeat("a", 4500)
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "c", Text: long}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if len(fs.Sent) != 3 {
		t.Fatalf("want 3 chunks, got %d", len(fs.Sent))
	}
	for i, m := range fs.Sent {
		if n := len([]rune(m.Data.Content)); n > MaxChunkLimit {
			t.Fatalf("chunk %d is %d runes", i, n)
		}
	}
}

func TestReplyRefusesUnallowedChannel(t *testing.T) {
	_, tools, fs, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "nope", Text: "hi"})
	if !isErr || !strings.Contains(out, "not allowlisted") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
	if len(fs.Sent) != 0 {
		t.Fatal("message sent to unallowed channel")
	}
}

func TestReplyAllowsConfiguredGuildChannel(t *testing.T) {
	_, tools, fs, _ := testEnv(t, func(a *access.Access) {
		a.Groups = map[string]access.GroupPolicy{"guildchan": {}}
		a.BubbleMode = "instant"
	})
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "guildchan", Text: "hi"}); isErr {
		t.Fatalf("reply errored: %s", out)
	}
	if len(fs.Sent) != 1 {
		t.Fatalf("sent %v", fs.Sent)
	}
}

func TestReplyNothingToSend(t *testing.T) {
	_, tools, _, _ := testEnv(t, nil)
	allowChat(t, tools, "c", "u1")
	// Channel gate runs first; make the channel pass by allowlisting the user.
	acc, _ := access.Load(tools.Cfg.AccessFile)
	acc.AllowFrom = []string{"u1"}
	_ = access.Save(acc, tools.Cfg.AccessFile)
	out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "c"})
	if !isErr || !strings.Contains(out, "nothing to send") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}

func TestReactAndEdit(t *testing.T) {
	_, tools, fs, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	allowChat(t, tools, "c", "u1")

	if out, isErr := tools.React(context.Background(), mcpchan.ReactInput{ChatID: "c", MessageID: "5", Emoji: "👍"}); isErr {
		t.Fatalf("react errored: %s", out)
	}
	if len(fs.Reactions) != 1 || fs.Reactions[0] != [3]string{"c", "5", "👍"} {
		t.Fatalf("reactions %v", fs.Reactions)
	}

	out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{ChatID: "c", MessageID: "5", Text: "fixed"})
	if isErr {
		t.Fatalf("edit errored: %s", out)
	}
	if len(fs.Edits) != 1 || *fs.Edits[0].Content != "fixed" || fs.Edits[0].ID != "5" || fs.Edits[0].Channel != "c" {
		t.Fatalf("edits %+v", fs.Edits)
	}
}

func TestEditRejectsOverlongText(t *testing.T) {
	_, tools, _, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	allowChat(t, tools, "c", "u1")
	out, isErr := tools.EditMessage(context.Background(), mcpchan.EditInput{
		ChatID: "c", MessageID: "5", Text: strings.Repeat("x", 2001),
	})
	if !isErr || !strings.Contains(out, "Discord max 2000") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}

func TestToolsWithoutSession(t *testing.T) {
	_, tools, _, _ := testEnv(t, nil)
	tools.Session = nil
	if out, isErr := tools.Reply(context.Background(), mcpchan.ReplyInput{ChatID: "c", Text: "x"}); !isErr || !strings.Contains(out, "no bot token") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
	if out, isErr := tools.DownloadAttachment(context.Background(), mcpchan.DownloadInput{FileID: "u"}); !isErr || !strings.Contains(out, "no bot token") {
		t.Fatalf("out=%q isErr=%v", out, isErr)
	}
}
