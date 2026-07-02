package signal

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// maxAttachmentBytes caps outbound file uploads — Signal's attachment cap is
// 100MB, but 50MB matches the download side and the other adapters.
const maxAttachmentBytes = 50 * 1024 * 1024

// maxButtons caps how many options one degraded-buttons reply can carry.
// Matches the telegram/discord adapters.
const maxButtons = 12

// Tools implements mcpchan.ToolSet against the signal-cli daemon's JSON-RPC
// endpoint.
type Tools struct {
	Client  *Client // nil when SIGNAL_ACCOUNT is not configured (handshake-only)
	Cfg     *config.Config
	Log     *transcript.Logger
	Options *optionStore
}

// NewTools builds the tool set sharing the Handler's option store.
func NewTools(c *Client, cfg *config.Config, log *transcript.Logger, opts *optionStore) *Tools {
	return &Tools{Client: c, Cfg: cfg, Log: log, Options: opts}
}

// Reply sends bubbles as sequential sends paced with sendTyping (mirroring
// the other adapters) or a single text (auto-split at the 2000-char cap),
// then each file as its own send. Signal has no inline buttons, so buttons
// degrade to numbered text options appended to the last bubble; the numbered
// labels are remembered so a bare-number answer maps back to its label.
func (t *Tools) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	if t.Client == nil {
		return "reply failed: no signal account configured", true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "reply failed: " + err.Error(), true
	}

	for _, f := range in.Files {
		if err := t.assertSendable(f); err != nil {
			return "reply failed: " + err.Error(), true
		}
		st, err := os.Stat(f)
		if err != nil {
			return "reply failed: " + err.Error(), true
		}
		if st.Size() > maxAttachmentBytes {
			return fmt.Sprintf("reply failed: file too large: %s (%.1fMB, max 50MB)", f, float64(st.Size())/1024/1024), true
		}
	}

	acc, err := access.Load(t.Cfg.AccessFile)
	if err != nil {
		return "reply failed: " + err.Error(), true
	}
	limit := min(acc.TextChunkLimit, MaxChunkLimit)

	if countNonBlank(in.Bubbles) == 0 && in.Text == "" && len(in.Files) == 0 {
		return "reply failed: nothing to send — provide bubbles, text, or files", true
	}

	buttons := sanitizeButtons(in.Buttons)
	if len(buttons) > 0 && countNonBlank(in.Bubbles) == 0 && in.Text == "" {
		return "reply failed: buttons need a question — provide bubbles or text to attach them to", true
	}
	numbered := ""
	if len(buttons) > 0 {
		numbered = "\n\n" + buttonsToNumberedText(buttons)
	}

	var sentIDs []string
	sendText := func(body string) error {
		ts, err := t.Client.Send(ctx, in.ChatID, body, nil, 0)
		if err != nil {
			return err
		}
		sentIDs = append(sentIDs, fmt.Sprintf("%d", ts))
		return nil
	}

	switch {
	case countNonBlank(in.Bubbles) > 0:
		paced := acc.BubbleMode != "instant"
		lastBubble := -1
		for i, b := range in.Bubbles {
			if strings.TrimSpace(b) != "" {
				lastBubble = i
			}
		}
		first := true
		for i, b := range in.Bubbles {
			if strings.TrimSpace(b) == "" {
				continue
			}
			if !first && paced {
				_ = t.Client.SendTyping(ctx, in.ChatID)
				if sleepCtx(ctx, bubbleDelay(b)) {
					break // ctx cancelled (shutdown) — stop the burst
				}
			}
			// Split each bubble defensively at the cap; the numbered options
			// ride on the last piece of the last bubble.
			pieces := Chunk(b, limit, acc.ChunkMode)
			for j, piece := range pieces {
				if numbered != "" && i == lastBubble && j == len(pieces)-1 {
					piece += numbered
				}
				if err := sendText(piece); err != nil {
					return fmt.Sprintf("reply failed after %d bubble(s) sent: %s", len(sentIDs), err), true
				}
			}
			first = false
		}

	case in.Text != "" || len(in.Files) == 0:
		chunks := Chunk(in.Text, limit, acc.ChunkMode)
		for i, chunk := range chunks {
			if numbered != "" && i == len(chunks)-1 {
				chunk += numbered
			}
			if err := sendText(chunk); err != nil {
				return fmt.Sprintf("reply failed after %d of %d chunk(s) sent: %s", len(sentIDs), len(chunks), err), true
			}
		}
	}

	if len(buttons) > 0 && len(sentIDs) > 0 {
		t.Options.set(in.ChatID, buttons)
	}

	for _, f := range in.Files {
		ts, sendErr := t.Client.Send(ctx, in.ChatID, "", []string{f}, 0)
		if sendErr != nil {
			return fmt.Sprintf("reply failed sending file %s: %s", f, sendErr), true
		}
		sentIDs = append(sentIDs, fmt.Sprintf("%d", ts))
	}

	t.Log.Append(transcript.Record{
		Dir:       "out",
		ChatID:    in.ChatID,
		Kind:      "reply",
		MessageID: strings.Join(sentIDs, ", "),
		Text:      outboundText(in),
	})

	if len(sentIDs) == 1 {
		return fmt.Sprintf("sent (id: %s)", sentIDs[0]), false
	}
	return fmt.Sprintf("sent %d parts (ids: %s)", len(sentIDs), strings.Join(sentIDs, ", ")), false
}

// sanitizeButtons trims labels, drops blanks, and caps the count.
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

// buttonsToNumberedText renders the degraded options with a hint line, using
// the shared provider degradation for the list itself.
func buttonsToNumberedText(buttons []string) string {
	return provider.ButtonsToNumberedText(buttons) + "\n\nReply with a number to choose."
}

// outboundText renders a reply's content for the transcript.
func outboundText(in mcpchan.ReplyInput) string {
	var b strings.Builder
	if countNonBlank(in.Bubbles) > 0 {
		first := true
		for _, s := range in.Bubbles {
			if strings.TrimSpace(s) == "" {
				continue
			}
			if !first {
				b.WriteByte('\n')
			}
			b.WriteString(s)
			first = false
		}
	} else {
		b.WriteString(in.Text)
	}
	for _, f := range in.Files {
		if b.Len() > 0 {
			b.WriteByte('\n')
		}
		b.WriteString("[file: ")
		b.WriteString(filepath.Base(f))
		b.WriteByte(']')
	}
	return b.String()
}

// React adds an emoji reaction to a message. Inbound message_ids carry
// "<timestamp>:<author>"; a bare timestamp addresses one of our own messages.
func (t *Tools) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	if t.Client == nil {
		return "react failed: no signal account configured", true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "react failed: " + err.Error(), true
	}
	ts, author, ok := parseMessageID(in.MessageID)
	if !ok {
		return fmt.Sprintf("react failed: invalid message_id %q", in.MessageID), true
	}
	if author == "" {
		author = t.Client.Account
	}
	if err := t.Client.SendReaction(ctx, in.ChatID, in.Emoji, author, ts); err != nil {
		return "react failed: " + err.Error(), true
	}
	return "reacted", false
}

// EditMessage edits a message we previously sent, via signal-cli's
// send --edit-timestamp (JSON-RPC param editTimestamp). Returns the edit's
// own timestamp as the new message_id — Signal chains subsequent edits off
// the newest revision.
func (t *Tools) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	if t.Client == nil {
		return "edit_message failed: no signal account configured", true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	ts, author, ok := parseMessageID(in.MessageID)
	if !ok {
		return fmt.Sprintf("edit_message failed: invalid message_id %q", in.MessageID), true
	}
	if author != "" && author != t.Client.Account {
		return "edit_message failed: can only edit messages the bot sent", true
	}
	if r := []rune(in.Text); len(r) > MaxChunkLimit {
		return fmt.Sprintf("edit_message failed: text is %d chars (Signal max %d)", len(r), MaxChunkLimit), true
	}
	newTS, err := t.Client.Send(ctx, in.ChatID, in.Text, nil, ts)
	if err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	return fmt.Sprintf("edited (id: %d)", newTS), false
}

// DownloadAttachment fetches an attachment through the daemon's getAttachment
// command into the inbox and returns its local path. The file_id is the
// "<id>|<chat>" pair from inbound meta.
func (t *Tools) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	if t.Client == nil {
		return "download_attachment failed: no signal account configured", true
	}
	id, chatID := parseAttachmentFileID(in.FileID)
	if id == "" {
		return "download_attachment failed: empty file_id", true
	}
	data, err := t.Client.GetAttachment(ctx, chatID, id)
	if err != nil {
		return "download_attachment failed: " + err.Error(), true
	}
	path, err := saveToInbox(t.Cfg.InboxDir, data, id, "")
	if err != nil {
		return "download_attachment failed: " + err.Error(), true
	}
	return path, false
}

// assertAllowedChat refuses outbound traffic to chats the inbound gate
// wouldn't deliver from: a configured group, or an allowlisted number. For
// Signal DMs the chat_id IS the peer's number, so the check is direct (like
// telegram, unlike discord's DM-channel indirection).
func (t *Tools) assertAllowedChat(chatID string) error {
	acc, err := access.Load(t.Cfg.AccessFile)
	if err != nil {
		return err
	}
	if _, ok := acc.Groups[chatID]; ok {
		return nil
	}
	if !strings.HasPrefix(chatID, groupChatPrefix) && slices.Contains(acc.AllowFrom, chatID) {
		return nil
	}
	return fmt.Errorf("chat %s is not allowlisted — pair via the hotline pair command", chatID)
}

// assertSendable refuses to attach the channel's own state files, while
// allowing anything in the inbox.
func (t *Tools) assertSendable(file string) error {
	real, err := filepath.EvalSymlinks(file)
	if err != nil {
		return nil // let os.Stat surface a proper "not found" later
	}
	stateReal, err := filepath.EvalSymlinks(t.Cfg.StateDir)
	if err != nil {
		return nil
	}
	inbox := stateReal + string(os.PathSeparator) + "inbox"
	if strings.HasPrefix(real, stateReal+string(os.PathSeparator)) &&
		!strings.HasPrefix(real, inbox+string(os.PathSeparator)) {
		return fmt.Errorf("refusing to send channel state: %s", file)
	}
	return nil
}

// sleepCtx sleeps for d or until ctx is done, reporting whether it was cut
// short by cancellation.
func sleepCtx(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return true
	case <-t.C:
		return false
	}
}
