package telegram

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/access"
)

// handleReaction relays an inbound reaction — a user adding or removing an emoji
// on a message — to Claude as a kind=reaction notification. Telegram never
// echoes reactions the bot itself set, so this can't loop on the channel's own
// ack reactions.
func (h *Handler) handleReaction(ctx context.Context, mr *gotgbot.MessageReactionUpdated) {
	if mr.User == nil {
		// Anonymous (changed on behalf of a chat) — nothing to attribute or gate.
		return
	}

	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: access load failed: %v\n", err)
		return
	}

	chatID := strconv.FormatInt(mr.Chat.Id, 10)
	senderID := strconv.FormatInt(mr.User.Id, 10)
	isGroup := mr.Chat.Type == "group" || mr.Chat.Type == "supergroup"

	// Same authorization as a button tap: only senders the inbound gate would
	// accept get relayed. A reaction never starts a pairing.
	if !actorAllowed(acc, isGroup, chatID, senderID) {
		return
	}

	added, removed := reactionDiff(mr.OldReaction, mr.NewReaction)
	var content, action string
	switch {
	case len(added) > 0:
		content, action = strings.Join(added, " "), "added"
	case len(removed) > 0:
		content, action = strings.Join(removed, " "), "removed"
	default:
		// Only custom/paid reactions changed, which the channel doesn't surface.
		return
	}

	// target_message_id (not message_id) names the reacted-to message — usually
	// one of the bot's own — so Claude doesn't mistake it for an inbound message
	// it should quote or react to.
	meta := map[string]string{
		"chat_id":           chatID,
		"user":              userDisplay(mr.User),
		"user_id":           senderID,
		"ts":                time.Unix(mr.Date, 0).UTC().Format("2006-01-02T15:04:05.000Z07:00"),
		"kind":              "reaction",
		"reaction":          action,
		"target_message_id": strconv.FormatInt(mr.MessageId, 10),
	}
	if err := h.relay(ctx, content, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: deliver reaction failed: %v\n", err)
	}
}

// reactionEmojis extracts plain-emoji reactions from a reaction list, ignoring
// custom and paid reactions the channel doesn't model.
func reactionEmojis(rs []gotgbot.ReactionType) []string {
	out := make([]string, 0, len(rs))
	for _, r := range rs {
		if e, ok := r.(gotgbot.ReactionTypeEmoji); ok {
			out = append(out, e.Emoji)
		}
	}
	return out
}

// reactionDiff returns the emoji a single user added and removed between their
// old and new reaction sets on a message.
func reactionDiff(oldR, newR []gotgbot.ReactionType) (added, removed []string) {
	o, n := reactionEmojis(oldR), reactionEmojis(newR)
	for _, e := range n {
		if !slices.Contains(o, e) {
			added = append(added, e)
		}
	}
	for _, e := range o {
		if !slices.Contains(n, e) {
			removed = append(removed, e)
		}
	}
	return added, removed
}
