package signal

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/provider"
	"github.com/1broseidon/hotline/internal/transcript"
)

// Handler processes the daemon's receive events: it gates on the sender
// (allowlist by E.164, same pairing-code flow as the other adapters), relays
// inbound messages to Claude, answers pending permission requests from text
// replies, and maps bare-number answers back to the last reply's numbered
// options. It mirrors the telegram/discord Handler structure: same access
// model, same coalescing, same permission claim semantics.
type Handler struct {
	Client  *Client
	Cfg     *config.Config
	Log     *transcript.Logger
	Options *optionStore

	notifierMu sync.RWMutex
	notifier   provider.InboundSink

	mu        sync.Mutex
	permCache map[string]permEntry

	// Inbound coalescing (see coalesce.go).
	coalMu          sync.Mutex
	buffers         map[string]*chatBuffer
	coalesceWindow  time.Duration
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

// NewHandler builds a Handler sharing the Tools' option store.
func NewHandler(c *Client, cfg *config.Config, log *transcript.Logger, opts *optionStore) *Handler {
	return &Handler{
		Client:          c,
		Cfg:             cfg,
		Log:             log,
		Options:         opts,
		permCache:       make(map[string]permEntry),
		buffers:         make(map[string]*chatBuffer),
		coalesceWindow:  defaultCoalesceWindow,
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

// HandleEvent processes one SSE event from the daemon's event stream.
func (h *Handler) HandleEvent(ctx context.Context, ev sseEvent) {
	if ev.Event != "receive" || ev.Data == "" {
		return
	}
	var payload receivePayload
	if err := json.Unmarshal([]byte(ev.Data), &payload); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: signal event decode failed: %v\n", err)
		return
	}
	// The stream can carry every local account's traffic; only ours matters.
	if payload.Account != "" && h.Client.Account != "" && payload.Account != h.Client.Account {
		return
	}
	h.HandleEnvelope(ctx, &payload.Envelope)
}

// HandleEnvelope normalizes and relays one envelope. Receipts, typing
// notifications, sync messages, and reactions carry no dataMessage.message —
// they are ignored (except attachments-only messages, which relay a
// synthetic marker).
func (h *Handler) HandleEnvelope(ctx context.Context, e *envelope) {
	dm := e.DataMessage
	if dm == nil {
		return
	}
	senderID := e.sender()
	if senderID == "" || senderID == h.Client.Account {
		return // never relay our own (synced) messages
	}

	acc, err := access.Load(h.Cfg.AccessFile)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: access load failed: %v\n", err)
		return
	}

	chatID := e.chatID()
	isGroup := strings.HasPrefix(chatID, groupChatPrefix)
	text := dm.Message

	// Permission text-reply intercept: a gate-approved sender answering a
	// pending permission request. Checked before the gate's pairing side
	// effects and before relaying — same order as telegram/discord.
	if mm := mcpchan.PermReplyRe.FindStringSubmatch(text); mm != nil && isAllowlisted(acc, senderID) {
		behavior := mcpchan.BehaviorFromYesNo(mm[1])
		code := strings.ToLower(mm[2])
		if !h.claimPerm(code) {
			h.react(ctx, chatID, senderID, dm.Timestamp, "🤷")
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
		h.react(ctx, chatID, senderID, dm.Timestamp, emoji)
		return
	}

	in := access.GateInput{
		IsGroup:      isGroup,
		ChatID:       chatID,
		SenderID:     senderID,
		Text:         text,
		RepliedToBot: dm.Quote != nil && dm.Quote.AuthorNumber == h.Client.Account,
	}

	switch access.Gate(acc, in) {
	case access.Drop:
		return
	case access.Pair:
		h.replyPairing(ctx, senderID, chatID)
		return
	case access.Allow:
		// fall through
	}

	// Numbered-option answer: a bare number picks the matching option from
	// the last buttons-degraded reply, relayed as a button choice (the label,
	// not the digit) — the Signal analog of a button tap.
	if label := h.Options.answer(chatID, text); label != "" {
		meta := map[string]string{
			"chat_id": chatID,
			"user":    SafeName(nonEmpty(e.SourceName, senderID)),
			"user_id": senderID,
			"ts":      tsRFC3339(dm.Timestamp),
			"kind":    "button",
		}
		if err := h.relay(ctx, label, meta); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: deliver option choice failed: %v\n", err)
		}
		return
	}

	if acc.AckReaction != "" {
		h.react(ctx, chatID, senderID, dm.Timestamp, acc.AckReaction)
	}

	content := text
	meta := map[string]string{
		"chat_id":    chatID,
		"user":       SafeName(nonEmpty(e.SourceName, senderID)),
		"user_id":    senderID,
		"ts":         tsRFC3339(dm.Timestamp),
		"message_id": inboundMessageID(dm.Timestamp, senderID),
	}

	// Quoted-reply context, mirroring the other adapters.
	if q := dm.Quote; q != nil {
		author := q.AuthorNumber
		meta["reply_to_message_id"] = inboundMessageID(q.ID, author)
		if author == h.Client.Account {
			meta["reply_to_from"] = "you"
		} else if author != "" {
			meta["reply_to_from"] = SafeName(author)
		}
		if qt := replyQuote(q.Text); qt != "" {
			meta["reply_to_text"] = qt
		}
	}

	// Attachments: images are eagerly fetched from the daemon so Claude can
	// Read them immediately; everything else is surfaced lazily by id (the
	// file_id the download_attachment tool accepts, which also carries the
	// chat so getAttachment can address it).
	if len(dm.Attachments) > 0 {
		att := dm.Attachments[0]
		ex := extract(att)
		if ex.IsPhoto {
			if content == "" {
				content = "(photo)"
			}
			if path, err := h.fetchToInbox(ctx, chatID, att); err != nil {
				fmt.Fprintf(os.Stderr, "hotline: attachment fetch failed: %v\n", err)
			} else {
				meta["image_path"] = path
			}
		} else {
			if content == "" {
				content = ex.Synthetic
			}
			meta["attachment_kind"] = ex.Kind
			meta["attachment_file_id"] = attachmentFileID(att.ID, chatID)
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

// fetchToInbox pulls an attachment's bytes through the daemon's getAttachment
// command and writes them to the inbox.
func (h *Handler) fetchToInbox(ctx context.Context, chatID string, att attachment) (string, error) {
	if att.Size > maxDownloadBytes {
		return "", fmt.Errorf("file too large to download: %d bytes (max 50MB)", att.Size)
	}
	data, err := h.Client.GetAttachment(ctx, chatID, att.ID)
	if err != nil {
		return "", err
	}
	return saveToInbox(h.Cfg.InboxDir, data, att.Filename, att.ContentType)
}

// attachmentFileID packs an attachment id with its chat so the
// download_attachment tool can later address getAttachment correctly.
func attachmentFileID(id, chatID string) string {
	return id + "|" + chatID
}

// parseAttachmentFileID reverses attachmentFileID. A bare id yields chatID "".
func parseAttachmentFileID(fileID string) (id, chatID string) {
	id, chatID, _ = strings.Cut(fileID, "|")
	return id, chatID
}

// react best-effort adds an emoji reaction to the message sent by author at
// timestamp ts.
func (h *Handler) react(ctx context.Context, chatID, author string, ts int64, emoji string) {
	if err := h.Client.SendReaction(ctx, chatID, emoji, author, ts); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: signal reaction failed: %v\n", err)
	}
}

// OnPermissionRequest fans a permission request out to allowlisted numbers as
// a plain-text prompt. Signal has no buttons, so the answer path is the text
// reply the other adapters also accept: "yes <code>" / "no <code>".
func (h *Handler) OnPermissionRequest(ctx context.Context, p mcpchan.PermissionRequestParams) {
	if h.Client == nil {
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
	body := fmt.Sprintf("🔐 Permission: %s\n%s\n\n%s\n\nReply \"yes %s\" to allow or \"no %s\" to deny.",
		p.ToolName, p.Description, p.InputPreview, p.RequestID, p.RequestID)
	for _, num := range acc.AllowFrom {
		if _, err := h.Client.Send(ctx, num, body, nil, 0); err != nil {
			fmt.Fprintf(os.Stderr, "hotline: permission_request send to %s failed: %v\n", num, err)
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
// shared access pairing store (same TTLs, caps, and re-prompt limits as the
// other adapters — the state file is just this provider's own access.json).
func (h *Handler) replyPairing(ctx context.Context, senderID, chatID string) {
	code, send, err := access.CreatePairing(h.Cfg.AccessFile, senderID, chatID)
	if err != nil {
		fmt.Fprintf(os.Stderr, "hotline: create pairing failed: %v\n", err)
		return
	}
	if !send {
		return
	}
	body := "Pairing required — the operator runs in their terminal:\n\n" + pairingInstruction(h.Cfg.BotName, code)
	if _, err := h.Client.Send(ctx, chatID, body, nil, 0); err != nil {
		fmt.Fprintf(os.Stderr, "hotline: pairing reply failed: %v\n", err)
	}
}

// pairingInstruction is the terminal command the operator runs to approve a
// pairing against this provider's own access.json.
func pairingInstruction(instance, code string) string {
	cmd := "hotline pair " + code + " --provider signal"
	if instance != "" {
		cmd += ":" + instance
	}
	return cmd
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

func isAllowlisted(a *access.Access, senderID string) bool {
	return slices.Contains(a.AllowFrom, senderID)
}
