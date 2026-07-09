package discord

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/bwmarrin/discordgo"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

func inboundMsg(id, channel, sender, text string) *discordgo.Message {
	return &discordgo.Message{
		ID:        id,
		ChannelID: channel,
		Content:   text,
		Author:    &discordgo.User{ID: sender, Username: "george"},
		Timestamp: time.Date(2026, 7, 2, 10, 0, 0, 0, time.UTC),
	}
}

// deliver synchronously captures coalesced bursts.
func captureBursts(h *Handler) *[][]pendingMsg {
	var bursts [][]pendingMsg
	h.coalDeliver = func(_ context.Context, msgs []pendingMsg) {
		bursts = append(bursts, msgs)
	}
	return &bursts
}

func TestHandleMessageNormalization(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})
	bursts := captureBursts(h)

	m := inboundMsg("42", "chan9", "u1", "hello there.")
	m.ReferencedMessage = &discordgo.Message{
		ID:      "40",
		Content: "earlier text",
		Author:  &discordgo.User{ID: "bot1"},
	}
	h.HandleMessage(context.Background(), m)
	h.FlushAll(context.Background()) // complete message now takes the grace hold; drain it

	if len(*bursts) != 1 || len((*bursts)[0]) != 1 {
		t.Fatalf("bursts %v", *bursts)
	}
	got := (*bursts)[0][0]
	if got.content != "hello there." {
		t.Fatalf("content %q", got.content)
	}
	meta := got.meta
	for k, want := range map[string]string{
		"chat_id":             "chan9",
		"message_id":          "42",
		"user":                "george",
		"user_id":             "u1",
		"ts":                  "2026-07-02T10:00:00.000Z",
		"reply_to_message_id": "40",
		"reply_to_from":       "you",
		"reply_to_text":       "earlier text",
	} {
		if meta[k] != want {
			t.Errorf("meta[%s] = %q, want %q", k, meta[k], want)
		}
	}
}

func TestHandleMessageDropsUnknownSenderUnderAllowlist(t *testing.T) {
	h, _, fs, _ := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})
	bursts := captureBursts(h)
	h.HandleMessage(context.Background(), inboundMsg("1", "c", "stranger", "hi."))
	if len(*bursts) != 0 || len(fs.Sent) != 0 {
		t.Fatalf("unknown sender not dropped: %v %v", *bursts, fs.Sent)
	}
}

func TestHandleMessageDropsBots(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	bursts := captureBursts(h)
	m := inboundMsg("1", "c", "u1", "beep.")
	m.Author.Bot = true
	h.HandleMessage(context.Background(), m)
	if len(*bursts) != 0 {
		t.Fatal("bot message relayed")
	}
}

func TestHandleMessagePairingReply(t *testing.T) {
	h, _, fs, _ := testEnv(t, nil) // default pairing policy, empty allowlist
	bursts := captureBursts(h)
	h.HandleMessage(context.Background(), inboundMsg("1", "dmchan", "newguy", "hi."))
	if len(*bursts) != 0 {
		t.Fatal("unpaired sender relayed")
	}
	if len(fs.Sent) != 1 || fs.Sent[0].ChannelID != "dmchan" {
		t.Fatalf("no pairing reply: %v", fs.Sent)
	}
	body := fs.Sent[0].Data.Content
	if want := "hotline pair "; !strings.Contains(body, want) || !strings.Contains(body, "--provider discord") {
		t.Fatalf("pairing body %q", body)
	}
	acc, _ := access.Load(h.Cfg.AccessFile)
	if len(acc.Pending) != 1 {
		t.Fatalf("pending %v", acc.Pending)
	}
}

func TestHandleMessageGuildGating(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) {
		a.AllowFrom = []string{"u1"}
		a.Groups = map[string]access.GroupPolicy{"guildchan": {RequireMention: true}}
	})
	bursts := captureBursts(h)

	m := inboundMsg("1", "guildchan", "u1", "no mention here.")
	m.GuildID = "g1"
	h.HandleMessage(context.Background(), m)
	if len(*bursts) != 0 {
		t.Fatal("unmentioned guild message relayed")
	}

	m2 := inboundMsg("2", "guildchan", "u1", "hey bot.")
	m2.GuildID = "g1"
	m2.Mentions = []*discordgo.User{{ID: "bot1"}}
	h.HandleMessage(context.Background(), m2)
	h.FlushAll(context.Background()) // complete message now takes the grace hold; drain it
	if len(*bursts) != 1 {
		t.Fatal("mentioned guild message not relayed")
	}
}

func TestHandleMessageAttachment(t *testing.T) {
	h, _, _, _ := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})
	bursts := captureBursts(h)
	m := inboundMsg("7", "c", "u1", "")
	m.Attachments = []*discordgo.MessageAttachment{{
		ID:          "att1",
		URL:         "https://cdn.discordapp.com/attachments/1/2/report.pdf",
		Filename:    "report.pdf",
		ContentType: "application/pdf",
		Size:        1234,
	}}
	h.HandleMessage(context.Background(), m)
	h.FlushAll(context.Background()) // synthetic content holds the window; drain it
	if len(*bursts) != 1 {
		t.Fatalf("bursts %v", *bursts)
	}
	got := (*bursts)[0][0]
	if got.content != "(document: report.pdf)" {
		t.Fatalf("content %q", got.content)
	}
	meta := got.meta
	if meta["attachment_kind"] != "document" ||
		meta["attachment_file_id"] != "https://cdn.discordapp.com/attachments/1/2/report.pdf" ||
		meta["attachment_name"] != "report.pdf" ||
		meta["attachment_mime"] != "application/pdf" ||
		meta["attachment_size"] != "1234" {
		t.Fatalf("meta %v", meta)
	}
}

func buttonInteraction(customID, channelID, userID string, msg *discordgo.Message) *discordgo.Interaction {
	return &discordgo.Interaction{
		Type:      discordgo.InteractionMessageComponent,
		ChannelID: channelID,
		User:      &discordgo.User{ID: userID, Username: "george"},
		Message:   msg,
		Data: discordgo.MessageComponentInteractionData{
			CustomID:      customID,
			ComponentType: discordgo.ButtonComponent,
		},
	}
}

func TestActionInteractionRelaysChoice(t *testing.T) {
	h, _, fs, sink := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	msg := &discordgo.Message{
		ID:        "q1",
		ChannelID: "c",
		Content:   "ship it?",
		Components: []discordgo.MessageComponent{
			&discordgo.ActionsRow{Components: []discordgo.MessageComponent{
				&discordgo.Button{Label: "ship it", CustomID: "act:0"},
				&discordgo.Button{Label: "not yet", CustomID: "act:1"},
			}},
		},
	}
	h.HandleInteraction(context.Background(), buttonInteraction("act:1", "c", "u1", msg))

	// The carrying message is rewritten with components cleared.
	if len(fs.Responses) != 1 {
		t.Fatalf("responses %v", fs.Responses)
	}
	resp := fs.Responses[0]
	if resp.Type != discordgo.InteractionResponseUpdateMessage {
		t.Fatalf("response type %v", resp.Type)
	}
	if !strings.Contains(resp.Data.Content, "→ not yet") || len(resp.Data.Components) != 0 {
		t.Fatalf("response data %+v", resp.Data)
	}

	// The choice reaches Claude as an ordinary inbound with kind=button.
	if len(sink.Contents) != 1 || sink.Contents[0] != "not yet" {
		t.Fatalf("sink %v", sink.Contents)
	}
	meta := sink.Metas[0]
	if meta["kind"] != "button" || meta["chat_id"] != "c" || meta["user_id"] != "u1" {
		t.Fatalf("meta %v", meta)
	}
}

func TestActionInteractionUnauthorized(t *testing.T) {
	h, _, fs, sink := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })
	msg := &discordgo.Message{ID: "q1", ChannelID: "c", Content: "?", Components: buttonComponents([]string{"a"})}
	h.HandleInteraction(context.Background(), buttonInteraction("act:0", "c", "stranger", msg))
	if len(sink.Contents) != 0 {
		t.Fatal("unauthorized choice relayed")
	}
	if len(fs.Responses) != 1 || fs.Responses[0].Data == nil || fs.Responses[0].Data.Content != "Not authorized." {
		t.Fatalf("responses %+v", fs.Responses)
	}
}

func TestPermissionRelayFlow(t *testing.T) {
	h, _, fs, sink := testEnv(t, func(a *access.Access) { a.AllowFrom = []string{"u1"} })

	h.OnPermissionRequest(context.Background(), mcpchan.PermissionRequestParams{
		RequestID:   "abcde",
		ToolName:    "Bash",
		Description: "run a command",
	})

	// Prompt is DM'd to the allowlisted user with allow/deny/more buttons.
	if len(fs.DMOpened) != 1 || fs.DMOpened[0] != "u1" {
		t.Fatalf("DM opened %v", fs.DMOpened)
	}
	if len(fs.Sent) != 1 || fs.Sent[0].ChannelID != "dm-u1" {
		t.Fatalf("sent %v", fs.Sent)
	}
	comps := fs.Sent[0].Data.Components
	if buttonLabel(comps, "perm:allow:abcde") == "" || buttonLabel(comps, "perm:deny:abcde") == "" {
		t.Fatalf("perm buttons missing: %+v", comps)
	}

	// Clicking allow claims the request and emits the verdict.
	msg := &discordgo.Message{ID: "p1", ChannelID: "dm-u1", Content: "🔐 Permission: Bash", Components: comps}
	h.HandleInteraction(context.Background(), buttonInteraction("perm:allow:abcde", "dm-u1", "u1", msg))
	if len(sink.Verdicts) != 1 || sink.Verdicts[0] != [2]string{"abcde", "allow"} {
		t.Fatalf("verdicts %v", sink.Verdicts)
	}

	// A second click finds the claim gone.
	h.HandleInteraction(context.Background(), buttonInteraction("perm:deny:abcde", "dm-u1", "u1", msg))
	if len(sink.Verdicts) != 1 {
		t.Fatalf("duplicate verdict: %v", sink.Verdicts)
	}
}

func TestPermissionTextReply(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})
	h.OnPermissionRequest(context.Background(), mcpchan.PermissionRequestParams{RequestID: "abcde", ToolName: "Bash"})

	h.HandleMessage(context.Background(), inboundMsg("9", "c", "u1", "no abcde"))
	if len(sink.Verdicts) != 1 || sink.Verdicts[0] != [2]string{"abcde", "deny"} {
		t.Fatalf("verdicts %v", sink.Verdicts)
	}
	// The text reply must not also be relayed as a channel message.
	if len(sink.Contents) != 0 {
		t.Fatalf("perm reply leaked to channel: %v", sink.Contents)
	}
}

func TestCoalesceBurst(t *testing.T) {
	h, _, _, sink := testEnv(t, func(a *access.Access) {
		a.DMPolicy = "allowlist"
		a.AllowFrom = []string{"u1"}
	})
	h.coalesceWindow = 30 * time.Millisecond

	h.HandleMessage(context.Background(), inboundMsg("1", "c", "u1", "ok so"))
	h.HandleMessage(context.Background(), inboundMsg("2", "c", "u1", "the auth thing"))
	time.Sleep(150 * time.Millisecond)

	sink.mu.Lock()
	defer sink.mu.Unlock()
	if len(sink.Contents) != 1 {
		t.Fatalf("want 1 coalesced delivery, got %v", sink.Contents)
	}
	if sink.Contents[0] != "ok so\nthe auth thing" {
		t.Fatalf("content %q", sink.Contents[0])
	}
	if sink.Metas[0]["bubbles"] != "2" || sink.Metas[0]["message_id"] != "2" {
		t.Fatalf("meta %v", sink.Metas[0])
	}
}

func TestPermPromptText(t *testing.T) {
	// Delegates to mcpchan.PermPromptText (canonical tests live there).
	got := permPromptText(mcpchan.PermissionRequestParams{ToolName: "Bash", InputPreview: "ls"})
	if !strings.Contains(got, "run") || !strings.Contains(got, "ls") {
		t.Fatalf("expected humanized ask with target, got %q", got)
	}
	got = permPromptText(mcpchan.PermissionRequestParams{ToolName: "external_directory"})
	if !strings.Contains(got, "external_directory") {
		t.Fatalf("expected tool name for unknown tool, got %q", got)
	}
}
