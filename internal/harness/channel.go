package harness

import (
	"html"
	"sort"
	"strings"
)

// channelAttrOrder is the preferred, stable ordering for the well-known meta
// keys the messaging providers stamp. It mirrors the attribute order Claude
// Code shows in its rendered <channel> block (source and chat_id first — the
// routing keys the agent must echo back into hotline_reply), so an OpenCode
// agent sees the same framing a Claude agent does. Any meta key not listed
// here is appended afterward in sorted order, so unknown keys are still carried
// deterministically.
var channelAttrOrder = []string{
	"source",
	"chat_id",
	"message_id",
	"user",
	"user_id",
	"ts",
	"kind",
	"element_msg",
	"element_id",
	"element_act",
	"element_value",
	"reply_to_message_id",
	"reply_to_from",
	"reply_to_text",
	"bubbles",
	"image_path",
	"attachment_file_id",
	"attachment_kind",
	"attachment_name",
}

// RenderChannel frames an inbound turn as the <channel …>content</channel>
// envelope agents read routing/context from. It exists so harnesses that only
// speak plain prompt text (OpenCode) deliver the SAME envelope Claude Code
// renders client-side from the claude/channel notification's (content, meta):
// the agent reads chat_id and source off the tag and echoes them back into the
// reply tool. Without it, meta is dropped and the agent has no chat_id to
// reply to (it hallucinates one).
//
// With no meta there is nothing to route, so the content is returned unchanged
// (no empty envelope). Attribute values are HTML-escaped so a meta value can't
// forge or break out of the tag.
func RenderChannel(in Inbound) string {
	if len(in.Meta) == 0 {
		return in.Content
	}

	var b strings.Builder
	b.WriteString("<channel")

	seen := make(map[string]bool, len(in.Meta))
	writeAttr := func(k string) {
		v, ok := in.Meta[k]
		if !ok || v == "" || seen[k] {
			return
		}
		seen[k] = true
		b.WriteByte(' ')
		b.WriteString(k)
		b.WriteString(`="`)
		b.WriteString(html.EscapeString(v))
		b.WriteByte('"')
	}

	for _, k := range channelAttrOrder {
		writeAttr(k)
	}
	// Any remaining (unknown) keys, sorted for determinism.
	rest := make([]string, 0)
	for k := range in.Meta {
		if !seen[k] && in.Meta[k] != "" {
			rest = append(rest, k)
		}
	}
	sort.Strings(rest)
	for _, k := range rest {
		writeAttr(k)
	}

	// element_action content is a box-generated one-line summary; strip any
	// control character so generated content can never fake or break the
	// channel framing (ERRATA E3 belt — the action parser is the primary
	// rejection point). Human message content is left untouched: real
	// multi-line messages are legitimate there.
	content := in.Content
	if in.Meta["kind"] == "element_action" {
		content = stripControlRunes(content)
	}

	// Fleet trust preamble (a2a-design-v2 §4, F12/H3): the marker is NOT added
	// here — it rides the injected CONTENT (see app.injectInbound), so it reaches
	// the default Claude Code path (which frames <channel> client-side from the
	// raw content, never through RenderChannel) AND the pi/opencode path (which
	// does call RenderChannel) with no double-marking. Framing the marker only
	// here would leave the live Claude path unmarked, the H3 blocker.
	b.WriteString(">\n")
	b.WriteString(content)
	b.WriteString("\n</channel>")
	return b.String()
}

// stripControlRunes replaces every rune below U+0020 with a space.
func stripControlRunes(s string) string {
	return strings.Map(func(r rune) rune {
		if r < 0x20 {
			return ' '
		}
		return r
	}, s)
}
