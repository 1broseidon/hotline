package discord

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/bwmarrin/discordgo"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
)

// maxAttachmentBytes caps outbound file uploads. Discord's default bot upload
// limit is 10MB (boosted servers allow more, but the default is the floor).
const maxAttachmentBytes = 10 * 1024 * 1024

// Tools implements mcpchan.ToolSet against the Discord REST API.
type Tools struct {
	Session Session // nil when no token is configured (handshake-only)
	Cfg     *config.Config
	Log     *transcript.Logger
}

// NewTools builds the tool set.
func NewTools(s Session, cfg *config.Config, log *transcript.Logger) *Tools {
	return &Tools{Session: s, Cfg: cfg, Log: log}
}

// Reply sends bubbles (paced with a typing indicator, mirroring telegram) or a
// single text (auto-split at Discord's 2000-char cap), native button
// components on the last bubble, then each file as its own message.
func (t *Tools) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	if t.Session == nil {
		return "reply failed: no bot token configured", true
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
			return fmt.Sprintf("reply failed: file too large: %s (%.1fMB, max 10MB)", f, float64(st.Size())/1024/1024), true
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
	var components []discordgo.MessageComponent
	if len(buttons) > 0 {
		components = buttonComponents(buttons)
	}

	var ref *discordgo.MessageReference
	if in.ReplyTo != "" {
		ref = &discordgo.MessageReference{ChannelID: in.ChatID, MessageID: in.ReplyTo}
	}
	replyMode := acc.ReplyToMode

	var sentIDs []string
	quoted := false
	sendText := func(body string, withComponents bool) error {
		msg := &discordgo.MessageSend{Content: body}
		if ref != nil && replyMode != "off" && (replyMode == "all" || !quoted) {
			msg.Reference = ref
			quoted = true
		}
		if withComponents {
			msg.Components = components
		}
		sent, err := t.Session.ChannelMessageSendComplex(in.ChatID, msg)
		if err != nil {
			return err
		}
		if sent != nil {
			sentIDs = append(sentIDs, sent.ID)
		}
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
				_ = t.Session.ChannelTyping(in.ChatID)
				if sleepCtx(ctx, bubbleDelay(b)) {
					break // ctx cancelled (shutdown) — stop the burst
				}
			}
			// Split each bubble defensively at the 2000-char cap; components
			// ride on the last piece of the last bubble.
			pieces := Chunk(b, limit, acc.ChunkMode)
			for j, piece := range pieces {
				withBtns := components != nil && i == lastBubble && j == len(pieces)-1
				if err := sendText(piece, withBtns); err != nil {
					return fmt.Sprintf("reply failed after %d bubble(s) sent: %s", len(sentIDs), err), true
				}
			}
			first = false
		}

	case in.Text != "" || len(in.Files) == 0:
		chunks := Chunk(in.Text, limit, acc.ChunkMode)
		for i, chunk := range chunks {
			if err := sendText(chunk, components != nil && i == len(chunks)-1); err != nil {
				return fmt.Sprintf("reply failed after %d of %d chunk(s) sent: %s", len(sentIDs), len(chunks), err), true
			}
		}
	}

	for _, f := range in.Files {
		fh, err := os.Open(f)
		if err != nil {
			return "reply failed: " + err.Error(), true
		}
		msg := &discordgo.MessageSend{
			Files: []*discordgo.File{{Name: filepath.Base(f), Reader: fh}},
		}
		if ref != nil && replyMode != "off" && (replyMode == "all" || !quoted) {
			msg.Reference = ref
			quoted = true
		}
		sent, sendErr := t.Session.ChannelMessageSendComplex(in.ChatID, msg)
		fh.Close()
		if sendErr != nil {
			return fmt.Sprintf("reply failed sending file %s: %s", f, sendErr), true
		}
		if sent != nil {
			sentIDs = append(sentIDs, sent.ID)
		}
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

// React adds a native emoji reaction to a message.
func (t *Tools) React(ctx context.Context, in mcpchan.ReactInput) (string, bool) {
	if t.Session == nil {
		return "react failed: no bot token configured", true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "react failed: " + err.Error(), true
	}
	if err := t.Session.MessageReactionAdd(in.ChatID, in.MessageID, in.Emoji); err != nil {
		return "react failed: " + err.Error(), true
	}
	return "reacted", false
}

// EditMessage edits the text of a message the bot previously sent.
func (t *Tools) EditMessage(ctx context.Context, in mcpchan.EditInput) (string, bool) {
	if t.Session == nil {
		return "edit_message failed: no bot token configured", true
	}
	if err := t.assertAllowedChat(in.ChatID); err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	text := in.Text
	if r := []rune(text); len(r) > MaxChunkLimit {
		return fmt.Sprintf("edit_message failed: text is %d chars (Discord max %d)", len(r), MaxChunkLimit), true
	}
	edited, err := t.Session.ChannelMessageEditComplex(&discordgo.MessageEdit{
		Channel: in.ChatID,
		ID:      in.MessageID,
		Content: &text,
	})
	if err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	id := in.MessageID
	if edited != nil {
		id = edited.ID
	}
	return fmt.Sprintf("edited (id: %s)", id), false
}

// DownloadAttachment fetches an attachment URL (the Discord file_id) into the
// inbox and returns its local path.
func (t *Tools) DownloadAttachment(ctx context.Context, in mcpchan.DownloadInput) (string, bool) {
	if t.Session == nil {
		return "download_attachment failed: no bot token configured", true
	}
	path, err := DownloadToInbox(t.Cfg.InboxDir, in.FileID)
	if err != nil {
		return "download_attachment failed: " + err.Error(), true
	}
	return path, false
}

// assertAllowedChat refuses outbound traffic to channels the inbound gate
// wouldn't deliver from: a configured guild channel, or a DM channel recorded
// as belonging to an allowlisted user (see dmchannels.go).
func (t *Tools) assertAllowedChat(chatID string) error {
	acc, err := access.Load(t.Cfg.AccessFile)
	if err != nil {
		return err
	}
	if _, ok := acc.Groups[chatID]; ok {
		return nil
	}
	if uid := dmChannelUser(t.Cfg.StateDir, chatID); uid != "" && slices.Contains(acc.AllowFrom, uid) {
		return nil
	}
	return fmt.Errorf("channel %s is not allowlisted — pair via the hotline pair command", chatID)
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
