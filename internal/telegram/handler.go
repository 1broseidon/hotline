package telegram

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf16"

	"github.com/PaulSonOfLars/gotgbot/v2"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Handler dispatches Telegram updates: it gates on the sender, relays inbound
// messages to Claude, handles the permission text-reply / button paths, and
// fans permission requests out to allowlisted DMs.
type Handler struct {
	Bot *gotgbot.Bot
	Cfg *config.Config
	// Notifier is the inbound sink (channel deliveries + permission verdicts).
	// In production it is the router's source-tagging wrapper around the MCP
	// notifier, bound by Provider.Start just before polling begins.
	Notifier provider.InboundSink
	Log      *transcript.Logger

	mu        sync.Mutex
	permCache map[string]permEntry

	// Inbound coalescing (see coalesce.go): buffers texting bursts per chat so a
	// rapid sequence reaches Claude as one turn instead of racing fragments.
	coalMu          sync.Mutex
	buffers         map[string]*chatBuffer
	coalesceWindow  time.Duration
	coalesceMaxWait time.Duration
	// coalDeliver, when set, replaces flush as the burst sink (tests capture
	// bursts without a live notifier).
	coalDeliver func(context.Context, []pendingMsg)
}

// permEntry is a cached permission request plus the time it was received, so
// stale (unanswered) prompts can be purged.
type permEntry struct {
	params mcpchan.PermissionRequestParams
	at     time.Time
}

// permCacheTTL bounds how long an unanswered permission request stays cached.
// Claude-side prompts that time out (the user simply ignores them) are never
// answered here, so without a TTL the cache would grow without bound.
const permCacheTTL = 10 * time.Minute

// NewHandler builds a Handler. Notifier is assigned later (after the transport
// connects) by the lifecycle wiring.
func NewHandler(bot *gotgbot.Bot, cfg *config.Config, n provider.InboundSink, log *transcript.Logger) *Handler {
	return &Handler{
		Bot:             bot,
		Cfg:             cfg,
		Notifier:        n,
		Log:             log,
		permCache:       make(map[string]permEntry),
		buffers:         make(map[string]*chatBuffer),
		coalesceWindow:  defaultCoalesceWindow,
		coalesceMaxWait: defaultCoalesceMaxWait,
	}
}

// relay logs an inbound event to the transcript, then delivers it to Claude.
// Every inbound path (messages, button taps, reactions) funnels through here so
// the transcript is the complete record of what reached the assistant.
func (h *Handler) relay(ctx context.Context, content string, meta map[string]string) error {
	h.Log.Append(transcript.Record{
		Dir:       "in",
		ChatID:    meta["chat_id"],
		User:      meta["user"],
		UserID:    meta["user_id"],
		Kind:      inboundKind(meta),
		MessageID: meta["message_id"],
		Text:      content,
	})
	if h.Notifier == nil {
		return nil
	}
	return h.Notifier.SendChannel(ctx, content, meta)
}

// inboundKind classifies an inbound for the transcript: an explicit meta kind
// (reaction / button) wins, then media, defaulting to text.
func inboundKind(meta map[string]string) string {
	if k := meta["kind"]; k != "" {
		return k
	}
	if meta["image_path"] != "" {
		return "photo"
	}
	if k := meta["attachment_kind"]; k != "" {
		return k
	}
	return "text"
}

// Dispatch routes a single update. It recovers from per-update panics in the
// poll loop, so it stays defensive but does not itself recover here.
func (h *Handler) Dispatch(ctx context.Context, u *gotgbot.Update) {
	if u.CallbackQuery != nil {
		switch {
		case mcpchan.PermBtnRe.MatchString(u.CallbackQuery.Data):
			h.handlePermCallback(ctx, u.CallbackQuery)
		case actBtnRe.MatchString(u.CallbackQuery.Data):
			h.handleActionCallback(ctx, u.CallbackQuery)
		default:
			_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, u.CallbackQuery.Id, nil)
		}
		return
	}
	if u.MessageReaction != nil {
		h.handleReaction(ctx, u.MessageReaction)
		return
	}
	msg := u.Message
	if msg == nil {
		msg = u.EditedMessage
	}
	if msg == nil || msg.From == nil {
		return
	}
	h.handleMessage(ctx, msg)
}

func (h *Handler) handleMessage(ctx context.Context, msg *gotgbot.Message) {
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: access load failed: %v\n", err)
		return
	}

	chatID := strconv.FormatInt(msg.Chat.Id, 10)
	senderID := strconv.FormatInt(msg.From.Id, 10)
	isGroup := msg.Chat.Type == "group" || msg.Chat.Type == "supergroup"
	text := messageText(msg)

	in := access.GateInput{
		IsGroup:      isGroup,
		ChatID:       chatID,
		SenderID:     senderID,
		Text:         text,
		MentionedBot: h.isMentioned(msg),
		RepliedToBot: h.repliedToBot(msg),
	}

	// Permission text-reply intercept: a gate-approved sender answering a
	// pending permission request. Must be checked before the gate's pairing
	// side effects and before relaying.
	if m := mcpchan.PermReplyRe.FindStringSubmatch(text); m != nil && isAllowlisted(acc, senderID) {
		behavior := mcpchan.BehaviorFromYesNo(m[1])
		code := strings.ToLower(m[2])
		// Claim the request atomically: only the answerer that removes the
		// cache entry may emit a verdict, so two allowlisted users (or a button
		// press plus a text reply) can't write conflicting allow+deny.
		if !h.claimPerm(code) {
			h.react(ctx, msg.Chat.Id, msg.MessageId, "🤷")
			return
		}
		if h.Notifier != nil {
			if err := h.Notifier.SendVerdict(ctx, code, behavior); err != nil {
				fmt.Fprintf(os.Stderr, "hotline: send verdict failed: %v\n", err)
			}
		}
		emoji := "✅"
		if behavior == "deny" {
			emoji = "❌"
		}
		h.react(ctx, msg.Chat.Id, msg.MessageId, emoji)
		return
	}

	switch access.Gate(acc, in) {
	case access.Drop:
		return
	case access.Pair:
		h.replyPairing(ctx, msg, senderID, chatID)
		return
	case access.Allow:
		// fall through
	}

	// Ack reaction (best-effort).
	if acc.AckReaction != "" {
		h.react(ctx, msg.Chat.Id, msg.MessageId, acc.AckReaction)
	}

	content := text
	meta := map[string]string{
		"chat_id": chatID,
		"user":    userDisplay(msg.From),
		"user_id": senderID,
		"ts":      time.Unix(msg.Date, 0).UTC().Format("2006-01-02T15:04:05.000Z07:00"),
	}
	if msg.MessageId != 0 {
		meta["message_id"] = strconv.FormatInt(msg.MessageId, 10)
	}

	// Reply-to context: when they reply to a specific earlier message, surface
	// which one and a sanitized snippet of it. reply_to_from == "you" marks a
	// reply to one of the bot's own (i.e. Claude's) messages.
	if rt := msg.ReplyToMessage; rt != nil {
		meta["reply_to_message_id"] = strconv.FormatInt(rt.MessageId, 10)
		if rt.From != nil {
			if rt.From.Id == h.Bot.User.Id {
				meta["reply_to_from"] = "you"
			} else {
				meta["reply_to_from"] = SafeName(userDisplay(rt.From))
			}
		}
		if q := replyQuote(messageText(rt)); q != "" {
			meta["reply_to_text"] = q
		}
	}

	if ex, ok := Extract(msg); ok {
		if content == "" {
			content = ex.Synthetic
		}
		if ex.IsPhoto {
			// Eager download photos so Claude can Read them immediately.
			if path, err := DownloadToInbox(h.Bot, h.Cfg.InboxDir, ex.FileID); err != nil {
				fmt.Fprintf(os.Stderr, "hotline: photo download failed: %v\n", err)
			} else {
				meta["image_path"] = path
			}
		} else {
			meta["attachment_kind"] = ex.Kind
			meta["attachment_file_id"] = ex.FileID
			if ex.Size != 0 {
				meta["attachment_size"] = strconv.FormatInt(ex.Size, 10)
			}
			if ex.MIME != "" {
				meta["attachment_mime"] = ex.MIME
			}
			if ex.Name != "" {
				meta["attachment_name"] = ex.Name
			}
		}
	}

	if content == "" {
		// Nothing to deliver (e.g. a service message we don't model).
		return
	}

	// Buffer into the coalescing window instead of relaying immediately, so a
	// burst of quick messages reaches Claude as one turn. Button/reaction paths
	// still use relay directly — they're discrete events, not texting bubbles.
	h.enqueue(ctx, content, meta)
}

// handlePermCallback processes inline permission buttons (perm:allow|deny|more).
func (h *Handler) handlePermCallback(ctx context.Context, cb *gotgbot.CallbackQuery) {
	m := mcpchan.PermBtnRe.FindStringSubmatch(cb.Data)
	if m == nil {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, nil)
		return
	}
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, nil)
		return
	}
	senderID := strconv.FormatInt(cb.From.Id, 10)
	if !isAllowlisted(acc, senderID) {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Not authorized."})
		return
	}

	action, code := m[1], m[2]

	if action == "more" {
		h.mu.Lock()
		entry, ok := h.permCache[code]
		h.mu.Unlock()
		details := entry.params
		if !ok {
			_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Details no longer available."})
			return
		}
		expanded := fmt.Sprintf("🔐 Permission: %s\n\ntool_name: %s\ndescription: %s\ninput_preview:\n%s",
			details.ToolName, details.ToolName, details.Description, details.InputPreview)
		kb := gotgbot.InlineKeyboardMarkup{InlineKeyboard: [][]gotgbot.InlineKeyboardButton{{
			{Text: "✅ Allow", CallbackData: "perm:allow:" + code},
			{Text: "❌ Deny", CallbackData: "perm:deny:" + code},
		}}}
		if msg := cb.Message; msg != nil {
			_, _, _ = h.Bot.EditMessageTextWithContext(ctx, expanded, &gotgbot.EditMessageTextOpts{
				ChatId:      msg.GetChat().Id,
				MessageId:   msg.GetMessageId(),
				ReplyMarkup: kb,
			})
		}
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, nil)
		return
	}

	// allow / deny verdict. Claim the request atomically so only the first
	// answerer emits a verdict; a second press (or a racing text reply) finds
	// the entry gone and is told it was already handled.
	if !h.claimPerm(code) {
		_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: "Already handled."})
		return
	}
	if h.Notifier != nil {
		if err := h.Notifier.SendVerdict(ctx, code, action); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: send verdict failed: %v\n", err)
		}
	}

	label := "✅ Allowed"
	if action == "deny" {
		label = "❌ Denied"
	}
	_, _ = h.Bot.AnswerCallbackQueryWithContext(ctx, cb.Id, &gotgbot.AnswerCallbackQueryOpts{Text: label})
	// Replace the buttons with the outcome so it can't be answered twice.
	if msg := cb.Message; msg != nil {
		_, _, _ = h.Bot.EditMessageTextWithContext(ctx, label, &gotgbot.EditMessageTextOpts{
			ChatId:    msg.GetChat().Id,
			MessageId: msg.GetMessageId(),
		})
	}
}

// OnPermissionRequest fans a permission request out to allowlisted DMs only.
// Groups are excluded: only paired single users may answer.
func (h *Handler) OnPermissionRequest(ctx context.Context, p mcpchan.PermissionRequestParams) {
	if h.Bot == nil {
		return
	}
	h.mu.Lock()
	h.purgePermLocked()
	h.permCache[p.RequestID] = permEntry{params: p, at: time.Now()}
	h.mu.Unlock()

	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: access load failed: %v\n", err)
		return
	}
	text := "🔐 Permission: " + p.ToolName
	kb := gotgbot.InlineKeyboardMarkup{InlineKeyboard: [][]gotgbot.InlineKeyboardButton{{
		{Text: "See more", CallbackData: "perm:more:" + p.RequestID},
		{Text: "✅ Allow", CallbackData: "perm:allow:" + p.RequestID},
		{Text: "❌ Deny", CallbackData: "perm:deny:" + p.RequestID},
	}}}
	for _, id := range acc.AllowFrom {
		chatID, err := strconv.ParseInt(id, 10, 64)
		if err != nil {
			continue
		}
		if _, err := h.Bot.SendMessageWithContext(ctx, chatID, text, &gotgbot.SendMessageOpts{ReplyMarkup: kb}); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: permission_request send to %s failed: %v\n", id, err)
		}
	}
}

// claimPerm atomically removes the cached entry for code and reports whether it
// was present. Only the caller that observes ok==true owns the request and may
// emit a verdict; concurrent or duplicate answers see ok==false.
func (h *Handler) claimPerm(code string) bool {
	h.mu.Lock()
	defer h.mu.Unlock()
	if _, ok := h.permCache[code]; !ok {
		return false
	}
	delete(h.permCache, code)
	return true
}

// purgePermLocked drops permission entries older than permCacheTTL. The caller
// must hold h.mu.
func (h *Handler) purgePermLocked() {
	cutoff := time.Now().Add(-permCacheTTL)
	for code, e := range h.permCache {
		if e.at.Before(cutoff) {
			delete(h.permCache, code)
		}
	}
}

func (h *Handler) replyPairing(ctx context.Context, msg *gotgbot.Message, senderID, chatID string) {
	code, send, err := access.CreatePairing(h.Cfg.AccessFile, senderID, chatID)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: create pairing failed: %v\n", err)
		return
	}
	if !send {
		return
	}
	body := "Pairing required — the operator runs in their terminal:\n\n" + pairingInstruction(h.Cfg.BotName, code)
	if _, err := h.Bot.SendMessageWithContext(ctx, msg.Chat.Id, body, nil); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: pairing reply failed: %v\n", err)
	}
}

// pairingInstruction is the terminal command the operator runs to approve a
// pairing. For a named bot it includes --bot <name>, since the pending code
// lives in that bot's own access.json and a bare `pair` would approve against
// the default bot instead.
func pairingInstruction(botName, code string) string {
	cmd := "hotline pair " + code
	if botName != "" {
		cmd += " --bot " + botName
	}
	return cmd
}

func (h *Handler) react(ctx context.Context, chatID, msgID int64, emoji string) {
	_, _ = h.Bot.SetMessageReactionWithContext(ctx, chatID, msgID, &gotgbot.SetMessageReactionOpts{
		Reaction: []gotgbot.ReactionType{gotgbot.ReactionTypeEmoji{Emoji: emoji}},
	})
}

// isMentioned reports whether the bot is @-mentioned in the message, using
// UTF-16 entity offsets against the bot's username.
func (h *Handler) isMentioned(msg *gotgbot.Message) bool {
	username := h.Bot.User.Username
	if username == "" {
		return false
	}
	want := strings.ToLower("@" + username)

	text := msg.Text
	entities := msg.Entities
	if text == "" {
		text = msg.Caption
		entities = msg.CaptionEntities
	}
	units := utf16Units(text)
	for _, e := range entities {
		switch e.Type {
		case "mention":
			if s := utf16Slice(units, e.Offset, e.Length); strings.EqualFold(s, want) {
				return true
			}
		case "text_mention":
			// Telegram emits text_mention only for users WITHOUT a username, so
			// matching on Username can never identify the bot. Match by identity.
			if e.User != nil && e.User.Id == h.Bot.User.Id {
				return true
			}
		}
	}
	return false
}

func (h *Handler) repliedToBot(msg *gotgbot.Message) bool {
	return msg.ReplyToMessage != nil &&
		msg.ReplyToMessage.From != nil &&
		msg.ReplyToMessage.From.Id == h.Bot.User.Id
}

func messageText(msg *gotgbot.Message) string {
	if msg.Text != "" {
		return msg.Text
	}
	return msg.Caption
}

// replyQuote sanitizes and truncates a quoted reply's text for safe inclusion as
// <channel> meta. It reuses SafeName (the meta-breakout sanitizer), additionally
// neutralizes double quotes (a freeform message body is likelier than a filename
// to contain them), collapses to a single line, and clamps to 200 runes.
func replyQuote(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	s = SafeName(strings.ReplaceAll(s, `"`, "'"))
	if r := []rune(s); len(r) > 200 {
		s = strings.TrimSpace(string(r[:200])) + "…"
	}
	return s
}

func isAllowlisted(a *access.Access, senderID string) bool {
	return slices.Contains(a.AllowFrom, senderID)
}

func userDisplay(u *gotgbot.User) string {
	if u.Username != "" {
		return u.Username
	}
	return strconv.FormatInt(u.Id, 10)
}

// utf16Units returns the UTF-16 code units of s, so Telegram entity offsets
// (which are in UTF-16) can index correctly into multibyte text.
func utf16Units(s string) []uint16 {
	return utf16.Encode([]rune(s))
}

func utf16Slice(units []uint16, offset, length int64) string {
	start := int(offset)
	end := int(offset + length)
	if start < 0 || start > len(units) || end < start || end > len(units) {
		return ""
	}
	return string(utf16.Decode(units[start:end]))
}
