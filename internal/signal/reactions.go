package signal

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strconv"

	"github.com/1broseidon/hotline/internal/access"
)

// handleReaction relays an inbound reaction — a user adding or removing an
// emoji on a message — to Claude as a kind=reaction notification, mirroring
// the telegram adapter's shape. Authorization matches a button tap: only
// senders the inbound gate would accept get relayed, and a reaction never
// starts a pairing. Our own (synced) reactions are already filtered out by
// HandleEnvelope's sender check.
func (h *Handler) handleReaction(ctx context.Context, e *envelope, acc *access.Access, chatID string, isGroup bool, senderID string) {
	r := e.DataMessage.Reaction
	if r.Emoji == "" {
		return
	}
	if !actorAllowed(acc, isGroup, chatID, senderID) {
		return
	}

	action := "added"
	if r.IsRemove {
		action = "removed"
	}

	// target_message_id (not message_id) names the reacted-to message — usually
	// one of the bot's own — so Claude doesn't mistake it for an inbound message
	// it should quote or react to.
	meta := map[string]string{
		"chat_id":           chatID,
		"user":              SafeName(nonEmpty(e.SourceName, senderID)),
		"user_id":           senderID,
		"ts":                tsRFC3339(e.DataMessage.Timestamp),
		"kind":              "reaction",
		"reaction":          action,
		"target_message_id": h.reactionTargetID(r),
	}
	if err := h.relay(ctx, r.Emoji, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: deliver reaction failed: %v\n", err)
	}
}

// reactionTargetID renders the reacted-to message's identity in the adapter's
// message_id shape: a bare timestamp for our own messages (the form outbound
// sends return), "<timestamp>:<authorE164>" for everyone else's — so the agent
// can correlate the target with ids it has already seen.
func (h *Handler) reactionTargetID(r *reaction) string {
	author := nonEmpty(r.TargetAuthorNumber, r.TargetAuthor)
	if author == "" || author == h.Client.Account {
		return strconv.FormatInt(r.TargetSentTimestamp, 10)
	}
	return inboundMessageID(r.TargetSentTimestamp, author)
}

// actorAllowed reports whether a reaction's sender may reach Claude, mirroring
// the telegram/discord helper of the same name. DMs require the sender to be
// allowlisted; groups require the group to be configured and, if it restricts
// senders, the sender to be listed. The mention requirement that gates inbound
// group messages does not apply — a reaction on the bot's message is already a
// direct, intentional response.
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
