package discord

import (
	"context"
	"fmt"
	"os"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/bwmarrin/discordgo"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Handler dispatches Discord gateway events: it gates on the sender, relays
// inbound messages to Claude, handles the button-interaction and permission
// paths, and fans permission requests out to allowlisted DMs. It mirrors the
// telegram Handler's structure: same access model, same coalescing, same
// permission claim semantics.
type Handler struct {
	Session Session
	Cfg     *config.Config
	// BotUserID is the bot's own user ID (self-messages are dropped). Bound by
	// Provider.Start once the gateway identifies; injectable in tests.
	BotUserID string
	Log       *transcript.Logger

	// notifier is the inbound sink (channel deliveries + permission verdicts),
	// bound by Provider.Start (or tests) via BindNotifier.
	notifierMu sync.RWMutex
	notifier   provider.InboundSink

	mu        sync.Mutex
	permCache map[string]permEntry

	// Inbound coalescing (see coalesce.go).
	coalMu          sync.Mutex
	buffers         map[string]*chatBuffer
	coalesceWindow  time.Duration
	graceWindow     time.Duration
	coalesceMaxWait time.Duration
	coalDeliver     func(context.Context, []pendingMsg)
}

// permEntry is a cached permission request plus arrival time (for TTL purge).
type permEntry struct {
	params mcpchan.PermissionRequestParams
	at     time.Time
}

// permCacheTTL bounds how long an unanswered permission request stays cached.
const permCacheTTL = 10 * time.Minute

// NewHandler builds a Handler. Notifier and BotUserID are assigned by
// Provider.Start once the gateway session is up.
func NewHandler(s Session, cfg *config.Config, log *transcript.Logger) *Handler {
	return &Handler{
		Session:         s,
		Cfg:             cfg,
		Log:             log,
		permCache:       make(map[string]permEntry),
		buffers:         make(map[string]*chatBuffer),
		coalesceWindow:  defaultCoalesceWindow,
		graceWindow:     defaultGraceWindow,
		coalesceMaxWait: defaultCoalesceMaxWait,
	}
}

// BindNotifier sets the inbound sink. Safe to call while events are flowing.
func (h *Handler) BindNotifier(sink provider.InboundSink) {
	h.notifierMu.Lock()
	defer h.notifierMu.Unlock()
	h.notifier = sink
}

// Notifier returns the currently bound inbound sink (nil before binding).
func (h *Handler) Notifier() provider.InboundSink {
	h.notifierMu.RLock()
	defer h.notifierMu.RUnlock()
	return h.notifier
}

// relay logs an inbound event to the transcript, then delivers it to Claude.
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
	n := h.Notifier()
	if n == nil {
		return nil
	}
	return n.SendChannel(ctx, content, meta)
}

// inboundKind classifies an inbound for the transcript.
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

// HandleMessage processes one MessageCreate event.
func (h *Handler) HandleMessage(ctx context.Context, m *discordgo.Message) {
	if m == nil || m.Author == nil {
		return
	}
	if m.Author.ID == h.BotUserID || m.Author.Bot {
		return // never relay the bot's own (or any bot's) messages
	}

	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: access load failed: %v\n", err)
		return
	}

	chatID := m.ChannelID
	senderID := m.Author.ID
	isGuild := m.GuildID != ""
	text := m.Content

	// Permission text-reply intercept: a gate-approved sender answering a
	// pending permission request. Checked before the gate's pairing side
	// effects and before relaying — same order as telegram.
	if mm := mcpchan.PermReplyRe.FindStringSubmatch(text); mm != nil && isAllowlisted(acc, senderID) {
		behavior := mcpchan.BehaviorFromYesNo(mm[1])
		code := strings.ToLower(mm[2])
		if !h.claimPerm(code) {
			_ = h.Session.MessageReactionAdd(chatID, m.ID, "🤷")
			return
		}
		if n := h.Notifier(); n != nil {
			if err := n.SendVerdict(ctx, code, behavior); err != nil {
				fmt.Fprintf(os.Stderr, "hotline: send verdict failed: %v\n", err)
			}
		}
		emoji := "✅"
		if behavior == "deny" {
			emoji = "❌"
		}
		_ = h.Session.MessageReactionAdd(chatID, m.ID, emoji)
		return
	}

	in := access.GateInput{
		IsGroup:      isGuild,
		ChatID:       chatID, // guild channels gate as "groups" keyed by channel ID
		SenderID:     senderID,
		Text:         text,
		MentionedBot: h.isMentioned(m),
		RepliedToBot: h.repliedToBot(m),
	}

	switch access.Gate(acc, in) {
	case access.Drop:
		return
	case access.Pair:
		h.replyPairing(ctx, m, senderID, chatID)
		return
	case access.Allow:
		// fall through
	}

	// Remember which DM channel belongs to this allowlisted user, so the
	// outbound gate (assertAllowedChat) can authorize replies to it — Discord
	// DM channel IDs are unrelated to user IDs, unlike Telegram.
	if !isGuild {
		recordDMChannel(h.Cfg.StateDir, chatID, senderID)
	}

	if acc.AckReaction != "" {
		_ = h.Session.MessageReactionAdd(chatID, m.ID, acc.AckReaction)
	}

	content := text
	meta := map[string]string{
		"chat_id":    chatID,
		"user":       userDisplay(m.Author),
		"user_id":    senderID,
		"ts":         m.Timestamp.UTC().Format("2006-01-02T15:04:05.000Z07:00"),
		"message_id": m.ID,
	}

	// Reply-to context, mirroring telegram's quoted-reply meta.
	if rt := m.ReferencedMessage; rt != nil {
		meta["reply_to_message_id"] = rt.ID
		if rt.Author != nil {
			if rt.Author.ID == h.BotUserID {
				meta["reply_to_from"] = "you"
			} else {
				meta["reply_to_from"] = SafeName(userDisplay(rt.Author))
			}
		}
		if q := replyQuote(rt.Content); q != "" {
			meta["reply_to_text"] = q
		}
	}

	// Attachments: images are eagerly downloaded so Claude can Read them
	// immediately; everything else is surfaced lazily by URL (the file_id the
	// download_attachment tool accepts).
	if len(m.Attachments) > 0 {
		att := m.Attachments[0]
		if ex := extract(att); ex.IsPhoto {
			if content == "" {
				content = "(photo)"
			}
			if path, err := DownloadToInbox(h.Cfg.InboxDir, att.URL); err != nil {
				fmt.Fprintf(os.Stderr, "hotline: attachment download failed: %v\n", err)
			} else {
				meta["image_path"] = path
			}
		} else {
			if content == "" {
				content = ex.Synthetic
			}
			meta["attachment_kind"] = ex.Kind
			meta["attachment_file_id"] = att.URL
			if att.Size != 0 {
				meta["attachment_size"] = fmt.Sprintf("%d", att.Size)
			}
			if att.ContentType != "" {
				meta["attachment_mime"] = att.ContentType
			}
			if ex.Name != "" {
				meta["attachment_name"] = ex.Name
			}
		}
	}

	if content == "" {
		return
	}

	h.enqueue(ctx, content, meta)
}

// HandleInteraction processes a component (button) interaction.
func (h *Handler) HandleInteraction(ctx context.Context, i *discordgo.Interaction) {
	if i == nil || i.Type != discordgo.InteractionMessageComponent {
		return
	}
	data := i.MessageComponentData()
	switch {
	case mcpchan.PermBtnRe.MatchString(data.CustomID):
		h.handlePermInteraction(ctx, i, data.CustomID)
	case actBtnRe.MatchString(data.CustomID):
		h.handleActionInteraction(ctx, i, data.CustomID)
	default:
		h.ackInteraction(i, "")
	}
}

// interactionUser returns who clicked: Member.User in guilds, User in DMs.
func interactionUser(i *discordgo.Interaction) *discordgo.User {
	if i.Member != nil && i.Member.User != nil {
		return i.Member.User
	}
	return i.User
}

// ackInteraction acknowledges an interaction; with text it answers with an
// ephemeral note (visible only to the clicker), otherwise a silent deferred
// update.
func (h *Handler) ackInteraction(i *discordgo.Interaction, text string) {
	if text == "" {
		_ = h.Session.InteractionRespond(i, &discordgo.InteractionResponse{
			Type: discordgo.InteractionResponseDeferredMessageUpdate,
		})
		return
	}
	_ = h.Session.InteractionRespond(i, &discordgo.InteractionResponse{
		Type: discordgo.InteractionResponseChannelMessageWithSource,
		Data: &discordgo.InteractionResponseData{
			Content: text,
			Flags:   discordgo.MessageFlagsEphemeral,
		},
	})
}

// updateInteractionMessage answers the interaction by rewriting the message
// that carried the buttons: new content, components cleared — so the question
// cannot be answered twice.
func (h *Handler) updateInteractionMessage(i *discordgo.Interaction, content string) error {
	return h.Session.InteractionRespond(i, &discordgo.InteractionResponse{
		Type: discordgo.InteractionResponseUpdateMessage,
		Data: &discordgo.InteractionResponseData{
			Content:    content,
			Components: []discordgo.MessageComponent{},
		},
	})
}

// handleActionInteraction processes a click on a reply's option button: it
// recovers the chosen label from the message's own components, clears the
// buttons, and relays the choice to Claude as an ordinary inbound message —
// the exact analog of telegram's callback-query path.
func (h *Handler) handleActionInteraction(ctx context.Context, i *discordgo.Interaction, customID string) {
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		h.ackInteraction(i, "")
		return
	}
	user := interactionUser(i)
	if user == nil {
		h.ackInteraction(i, "")
		return
	}
	isGuild := i.GuildID != ""
	if !actorAllowed(acc, isGuild, i.ChannelID, user.ID) {
		h.ackInteraction(i, "Not authorized.")
		return
	}
	if i.Message == nil {
		h.ackInteraction(i, "This question expired.")
		return
	}
	label := buttonLabel(i.Message.Components, customID)
	if label == "" {
		h.ackInteraction(i, "Already answered.")
		return
	}

	// Answer by rewriting the carrying message: choice recorded inline, buttons
	// gone. A racing second click finds no components and gets "Already
	// answered" via the interaction-token failure path.
	if err := h.updateInteractionMessage(i, i.Message.Content+"\n\n→ "+label); err != nil {
		return
	}

	meta := map[string]string{
		"chat_id": i.ChannelID,
		"user":    userDisplay(user),
		"user_id": user.ID,
		"ts":      time.Now().UTC().Format("2006-01-02T15:04:05.000Z07:00"),
		"kind":    "button",
	}
	if err := h.relay(ctx, label, meta); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: deliver button choice failed: %v\n", err)
	}
}

// handlePermInteraction processes permission buttons (perm:allow|deny|more).
func (h *Handler) handlePermInteraction(ctx context.Context, i *discordgo.Interaction, customID string) {
	m := mcpchan.PermBtnRe.FindStringSubmatch(customID)
	if m == nil {
		h.ackInteraction(i, "")
		return
	}
	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		h.ackInteraction(i, "")
		return
	}
	user := interactionUser(i)
	if user == nil || !isAllowlisted(acc, user.ID) {
		h.ackInteraction(i, "Not authorized.")
		return
	}

	action, code := m[1], m[2]

	if action == "more" {
		h.mu.Lock()
		entry, ok := h.permCache[code]
		h.mu.Unlock()
		if !ok {
			h.ackInteraction(i, "Details no longer available.")
			return
		}
		d := entry.params
		expanded := fmt.Sprintf("🔐 Permission: %s\n\ntool_name: %s\ndescription: %s\ninput_preview:\n%s",
			d.ToolName, d.ToolName, d.Description, d.InputPreview)
		_ = h.Session.InteractionRespond(i, &discordgo.InteractionResponse{
			Type: discordgo.InteractionResponseUpdateMessage,
			Data: &discordgo.InteractionResponseData{
				Content: expanded,
				Components: []discordgo.MessageComponent{discordgo.ActionsRow{
					Components: []discordgo.MessageComponent{
						discordgo.Button{Label: "✅ Allow", Style: discordgo.SuccessButton, CustomID: "perm:allow:" + code},
						discordgo.Button{Label: "❌ Deny", Style: discordgo.DangerButton, CustomID: "perm:deny:" + code},
					},
				}},
			},
		})
		return
	}

	// allow / deny verdict. Peek the params (for a context-preserving outcome)
	// before claiming atomically so only the first answerer (button or text reply,
	// here or on another provider's relay) emits a verdict.
	h.mu.Lock()
	entry, hadEntry := h.permCache[code]
	h.mu.Unlock()

	if !h.claimPerm(code) {
		h.ackInteraction(i, "Already handled.")
		return
	}
	if n := h.Notifier(); n != nil {
		if err := n.SendVerdict(ctx, code, action); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: send verdict failed: %v\n", err)
		}
	}
	allow := action == "allow"
	// Keep the prompt's lead line so scrollback shows what was approved.
	outcome := "✅ Allowed"
	if !allow {
		outcome = "❌ Denied"
	}
	if hadEntry {
		outcome = mcpchan.PermVerdictLine(mcpchan.PermPromptText(entry.params), allow)
	}
	_ = h.updateInteractionMessage(i, outcome)
}

// permPromptText renders the collapsed permission prompt as a warm, plain-language
// ask that still surfaces the concrete target. Shared with telegram via mcpchan so
// both channels read the same. Full detail stays behind "See more".
func permPromptText(p mcpchan.PermissionRequestParams) string {
	return mcpchan.PermPromptText(p)
}

// OnPermissionRequest fans a permission request out to allowlisted DMs only.
func (h *Handler) OnPermissionRequest(ctx context.Context, p mcpchan.PermissionRequestParams) {
	if h.Session == nil {
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
	send := &discordgo.MessageSend{
		Content: permPromptText(p),
		Components: []discordgo.MessageComponent{discordgo.ActionsRow{
			Components: []discordgo.MessageComponent{
				discordgo.Button{Label: "See more", Style: discordgo.SecondaryButton, CustomID: "perm:more:" + p.RequestID},
				discordgo.Button{Label: "✅ Allow", Style: discordgo.SuccessButton, CustomID: "perm:allow:" + p.RequestID},
				discordgo.Button{Label: "❌ Deny", Style: discordgo.DangerButton, CustomID: "perm:deny:" + p.RequestID},
			},
		}},
	}
	for _, id := range acc.AllowFrom {
		ch, err := h.Session.UserChannelCreate(id)
		if err != nil {
			fmt.Fprintf(os.Stderr, "hotline: permission_request DM open to %s failed: %v\n", id, err)
			continue
		}
		recordDMChannel(h.Cfg.StateDir, ch.ID, id)
		if _, err := h.Session.ChannelMessageSendComplex(ch.ID, send); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: permission_request send to %s failed: %v\n", id, err)
		}
	}
}

// claimPerm atomically removes the cached entry for code and reports whether
// it was present.
func (h *Handler) claimPerm(code string) bool {
	h.mu.Lock()
	defer h.mu.Unlock()
	if _, ok := h.permCache[code]; !ok {
		return false
	}
	delete(h.permCache, code)
	return true
}

// purgePermLocked drops permission entries older than permCacheTTL.
func (h *Handler) purgePermLocked() {
	cutoff := time.Now().Add(-permCacheTTL)
	for code, e := range h.permCache {
		if e.at.Before(cutoff) {
			delete(h.permCache, code)
		}
	}
}

// replyPairing answers an unknown DM sender with a pairing code, using the
// shared access pairing store (same TTLs, caps, and re-prompt limits as
// telegram — the state file is just this provider's own access.json).
func (h *Handler) replyPairing(ctx context.Context, m *discordgo.Message, senderID, chatID string) {
	code, send, err := access.CreatePairing(h.Cfg.AccessFile, senderID, chatID)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: create pairing failed: %v\n", err)
		return
	}
	if !send {
		return
	}
	body := "Pairing required — the operator runs in their terminal:\n\n" + pairingInstruction(h.Cfg.BotName, code)
	if _, err := h.Session.ChannelMessageSendComplex(chatID, &discordgo.MessageSend{Content: body}); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: pairing reply failed: %v\n", err)
	}
	_ = ctx
}

// pairingInstruction is the terminal command the operator runs to approve a
// pairing against this provider's own access.json.
func pairingInstruction(instance, code string) string {
	cmd := "hotline pair " + code + " --provider discord"
	if instance != "" {
		cmd += ":" + instance
	}
	return cmd
}

// isMentioned reports whether the bot is @-mentioned in the message.
func (h *Handler) isMentioned(m *discordgo.Message) bool {
	for _, u := range m.Mentions {
		if u != nil && u.ID == h.BotUserID {
			return true
		}
	}
	return false
}

func (h *Handler) repliedToBot(m *discordgo.Message) bool {
	return m.ReferencedMessage != nil &&
		m.ReferencedMessage.Author != nil &&
		m.ReferencedMessage.Author.ID == h.BotUserID
}

// replyQuote sanitizes and truncates a quoted reply's text for safe inclusion
// as <channel> meta.
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

// actorAllowed reports whether the user who clicked a button may answer it.
// DMs require the sender to be allowlisted; guild channels require the channel
// to be configured and, if it restricts senders, the sender to be listed.
func actorAllowed(acc *access.Access, isGuild bool, chatID, senderID string) bool {
	if isGuild {
		g, ok := acc.Groups[chatID]
		if !ok {
			return false
		}
		return len(g.AllowFrom) == 0 || slices.Contains(g.AllowFrom, senderID)
	}
	return slices.Contains(acc.AllowFrom, senderID)
}

func isAllowlisted(a *access.Access, senderID string) bool {
	return slices.Contains(a.AllowFrom, senderID)
}

// userDisplay prefers the login username, falling back to the snowflake ID.
func userDisplay(u *discordgo.User) string {
	if u.Username != "" {
		return u.Username
	}
	return u.ID
}
