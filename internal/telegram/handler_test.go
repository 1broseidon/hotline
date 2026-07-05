package telegram

import (
	"strings"
	"testing"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/mcpchan"
)

func TestPermPromptText(t *testing.T) {
	// Delegates to mcpchan.PermPromptText (canonical tests live there). Known tools
	// read as a warm ask that still surfaces the target; unknown tools keep 🔐 name.
	got := permPromptText(mcpchan.PermissionRequestParams{ToolName: "Bash", InputPreview: "ls"})
	if !strings.Contains(got, "run") || !strings.Contains(got, "ls") {
		t.Fatalf("expected humanized ask with target, got %q", got)
	}
	got = permPromptText(mcpchan.PermissionRequestParams{ToolName: "external_directory"})
	if !strings.Contains(got, "external_directory") {
		t.Fatalf("expected tool name for unknown tool, got %q", got)
	}
}

func botHandler(id int64, username string) *Handler {
	return &Handler{Bot: &gotgbot.Bot{User: gotgbot.User{Id: id, Username: username}}}
}

func TestIsMentionedByEntity(t *testing.T) {
	h := botHandler(100, "myclaudebot")
	// "@myclaudebot" starts at offset 6 ("hello @myclaudebot"), length 12.
	msg := &gotgbot.Message{
		Text: "hello @myclaudebot",
		Entities: []gotgbot.MessageEntity{
			{Type: "mention", Offset: 6, Length: 12},
		},
	}
	if !h.isMentioned(msg) {
		t.Fatal("expected mention to be detected by entity offset")
	}
}

func TestIsMentionedWrongUsername(t *testing.T) {
	h := botHandler(100, "myclaudebot")
	msg := &gotgbot.Message{
		Text:     "hi @someoneelse",
		Entities: []gotgbot.MessageEntity{{Type: "mention", Offset: 3, Length: 12}},
	}
	if h.isMentioned(msg) {
		t.Fatal("should not match a different @username")
	}
}

func TestIsMentionedUTF16Offsets(t *testing.T) {
	h := botHandler(100, "bot")
	// A leading emoji is 2 UTF-16 code units; the @mention starts after it.
	// "😀 @bot" -> units: [😀(2)][space(1)] then "@bot" at offset 3, length 4.
	msg := &gotgbot.Message{
		Text:     "😀 @bot",
		Entities: []gotgbot.MessageEntity{{Type: "mention", Offset: 3, Length: 4}},
	}
	if !h.isMentioned(msg) {
		t.Fatal("UTF-16 offset mention not detected")
	}
}

func TestIsMentionedTextMention(t *testing.T) {
	h := botHandler(100, "bot")
	// Real text_mention entities carry the referenced User but (for usernamed
	// users like a bot) an empty Username, so the bot must be matched by Id.
	msg := &gotgbot.Message{
		Text: "thanks bot",
		Entities: []gotgbot.MessageEntity{
			{Type: "text_mention", Offset: 7, Length: 3, User: &gotgbot.User{Id: 100}},
		},
	}
	if !h.isMentioned(msg) {
		t.Fatal("text_mention referencing the bot should match")
	}
}

func TestIsMentionedTextMentionOther(t *testing.T) {
	h := botHandler(100, "bot")
	// A text_mention of a different user must not match the bot.
	msg := &gotgbot.Message{
		Text: "thanks pal",
		Entities: []gotgbot.MessageEntity{
			{Type: "text_mention", Offset: 7, Length: 3, User: &gotgbot.User{Id: 200}},
		},
	}
	if h.isMentioned(msg) {
		t.Fatal("text_mention of another user must not match the bot")
	}
}

func TestIsMentionedCaption(t *testing.T) {
	h := botHandler(100, "bot")
	// Mention lives in the caption (e.g. a photo), not Text.
	msg := &gotgbot.Message{
		Caption:         "@bot look",
		CaptionEntities: []gotgbot.MessageEntity{{Type: "mention", Offset: 0, Length: 4}},
	}
	if !h.isMentioned(msg) {
		t.Fatal("mention in caption should match")
	}
}

func TestIsMentionedEmptyUsername(t *testing.T) {
	h := botHandler(100, "") // bot with no username can't be @-mentioned
	msg := &gotgbot.Message{
		Text:     "@x",
		Entities: []gotgbot.MessageEntity{{Type: "mention", Offset: 0, Length: 2}},
	}
	if h.isMentioned(msg) {
		t.Fatal("no username -> never mentioned")
	}
}

func TestRepliedToBot(t *testing.T) {
	h := botHandler(100, "bot")
	yes := &gotgbot.Message{ReplyToMessage: &gotgbot.Message{From: &gotgbot.User{Id: 100}}}
	if !h.repliedToBot(yes) {
		t.Fatal("reply to the bot's own message should be detected")
	}
	no := &gotgbot.Message{ReplyToMessage: &gotgbot.Message{From: &gotgbot.User{Id: 999}}}
	if h.repliedToBot(no) {
		t.Fatal("reply to someone else is not a bot reply")
	}
	if h.repliedToBot(&gotgbot.Message{}) {
		t.Fatal("no reply target -> false")
	}
}

func TestMessageText(t *testing.T) {
	if got := messageText(&gotgbot.Message{Text: "hi"}); got != "hi" {
		t.Fatalf("text = %q", got)
	}
	if got := messageText(&gotgbot.Message{Caption: "cap"}); got != "cap" {
		t.Fatalf("caption fallback = %q", got)
	}
	if got := messageText(&gotgbot.Message{}); got != "" {
		t.Fatalf("empty = %q", got)
	}
}

func TestUserDisplay(t *testing.T) {
	if got := userDisplay(&gotgbot.User{Username: "george", Id: 5}); got != "george" {
		t.Fatalf("username preferred, got %q", got)
	}
	if got := userDisplay(&gotgbot.User{Id: 5}); got != "5" {
		t.Fatalf("id fallback, got %q", got)
	}
}

func TestIsAllowlisted(t *testing.T) {
	a := &access.Access{AllowFrom: []string{"1", "2"}}
	if !isAllowlisted(a, "2") {
		t.Fatal("expected allowlisted")
	}
	if isAllowlisted(a, "3") {
		t.Fatal("unexpected allowlist hit")
	}
}

func TestPairingInstruction(t *testing.T) {
	// Default bot: bare command.
	if got := pairingInstruction("", "abc123"); got != "hotline pair abc123" {
		t.Errorf("default bot = %q", got)
	}
	// Named bot: must carry --bot so the code is approved against the right
	// bot's access.json, not the default's.
	if got := pairingInstruction("Ada", "abc123"); got != "hotline pair abc123 --bot Ada" {
		t.Errorf("named bot = %q", got)
	}
}

func TestUTF16SliceBounds(t *testing.T) {
	units := utf16Units("hello")
	if got := utf16Slice(units, 1, 3); got != "ell" {
		t.Fatalf("slice = %q", got)
	}
	// Out-of-range offsets return "" rather than panicking.
	if got := utf16Slice(units, 4, 10); got != "" {
		t.Fatalf("expected empty for OOB, got %q", got)
	}
	if got := utf16Slice(units, -1, 2); got != "" {
		t.Fatalf("expected empty for negative offset, got %q", got)
	}
}
