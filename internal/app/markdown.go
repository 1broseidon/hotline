package app

import (
	"html"
	"regexp"
	"strings"
)

// ToCommonMark translates a reply/edit body from the reply tool's `format` into
// the plain CommonMark the app channel renders client-side.
//
// The app is not Telegram: it consumes CommonMark-ish markdown, so the two
// Telegram-native formats have to be down-converted or their escape backslashes
// and tag/entity syntax leak onto the screen (the mdv2 "\." bug). The contract:
//
//   - "text":        pass through unchanged.
//   - "markdownv2":  strip mdv2 escape backslashes and map syntax to CommonMark.
//   - "html":        convert the basic Telegram HTML tag set to CommonMark.
//   - anything else: treated as plain text (passed through).
//
// This is a pragmatic bridge translator, not a full parser. The load-bearing
// invariants are: never leak an escape backslash, and never corrupt a code span
// or pre block.
func ToCommonMark(text, format string) string {
	switch strings.ToLower(strings.TrimSpace(format)) {
	case "markdownv2":
		return markdownV2ToCommonMark(text)
	case "html":
		return htmlToCommonMark(text)
	default:
		return text
	}
}

// mdv2Specials is the exact set MarkdownV2 requires callers to backslash-escape.
// The inverse (this converter) drops a backslash before any of them. Kept in
// sync with telegram.markdownV2Specials.
const mdv2Specials = "_*[]()~`>#+-=|{}.!"

func isMdv2Special(c byte) bool {
	return strings.IndexByte(mdv2Specials, c) >= 0
}

// markdownV2ToCommonMark is a single-pass tokenizer that tracks code-span / pre
// state. Inside code and pre it copies verbatim (mdv2 does not process escapes
// there, so neither do we); everywhere else it unescapes and maps delimiters.
func markdownV2ToCommonMark(s string) string {
	var b strings.Builder
	b.Grow(len(s))
	i, n := 0, len(s)
	for i < n {
		c := s[i]
		switch c {
		case '`':
			// Code span (single) or pre block (triple): copy verbatim.
			if strings.HasPrefix(s[i:], "```") {
				b.WriteString("```")
				i += 3
				if end := strings.Index(s[i:], "```"); end >= 0 {
					b.WriteString(s[i : i+end])
					b.WriteString("```")
					i += end + 3
				} else {
					b.WriteString(s[i:])
					i = n
				}
			} else {
				b.WriteByte('`')
				i++
				if end := strings.IndexByte(s[i:], '`'); end >= 0 {
					b.WriteString(s[i : i+end])
					b.WriteByte('`')
					i += end + 1
				} else {
					b.WriteString(s[i:])
					i = n
				}
			}
		case '\\':
			// Unescape: "\x" -> "x" for any mdv2 special (and a literal "\\").
			// A backslash before a non-special is a genuine backslash; keep it.
			if i+1 < n {
				next := s[i+1]
				if isMdv2Special(next) || next == '\\' {
					b.WriteByte(next)
					i += 2
					continue
				}
			}
			b.WriteByte('\\')
			i++
		case '*':
			// bold delimiter (any run length) -> CommonMark **.
			for i < n && s[i] == '*' {
				i++
			}
			b.WriteString("**")
		case '_':
			// _italic_ and __underline__ both -> CommonMark * (italic; the
			// accepted degradation, CommonMark has no underline).
			for i < n && s[i] == '_' {
				i++
			}
			b.WriteByte('*')
		case '~':
			// ~strikethrough~ -> CommonMark ~~.
			for i < n && s[i] == '~' {
				i++
			}
			b.WriteString("~~")
		case '|':
			// ||spoiler|| -> drop the pipes (plain text). A lone | is literal.
			run := 0
			for i < n && s[i] == '|' {
				i++
				run++
			}
			if run == 1 {
				b.WriteByte('|')
			}
		case '[':
			// [label](url) passes through: convert the label, leave the URL
			// untouched (only strip escapes so no backslash leaks).
			if label, url, end, ok := matchMdLink(s, i); ok {
				b.WriteByte('[')
				b.WriteString(markdownV2ToCommonMark(label))
				b.WriteString("](")
				b.WriteString(stripMdv2Escapes(url))
				b.WriteByte(')')
				i = end
			} else {
				b.WriteByte('[')
				i++
			}
		default:
			b.WriteByte(c)
			i++
		}
	}
	return b.String()
}

// matchMdLink recognizes a well-formed [label](url) starting at s[i]=='['. It
// stops at unescaped ']' / ')' and bails on any newline, so a stray bracket
// falls through to being emitted literally.
func matchMdLink(s string, i int) (label, url string, end int, ok bool) {
	n := len(s)
	j := i + 1
	for j < n {
		if s[j] == '\\' && j+1 < n {
			j += 2
			continue
		}
		if s[j] == ']' || s[j] == '\n' {
			break
		}
		j++
	}
	if j >= n || s[j] != ']' || j+1 >= n || s[j+1] != '(' {
		return "", "", 0, false
	}
	label = s[i+1 : j]
	k := j + 2
	for k < n {
		if s[k] == '\\' && k+1 < n {
			k += 2
			continue
		}
		if s[k] == ')' || s[k] == '\n' {
			break
		}
		k++
	}
	if k >= n || s[k] != ')' {
		return "", "", 0, false
	}
	return label, s[j+2 : k], k + 1, true
}

// stripMdv2Escapes removes escape backslashes without mapping any delimiter —
// used for link URLs, where the text is literal but must not carry a stray "\".
func stripMdv2Escapes(s string) string {
	if !strings.ContainsRune(s, '\\') {
		return s
	}
	var b strings.Builder
	b.Grow(len(s))
	for i := 0; i < len(s); i++ {
		if s[i] == '\\' && i+1 < len(s) {
			if next := s[i+1]; isMdv2Special(next) || next == '\\' {
				b.WriteByte(next)
				i++
				continue
			}
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

var htmlTagRe = regexp.MustCompile(`<[^>]+>`)

// htmlToCommonMark converts the basic Telegram HTML tag set to CommonMark,
// unescapes entities in the text runs, and strips unknown tags while keeping
// their inner text. Inside <pre> the inner tags are stripped (a nested <code>
// is common) and only the closing </pre> matters, so the fenced block is clean.
func htmlToCommonMark(s string) string {
	var b strings.Builder
	var aHrefs []string
	preDepth := 0
	last := 0
	for _, loc := range htmlTagRe.FindAllStringIndex(s, -1) {
		b.WriteString(html.UnescapeString(s[last:loc[0]]))
		last = loc[1]
		name, closing, attrs := parseHTMLTag(s[loc[0]:loc[1]])

		if preDepth > 0 {
			if name == "pre" && closing {
				preDepth--
				b.WriteString("\n```")
			}
			continue
		}

		switch name {
		case "b", "strong":
			b.WriteString("**")
		case "i", "em":
			b.WriteString("*")
		case "u":
			b.WriteString("*")
		case "s", "strike", "del":
			b.WriteString("~~")
		case "code":
			b.WriteByte('`')
		case "pre":
			if !closing {
				preDepth++
				b.WriteString("```\n")
			}
		case "a":
			if closing {
				href := ""
				if len(aHrefs) > 0 {
					href = aHrefs[len(aHrefs)-1]
					aHrefs = aHrefs[:len(aHrefs)-1]
				}
				b.WriteString("](")
				b.WriteString(href)
				b.WriteByte(')')
			} else {
				aHrefs = append(aHrefs, attrHref(attrs))
				b.WriteByte('[')
			}
		default:
			// Unknown tag: strip it, keep the inner text.
		}
	}
	b.WriteString(html.UnescapeString(s[last:]))
	return b.String()
}

// parseHTMLTag splits "<name attrs>" or "</name>" into its lowercased name, a
// closing flag, and the raw attribute string.
func parseHTMLTag(tag string) (name string, closing bool, attrs string) {
	inner := strings.TrimSpace(tag[1 : len(tag)-1])
	inner = strings.TrimSpace(strings.TrimSuffix(inner, "/"))
	if strings.HasPrefix(inner, "/") {
		closing = true
		inner = strings.TrimSpace(inner[1:])
	}
	name = inner
	if sp := strings.IndexAny(inner, " \t\r\n"); sp >= 0 {
		name = inner[:sp]
		attrs = inner[sp+1:]
	}
	return strings.ToLower(name), closing, attrs
}

var hrefRe = regexp.MustCompile(`(?i)href\s*=\s*("([^"]*)"|'([^']*)'|(\S+))`)

func attrHref(attrs string) string {
	m := hrefRe.FindStringSubmatch(attrs)
	if m == nil {
		return ""
	}
	switch {
	case m[2] != "":
		return html.UnescapeString(m[2])
	case m[3] != "":
		return html.UnescapeString(m[3])
	default:
		return html.UnescapeString(m[4])
	}
}
