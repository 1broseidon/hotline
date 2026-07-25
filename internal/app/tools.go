package app

import (
	"context"
	"fmt"
	"html"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/1broseidon/hotline/internal/access"
	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
)

type Tools struct {
	srv *Server
	cfg *config.Config
	log *transcript.Logger
}

func NewTools(srv *Server, cfg *config.Config, log *transcript.Logger) *Tools {
	return &Tools{srv: srv, cfg: cfg, log: log}
}

func (t *Tools) Reply(ctx context.Context, in mcpchan.ReplyInput) (string, bool) {
	if !t.srv.chatAllowed(in.ChatID) {
		return "reply failed: device is not active", true
	}
	texts := nonBlankItems(in.Bubbles)
	if len(texts) == 0 && strings.TrimSpace(in.Text) != "" {
		texts = []string{in.Text}
	}
	// App contract is plain CommonMark: down-convert any markdownv2/html body
	// before it hits the wire (and the size probes below) so escape backslashes
	// and tag syntax don't leak onto the screen. "text" passes through.
	for i := range texts {
		texts[i] = ToCommonMark(texts[i], in.Format)
	}
	var files []*fileRef
	for _, path := range in.Files {
		ref, err := t.srv.importAttachment(path)
		if err != nil {
			return "reply failed: " + err.Error(), true
		}
		files = append(files, ref)
	}
	elements, err := validateElements(in.Elements)
	if err != nil {
		return "reply failed: " + err.Error(), true
	}
	if len(texts) == 0 && len(files) == 0 && len(elements) == 0 {
		return "reply failed: nothing to send", true
	}
	paced := true
	if acc, err := access.Load(t.cfg.AccessFile); err == nil {
		paced = acc.BubbleMode != "instant"
	}
	buttons := sanitizeButtons(in.Buttons)
	// Elements ride the last message of the reply — the last file if any, else
	// the last text bubble; with neither, a standalone element-only message
	// whose text is the E6 synthesized fallback join (old clients render it).
	elementsOnLastText := len(elements) > 0 && len(files) == 0 && len(texts) > 0
	elementsOnLastFile := len(elements) > 0 && len(files) > 0
	standaloneElements := len(elements) > 0 && len(texts) == 0 && len(files) == 0

	// E9: the complete element-carrying payload must fit the 16 KiB cap.
	// Probe with the widest possible seq/id so the check can never undercount,
	// and reject BEFORE any bubble is emitted (atomic failure).
	if len(elements) > 0 {
		var probe []byte
		switch {
		case elementsOnLastFile:
			probe = msgFrame(probeSeq, probeID, "", buttons, "", files[len(files)-1], elements)
		case elementsOnLastText:
			btn := buttons
			replyTo := ""
			if len(texts) == 1 {
				replyTo = in.ReplyTo
			}
			probe = msgFrame(probeSeq, probeID, texts[len(texts)-1], btn, replyTo, nil, elements)
		default: // standalone
			probe = msgFrame(probeSeq, probeID, synthesizedText(elements), buttons, in.ReplyTo, nil, elements)
		}
		if err := validatePayloadSize(probe); err != nil {
			return "reply failed: " + err.Error(), true
		}
	}

	var ids []string
	recordElements := func(seq uint64, els []Element) string {
		id := fmt.Sprintf("a-%d", seq)
		if len(els) > 0 {
			t.srv.elIndex.record(id, els)
		}
		return id
	}
	for i, text := range texts {
		if paced {
			t.srv.emitTransient(func(seq uint64) []byte { return typingFrame(seq, true) })
			if sleepCtx(ctx, bubbleDelay(text)) {
				break
			}
		}
		var btn []string
		if i == len(texts)-1 && len(files) == 0 {
			btn = buttons
		}
		replyTo := ""
		if i == 0 {
			replyTo = in.ReplyTo
		}
		var els []Element
		if i == len(texts)-1 && elementsOnLastText {
			els = elements
		}
		seq := t.srv.emit(func(seq uint64) []byte { return msgFrame(seq, fmt.Sprintf("a-%d", seq), text, btn, replyTo, nil, els) })
		ids = append(ids, recordElements(seq, els))
	}
	for i, file := range files {
		var btn []string
		if i == len(files)-1 {
			btn = buttons
		}
		var els []Element
		if i == len(files)-1 && elementsOnLastFile {
			els = elements
		}
		f := file
		seq := t.srv.emit(func(seq uint64) []byte { return msgFrame(seq, fmt.Sprintf("a-%d", seq), "", btn, "", f, els) })
		ids = append(ids, recordElements(seq, els))
	}
	if standaloneElements {
		// Standalone element-only message: text = synthesized fallback join
		// (E6) — old clients render it and push previews use it (E10).
		synth := synthesizedText(elements)
		seq := t.srv.emit(func(seq uint64) []byte { return msgFrame(seq, fmt.Sprintf("a-%d", seq), synth, buttons, in.ReplyTo, nil, elements) })
		ids = append(ids, recordElements(seq, elements))
	}
	if paced {
		t.srv.emitTransient(func(seq uint64) []byte { return typingFrame(seq, false) })
	}
	if t.log != nil {
		t.log.Append(transcript.Record{Dir: "out", ChatID: in.ChatID, Kind: "reply", MessageID: strings.Join(ids, ", "), Text: outboundText(in)})
	}
	return fmt.Sprintf("queued %d bubble(s) (ids: %s)", len(ids), strings.Join(ids, ", ")), false
}

func (t *Tools) React(_ context.Context, in mcpchan.ReactInput) (string, bool) {
	if !t.srv.chatAllowed(in.ChatID) {
		return "react failed: device is not active", true
	}
	if strings.TrimSpace(in.MessageID) == "" || strings.TrimSpace(in.Emoji) == "" {
		return "react failed: message_id and emoji are required", true
	}
	t.srv.emit(func(seq uint64) []byte { return reactFrame(seq, in.MessageID, in.Emoji, "") })
	return "reacted", false
}

func (t *Tools) EditMessage(_ context.Context, in mcpchan.EditInput) (string, bool) {
	if !t.srv.chatAllowed(in.ChatID) {
		return "edit_message failed: device is not active", true
	}
	if strings.TrimSpace(in.MessageID) == "" {
		return "edit_message failed: message_id is required", true
	}
	elements, err := validateElements(in.Elements)
	if err != nil {
		return "edit_message failed: " + err.Error(), true
	}
	if strings.TrimSpace(in.Text) == "" && len(elements) == 0 {
		return "edit_message failed: text or elements is required", true
	}
	// E6: an element-only edit (empty text + elements) carries the synthesized
	// fallback join as its text — old clients render it; the app suppresses it.
	// A real body is down-converted to the app's CommonMark contract, mirroring
	// the reply path; the synthesized fallback join is already plain text.
	wireText := ToCommonMark(in.Text, in.Format)
	if strings.TrimSpace(in.Text) == "" && len(elements) > 0 {
		wireText = synthesizedText(elements)
	}
	if len(elements) > 0 {
		// E9: complete payload cap, probed with the widest seq.
		if err := validatePayloadSize(editFrame(probeSeq, in.MessageID, wireText, elements)); err != nil {
			return "edit_message failed: " + err.Error(), true
		}
		// E8 belt: the id-matched merge must never grow the message past the
		// element cap.
		if err := t.srv.elIndex.applyEdit(in.MessageID, elements); err != nil {
			return "edit_message failed: " + err.Error(), true
		}
	}
	t.srv.emit(func(seq uint64) []byte { return editFrame(seq, in.MessageID, wireText, elements) })
	return fmt.Sprintf("edited (id: %s)", in.MessageID), false
}

func (t *Tools) DownloadAttachment(_ context.Context, in mcpchan.DownloadInput) (string, bool) {
	rec, ok := t.srv.blobs.resolve(in.FileID)
	if !ok {
		return "download_attachment failed: transfer not found", true
	}
	return rec.Path, false
}

func (t *Tools) PublishArtifact(_ context.Context, in mcpchan.PublishInput) (string, bool) {
	if !t.srv.chatAllowed(in.ChatID) {
		return "publish failed: device is not active", true
	}
	info, err := os.Stat(in.Path)
	if err != nil || !info.Mode().IsRegular() || !strings.EqualFold(filepath.Ext(in.Path), ".html") || info.Size() <= 0 || info.Size() > 1<<20 {
		return "publish failed: artifact must be one HTML file no larger than 1 MiB", true
	}
	data, err := os.ReadFile(in.Path)
	if err != nil || !utf8.Valid(data) {
		return "publish failed: artifact must be UTF-8 HTML", true
	}
	blob, err := t.srv.blobs.register(in.Path, "text/html; charset=utf-8")
	if err != nil {
		return "publish failed: " + err.Error(), true
	}
	// Prefer the document's own <title> so the app card + apps drawer read by
	// name, not filename. Fall back to the filename when there's no usable title.
	title := htmlDocTitle(data)
	if title == "" {
		title = filepath.Base(in.Path)
	}
	var id string
	t.srv.emit(func(seq uint64) []byte {
		id = fmt.Sprintf("a-%d", seq)
		return artifactMsgFrame(seq, id, "Published artifact: "+title, &artifactRef{Title: title, Mime: "text/html", Size: info.Size(), Sandbox: "interactive-html-v1", Xfer: blob.Xfer})
	})
	return fmt.Sprintf("published artifact (id: %s)", id), false
}

// htmlDocTitle extracts the trimmed, entity-decoded text of the first <title>
// element, collapsing internal whitespace and capping length. Returns "" when
// there is no non-empty title so the caller can fall back to the filename.
func htmlDocTitle(data []byte) string {
	s := string(data)
	lower := strings.ToLower(s)
	open := strings.Index(lower, "<title")
	if open < 0 {
		return ""
	}
	gt := strings.IndexByte(s[open:], '>')
	if gt < 0 {
		return ""
	}
	start := open + gt + 1
	end := strings.Index(lower[start:], "</title>")
	if end < 0 {
		return ""
	}
	title := html.UnescapeString(s[start : start+end])
	title = strings.Join(strings.Fields(title), " ")
	if len(title) > 120 {
		title = strings.TrimSpace(title[:120])
	}
	return title
}

func outboundText(in mcpchan.ReplyInput) string {
	if countNonBlank(in.Bubbles) > 0 {
		return strings.Join(nonBlankItems(in.Bubbles), "\n")
	}
	return in.Text
}
