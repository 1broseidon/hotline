package telegram

import (
	"context"
	"fmt"
	"os"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/access"
)

// maxButtons caps how many inline buttons one reply can carry. Telegram permits
// far more, but a question with a dozen-plus options is a UX smell on a phone.
const maxButtons = 12

// actBtnRe matches an action-button callback payload: "act:<index>". The index
// identifies which option was tapped; the option's label is recovered from the
// message's own keyboard, so no server-side state is needed.
var actBtnRe = regexp.MustCompile(`^act:(\d+)$`)

// sanitizeButtons trims labels, drops blanks, and caps the count. The surviving
// order is preserved so callback indices line up with what was sent.
func sanitizeButtons(in []string) []string {
	out := make([]string, 0, len(in))
	for _, b := range in {
		if b = strings.TrimSpace(b); b == "" {
			continue
		}
		out = append(out, b)
		if len(out) == maxButtons {
			break
		}
	}
	return out
}

// buttonKeyboard lays out one button per row — always readable on a phone and
// never truncated mid-label. Callback data is "act:<i>"; the label (which is the
// value sent back to Claude) is read from this keyboard when the button is
// tapped, so the value never has to fit in Telegram's 64-byte callback_data cap.
func buttonKeyboard(labels []string) gotgbot.InlineKeyboardMarkup {
	rows := make([][]gotgbot.InlineKeyboardButton, 0, len(labels))
	for i, label := range labels {
		rows = append(rows, []gotgbot.InlineKeyboardButton{
			{Text: label, CallbackData: "act:" + strconv.Itoa(i)},
		})
	}
	return gotgbot.InlineKeyboardMarkup{InlineKeyboard: rows}
}

// buttonLabel returns the label of the button whose callback data matches data,
// or "" if the keyboard is gone or has no such button (e.g. already answered).
func buttonLabel(kb *gotgbot.InlineKeyboardMarkup, data string) string {
	if kb == nil {
		return ""
	}
	for _, row := range kb.InlineKeyboard {
		for _, btn := range row {
			if btn.CallbackData == data {
				return btn.Text
			}
		}
	}
	return ""
}

// actorAllowed reports whether the user who tapped a button may answer it. DMs
// require the sender to be allowlisted; groups require the group to be
// configured and, if it restricts senders, the sender to be listed. The mention
// requirement that gates inbound group messages does not apply here — a tap on a
// button the bot itself posted is already a direct, intentional response.
func actorAllowed(acc *access.Access, isGroup bool, chatID, senderID string) bool {
	if isGroup {
		g, ok := acc.Groups[chatID]
		if !ok {
			return false
		}
		return len(g.AllowFrom) == 0 || slices.Contains(g.AllowFrom, senderID)
	}
	return slices.Contains(acc.AllowFrom, senderID)
}

// handleActionCallback processes a tap on a reply's inline button. It recovers
// the chosen label from the message's keyboard, clears the keyboard so the
// question can't be answered twice, and relays the choice to Claude as an
// ordinary inbound message (content = the label).
func (h *Handler) handleActionCallback(ctx context.Context, cb *gotgbot.CallbackQuery) {
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, nil)
		return
	}

	// The message that carried the buttons. gotgbot boxes an accessible message
	// as a Message *value* (not a pointer) into the MaybeInaccessibleMessage
	// interface; an InaccessibleMessage (very old/deleted, no keyboard to read)
	// boxes as the other type, which we treat as expired.
	msg, ok := cb.Message.(gotgbot.Message)
	if !ok {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "This question expired."})
		return
	}

	chat := msg.Chat
	chatID := strconv.FormatInt(chat.Id, 10)
	senderID := strconv.FormatInt(cb.From.Id, 10)
	isGroup := chat.Type == "group" || chat.Type == "supergroup"

	if !actorAllowed(acc, isGroup, chatID, senderID) {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Not authorized."})
		return
	}

	label := buttonLabel(msg.ReplyMarkup, cb.Data)
	if label == "" {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Already answered."})
		return
	}

	// Clear the keyboard by editing the message text (the proven path; the same
	// one the permission flow uses). Appending the choice both removes the
	// buttons and records the outcome inline, and it changes the text — so a
	// racing second tap re-edits to the same value, Telegram returns "not
	// modified", and we bail without delivering a duplicate.
	answered := msg.Text + "\n\n→ " + label
	if _, _, err := h.Bot.EditMessageTextWithContext(ctx, answered, &gotgbot.EditMessageTextOpts{
		ChatId:    chat.Id,
		MessageId: msg.MessageId,
	}); err != nil {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Already answered."})
		return
	}

	_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "✓ " + label})

	// No message_id: the only message here is the bot's own question, not an
	// inbound the reply should quote or react to. kind=button marks the source.
	meta := map[string]string{
		"chat_id": chatID,
		"user":    userDisplay(&cb.From),
		"user_id": senderID,
		"ts":      time.Now().UTC().Format("2006-01-02T15:04:05.000Z07:00"),
		"kind":    "button",
	}
	if err := h.relay(ctx, label, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: deliver button choice failed: %v\n", err)
	}
}
