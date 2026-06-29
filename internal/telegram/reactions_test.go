package telegram

import (
	"testing"

	"github.com/PaulSonOfLars/gotgbot/v2"
)

func emojiReactions(es ...string) []gotgbot.ReactionType {
	out := make([]gotgbot.ReactionType, len(es))
	for i, e := range es {
		out[i] = gotgbot.ReactionTypeEmoji{Emoji: e}
	}
	return out
}

func TestReactionEmojisIgnoresNonEmoji(t *testing.T) {
	rs := []gotgbot.ReactionType{
		gotgbot.ReactionTypeEmoji{Emoji: "🔥"},
		gotgbot.ReactionTypeCustomEmoji{CustomEmojiId: "123"}, // not surfaced
	}
	got := reactionEmojis(rs)
	if len(got) != 1 || got[0] != "🔥" {
		t.Fatalf("got %v, want [🔥]", got)
	}
}

func TestReactionDiff(t *testing.T) {
	cases := []struct {
		name             string
		old, new         []gotgbot.ReactionType
		wantAdd, wantRem []string
	}{
		{"add from none", nil, emojiReactions("🔥"), []string{"🔥"}, nil},
		{"remove to none", emojiReactions("👍"), nil, nil, []string{"👍"}},
		{"swap", emojiReactions("👍"), emojiReactions("🔥"), []string{"🔥"}, []string{"👍"}},
		{"no change", emojiReactions("🔥"), emojiReactions("🔥"), nil, nil},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			add, rem := reactionDiff(c.old, c.new)
			if !eqStrings(add, c.wantAdd) {
				t.Errorf("added = %v, want %v", add, c.wantAdd)
			}
			if !eqStrings(rem, c.wantRem) {
				t.Errorf("removed = %v, want %v", rem, c.wantRem)
			}
		})
	}
}

// TestReactionBoxedAsValue locks the gotgbot contract that handleReaction relies
// on: an emoji reaction is boxed into the ReactionType interface as a
// ReactionTypeEmoji *value*, so reactionEmojis must assert the value type.
func TestReactionBoxedAsValue(t *testing.T) {
	var rt gotgbot.ReactionType = gotgbot.ReactionTypeEmoji{Emoji: "👍"}
	if _, ok := rt.(*gotgbot.ReactionTypeEmoji); ok {
		t.Fatal("pointer assertion unexpectedly succeeded — gotgbot boxing changed")
	}
	if _, ok := rt.(gotgbot.ReactionTypeEmoji); !ok {
		t.Fatal("value assertion must succeed")
	}
}

func TestReplyQuote(t *testing.T) {
	// Plain text passes through trimmed.
	if got := replyQuote("  hi there  "); got != "hi there" {
		t.Errorf("plain = %q", got)
	}
	// Meta-breakout chars and quotes are neutralized; newlines collapse.
	got := replyQuote("a<b>\nc\"d;e")
	for _, bad := range []string{"<", ">", "\n", "\"", ";"} {
		if containsStr(got, bad) {
			t.Errorf("sanitized %q still contains %q", got, bad)
		}
	}
	// Long text is clamped with an ellipsis.
	long := make([]rune, 300)
	for i := range long {
		long[i] = 'x'
	}
	out := replyQuote(string(long))
	if r := []rune(out); len(r) != 201 || r[200] != '…' {
		t.Errorf("clamp length = %d, last = %q", len(r), string(r[len(r)-1]))
	}
	if got := replyQuote("   "); got != "" {
		t.Errorf("blank should be empty, got %q", got)
	}
}

func eqStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func containsStr(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
