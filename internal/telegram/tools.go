package telegram

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
)

const maxAttachmentBytes = 50 * 1024 * 1024

// Tools implements mcpchan.ToolSet against the Telegram Bot API.
type Tools struct {
	Bot *gotgbot.Bot
	Cfg *config.Config
	Log *transcript.Logger
}

// NewTools builds the tool set.
func NewTools(bot *gotgbot.Bot, cfg *config.Config, log *transcript.Logger) *Tools {
	return &Tools{Bot: bot, Cfg: cfg, Log: log}
}

// Reply sends one or more text chunks and then each file as a separate message
// (photos inline, others as documents). Returns a summary and an isError flag.
func (t *Tools) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	if t.Bot == nil {
		return "reply failed: no bot token configured", true
	}
	chatID, err := strconv.ParseInt(in.ChatID, 10, 64)
	if err != nil {
		return "reply failed: invalid chat_id: " + in.ChatID, true
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
	parseMode := parseModeFor(in.Format)

	var replyTo int64
	hasReplyTo := false
	if in.ReplyTo != "" {
		if v, err := strconv.ParseInt(in.ReplyTo, 10, 64); err == nil {
			replyTo, hasReplyTo = v, true
		}
	}
	replyMode := acc.ReplyToMode

	// Nothing to send is a caller error — text is no longer required, so guard it
	// here instead of letting Telegram reject an empty message.
	if countNonBlank(in.Bubbles) == 0 && in.Text == "" && len(in.Files) == 0 {
		return "reply failed: nothing to send — provide bubbles, text, or files", true
	}

	// Optional inline buttons. They attach to a text message (the last bubble or
	// the last text chunk), so they need something textual to sit under.
	buttons := sanitizeButtons(in.Buttons)
	if len(buttons) > 0 && countNonBlank(in.Bubbles) == 0 && in.Text == "" {
		return "reply failed: buttons need a question — provide bubbles or text to attach them to", true
	}
	var keyboard *gotgbot.InlineKeyboardMarkup
	if len(buttons) > 0 {
		k := buttonKeyboard(buttons)
		keyboard = &k
	}

	var sentIDs []int64
	// quoted tracks whether a quote-reply has already been emitted, so under the
	// default "first" mode only the first outbound part (bubble, text, or file)
	// quotes.
	quoted := false

	switch {
	case countNonBlank(in.Bubbles) > 0:
		// Paced bubble burst: each non-blank bubble is its own message. Under the
		// default "paced" mode a typing indicator and a length-scaled pause go
		// before every bubble after the first, for a human texting cadence;
		// "instant" sends them back-to-back. text is ignored when bubbles is set.
		paced := acc.BubbleMode != "instant"
		// Buttons ride on the last non-blank bubble — typically the question.
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
				_, _ = t.Bot.SendChatActionWithContext(ctx, chatID, "typing", nil)
				if sleepCtx(ctx, bubbleDelay(b)) {
					break // ctx cancelled (shutdown) — stop the burst
				}
			}
			opts := &gotgbot.SendMessageOpts{ParseMode: parseMode}
			if hasReplyTo && replyMode != "off" && (replyMode == "all" || !quoted) {
				opts.ReplyParameters = &gotgbot.ReplyParameters{MessageId: replyTo}
				quoted = true
			}
			if keyboard != nil && i == lastBubble {
				opts.ReplyMarkup = *keyboard
			}
			sent, err := t.Bot.SendMessageWithContext(ctx, chatID, b, opts)
			if err != nil {
				return fmt.Sprintf("reply failed after %d bubble(s) sent: %s", len(sentIDs), err), true
			}
			sentIDs = append(sentIDs, sent.MessageId)
			first = false
		}

	case in.Text != "" || len(in.Files) == 0:
		// Single-message / fallback path. Skip it entirely when there is no text
		// but there are files, so a media-only reply isn't aborted by Telegram's
		// "message text is empty" rejection.
		chunks := Chunk(in.Text, limit, acc.ChunkMode)
		for i, chunk := range chunks {
			opts := &gotgbot.SendMessageOpts{ParseMode: parseMode}
			if hasReplyTo && replyMode != "off" && (replyMode == "all" || i == 0) {
				opts.ReplyParameters = &gotgbot.ReplyParameters{MessageId: replyTo}
				quoted = true
			}
			if keyboard != nil && i == len(chunks)-1 {
				opts.ReplyMarkup = *keyboard
			}
			sent, err := t.Bot.SendMessageWithContext(ctx, chatID, chunk, opts)
			if err != nil {
				return fmt.Sprintf("reply failed after %d of %d chunk(s) sent: %s", len(sentIDs), len(chunks), err), true
			}
			sentIDs = append(sentIDs, sent.MessageId)
		}
	}

	for _, f := range in.Files {
		fh, err := os.Open(f)
		if err != nil {
			return "reply failed: " + err.Error(), true
		}
		input := gotgbot.InputFileByReader(filepath.Base(f), fh)
		var sentID int64
		var sendErr error
		if PhotoExts[strings.ToLower(filepath.Ext(f))] {
			opts := &gotgbot.SendPhotoOpts{}
			if hasReplyTo && replyMode != "off" && (replyMode == "all" || !quoted) {
				opts.ReplyParameters = &gotgbot.ReplyParameters{MessageId: replyTo}
				quoted = true
			}
			var sent *gotgbot.Message
			sent, sendErr = t.Bot.SendPhotoWithContext(ctx, chatID, input, opts)
			if sent != nil {
				sentID = sent.MessageId
			}
		} else {
			opts := &gotgbot.SendDocumentOpts{}
			if hasReplyTo && replyMode != "off" && (replyMode == "all" || !quoted) {
				opts.ReplyParameters = &gotgbot.ReplyParameters{MessageId: replyTo}
				quoted = true
			}
			var sent *gotgbot.Message
			sent, sendErr = t.Bot.SendDocumentWithContext(ctx, chatID, input, opts)
			if sent != nil {
				sentID = sent.MessageId
			}
		}
		fh.Close()
		if sendErr != nil {
			return fmt.Sprintf("reply failed sending file %s: %s", f, sendErr), true
		}
		sentIDs = append(sentIDs, sentID)
	}

	t.Log.Append(transcript.Record{
		Dir:       "out",
		ChatID:    in.ChatID,
		Kind:      "reply",
		MessageID: joinInts(sentIDs),
		Text:      outboundText(in),
	})

	if len(sentIDs) == 1 {
		return fmt.Sprintf("sent (id: %d)", sentIDs[0]), false
	}
	return fmt.Sprintf("sent %d parts (ids: %s)", len(sentIDs), joinInts(sentIDs)), false
}

// outboundText renders a reply's content for the transcript: the non-blank
// bubbles joined by newlines (else the single text), with a note appended for
// each attached file.
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

// React sets an emoji reaction on a message.
func (t *Tools) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	if t.Bot == nil {
		return "react failed: no bot token configured", true
	}
	chatID, err := strconv.ParseInt(in.ChatID, 10, 64)
	if err != nil {
		return "react failed: invalid chat_id: " + in.ChatID, true
	}
	msgID, err := strconv.ParseInt(in.MessageID, 10, 64)
	if err != nil {
		return "react failed: invalid message_id: " + in.MessageID, true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "react failed: " + err.Error(), true
	}
	_, err = t.Bot.SetMessageReactionWithContext(ctx, chatID, msgID, &gotgbot.SetMessageReactionOpts{
		Reaction: []gotgbot.ReactionType{gotgbot.ReactionTypeEmoji{Emoji: in.Emoji}},
	})
	if err != nil {
		return "react failed: " + err.Error(), true
	}
	return "reacted", false
}

// EditMessage edits the text of a message the bot previously sent.
func (t *Tools) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	if t.Bot == nil {
		return "edit_message failed: no bot token configured", true
	}
	chatID, err := strconv.ParseInt(in.ChatID, 10, 64)
	if err != nil {
		return "edit_message failed: invalid chat_id: " + in.ChatID, true
	}
	msgID, err := strconv.ParseInt(in.MessageID, 10, 64)
	if err != nil {
		return "edit_message failed: invalid message_id: " + in.MessageID, true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	edited, _, err := t.Bot.EditMessageTextWithContext(ctx, in.Text, &gotgbot.EditMessageTextOpts{
		ChatId:    chatID,
		MessageId: msgID,
		ParseMode: parseModeFor(in.Format),
	})
	if err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	id := msgID
	if edited != nil {
		id = edited.MessageId
	}
	return fmt.Sprintf("edited (id: %d)", id), false
}

// DownloadAttachment fetches a file by ID into the inbox and returns its path.
func (t *Tools) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	if t.Bot == nil {
		return "download_attachment failed: no bot token configured", true
	}
	path, err := DownloadToInbox(t.Bot, t.Cfg.InboxDir, in.FileID)
	if err != nil {
		return "download_attachment failed: " + err.Error(), true
	}
	return path, false
}

// assertAllowedChat refuses outbound traffic to chats the inbound gate wouldn't
// deliver from. Telegram DM chat_id == user_id, so allowFrom covers DMs.
func (t *Tools) assertAllowedChat(chatID string) error {
	acc, err := access.Load(t.Cfg.AccessFile)
	if err != nil {
		return err
	}
	if slices.Contains(acc.AllowFrom, chatID) {
		return nil
	}
	if _, ok := acc.Groups[chatID]; ok {
		return nil
	}
	return fmt.Errorf("chat %s is not allowlisted — pair via the hotline pair command", chatID)
}

// assertSendable refuses to attach the channel's own state files (which Claude
// has no reason to send), while allowing anything in the inbox.
func (t *Tools) assertSendable(file string) error {
	real, err := filepath.EvalSymlinks(file)
	if err != nil {
		// Let os.Stat surface a proper "not found" later.
		return nil
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

func joinInts(xs []int64) string {
	parts := make([]string, len(xs))
	for i, x := range xs {
		parts[i] = strconv.FormatInt(x, 10)
	}
	return strings.Join(parts, ", ")
}
