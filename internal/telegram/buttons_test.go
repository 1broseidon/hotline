package telegram

import (
	"testing"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"example.com/tele-go/internal/access"
)

func TestSanitizeButtons(t *testing.T) {
	got := sanitizeButtons([]string{" ship it ", "", "  ", "not yet"})
	want := []string{"ship it", "not yet"}
	if len(got) != len(want) {
		t.Fatalf("got %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("got[%d] = %q, want %q", i, got[i], want[i])
		}
	}

	// Count is capped at maxButtons.
	many := make([]string, maxButtons+5)
	for i := range many {
		many[i] = "opt"
	}
	if n := len(sanitizeButtons(many)); n != maxButtons {
		t.Fatalf("capped count = %d, want %d", n, maxButtons)
	}

	// All-blank yields nothing (so the caller treats it as no buttons).
	if n := len(sanitizeButtons([]string{"", "   "})); n != 0 {
		t.Fatalf("all-blank count = %d, want 0", n)
	}
}

func TestButtonKeyboardLayout(t *testing.T) {
	kb := buttonKeyboard([]string{"yes", "no"})
	if len(kb.InlineKeyboard) != 2 {
		t.Fatalf("want one row per button, got %d rows", len(kb.InlineKeyboard))
	}
	if kb.InlineKeyboard[0][0].Text != "yes" || kb.InlineKeyboard[0][0].CallbackData != "act:0" {
		t.Errorf("row 0 = %+v", kb.InlineKeyboard[0][0])
	}
	if kb.InlineKeyboard[1][0].CallbackData != "act:1" {
		t.Errorf("row 1 callback = %q, want act:1", kb.InlineKeyboard[1][0].CallbackData)
	}
}

func TestButtonLabelRoundTrip(t *testing.T) {
	kb := buttonKeyboard([]string{"ship it", "not yet"})
	if got := buttonLabel(&kb, "act:1"); got != "not yet" {
		t.Errorf("act:1 -> %q, want \"not yet\"", got)
	}
	if got := buttonLabel(&kb, "act:0"); got != "ship it" {
		t.Errorf("act:0 -> %q, want \"ship it\"", got)
	}
	// Unknown data and a nil keyboard both yield "" (treated as already answered).
	if got := buttonLabel(&kb, "act:9"); got != "" {
		t.Errorf("unknown index -> %q, want empty", got)
	}
	if got := buttonLabel(nil, "act:0"); got != "" {
		t.Errorf("nil keyboard -> %q, want empty", got)
	}
}

func TestActBtnRe(t *testing.T) {
	for _, ok := range []string{"act:0", "act:11"} {
		if !actBtnRe.MatchString(ok) {
			t.Errorf("%q should match", ok)
		}
	}
	for _, bad := range []string{"perm:allow:abcde", "act:", "act:x", "xact:0", "act:0:1"} {
		if actBtnRe.MatchString(bad) {
			t.Errorf("%q should not match", bad)
		}
	}
}

func TestActorAllowed(t *testing.T) {
	acc := &access.Access{
		AllowFrom: []string{"100"},
		Groups: map[string]access.GroupPolicy{
			"-500": {},                          // open group: any member may answer
			"-600": {AllowFrom: []string{"77"}}, // restricted group
		},
	}

	// DM: only the allowlisted sender may answer.
	if !actorAllowed(acc, false, "100", "100") {
		t.Error("allowlisted DM sender should be allowed")
	}
	if actorAllowed(acc, false, "100", "999") {
		t.Error("non-allowlisted DM sender must be denied")
	}

	// Open group: configured but unrestricted -> any sender allowed.
	if !actorAllowed(acc, true, "-500", "12345") {
		t.Error("open group member should be allowed")
	}
	// Restricted group: only listed senders.
	if !actorAllowed(acc, true, "-600", "77") {
		t.Error("listed group sender should be allowed")
	}
	if actorAllowed(acc, true, "-600", "78") {
		t.Error("unlisted group sender must be denied")
	}
	// Unconfigured group: denied.
	if actorAllowed(acc, true, "-999", "100") {
		t.Error("unconfigured group must be denied")
	}
}

// TestCallbackMessageBoxedAsValue locks the gotgbot contract that
// handleActionCallback relies on: an accessible callback message is boxed into
// the MaybeInaccessibleMessage interface as a Message *value*, so it must be
// type-asserted as gotgbot.Message, never *gotgbot.Message. A pointer assertion
// silently fails and sends every tap to the "expired" branch.
func TestCallbackMessageBoxedAsValue(t *testing.T) {
	var mim gotgbot.MaybeInaccessibleMessage = gotgbot.Message{MessageId: 7}
	if _, ok := mim.(*gotgbot.Message); ok {
		t.Fatal("pointer assertion unexpectedly succeeded — gotgbot boxing changed")
	}
	msg, ok := mim.(gotgbot.Message)
	if !ok {
		t.Fatal("value assertion must succeed")
	}
	if msg.MessageId != 7 {
		t.Fatalf("MessageId = %d, want 7", msg.MessageId)
	}
}

// compile-time check that an empty button set produces a usable (empty) keyboard
// rather than panicking.
func TestButtonKeyboardEmpty(t *testing.T) {
	kb := buttonKeyboard(nil)
	if len(kb.InlineKeyboard) != 0 {
		t.Fatalf("empty input -> %d rows, want 0", len(kb.InlineKeyboard))
	}
	var _ gotgbot.InlineKeyboardMarkup = kb
}
