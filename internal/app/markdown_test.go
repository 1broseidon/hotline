package app

import "testing"

func TestToCommonMark(t *testing.T) {
	// The exact string from tonight's bug (markdownv2 sent to the app channel).
	bugIn := "test 2: *formatting* — this bubble is markdown\\. you should see " +
		"**bold**, _italic_, `inline code`, and a [link](https://relay.hotline.dev)\\. " +
		"if you're seeing asterisks and backslashes instead, that's a fix"
	bugWant := "test 2: **formatting** — this bubble is markdown. you should see " +
		"**bold**, *italic*, `inline code`, and a [link](https://relay.hotline.dev). " +
		"if you're seeing asterisks and backslashes instead, that's a fix"

	tests := []struct {
		name   string
		text   string
		format string
		want   string
	}{
		// format=text: never touched, even with literal markup characters.
		{"text passthrough", "plain *stars* and \\.dot", "text", "plain *stars* and \\.dot"},
		{"text literal asterisks", "2 * 3 * 4 = 24", "text", "2 * 3 * 4 = 24"},
		{"empty format is passthrough", "*not converted*", "", "*not converted*"},
		{"unknown format is passthrough", "*x*", "somethingelse", "*x*"},

		// mdv2 escape removal.
		{"escape removal", "a\\.b\\!c\\-d", "markdownv2", "a.b!c-d"},
		{"escaped literal star", "a \\* b", "markdownv2", "a * b"},
		{"escaped backslash collapses", "path\\\\to", "markdownv2", "path\\to"},
		{"lone backslash before nonspecial kept", "C:\\path", "markdownv2", "C:\\path"},

		// mdv2 syntax mapping.
		{"bold", "*bold*", "markdownv2", "**bold**"},
		{"italic", "_italic_", "markdownv2", "*italic*"},
		{"underline to italic", "__underline__", "markdownv2", "*underline*"},
		{"strikethrough", "~struck~", "markdownv2", "~~struck~~"},
		{"already double bold passthrough", "**bold**", "markdownv2", "**bold**"},
		{"spoiler drops pipes", "a ||secret|| b", "markdownv2", "a secret b"},
		{"lone pipe is literal", "a | b", "markdownv2", "a | b"},

		// Code spans / pre blocks: escapes and syntax preserved verbatim.
		{"escapes inside inline code preserved", "`a\\.b*c*`", "markdownv2", "`a\\.b*c*`"},
		{"escapes inside pre preserved", "```\nx\\.y _z_\n```", "markdownv2", "```\nx\\.y _z_\n```"},
		{"code span passthrough", "before `code` after", "markdownv2", "before `code` after"},
		{"pre with lang passthrough", "```go\nfmt.Println()\n```", "markdownv2", "```go\nfmt.Println()\n```"},

		// Links untouched; underscores in URL not mangled.
		{"link passthrough", "see [label](https://x.com)", "markdownv2", "see [label](https://x.com)"},
		{"link url with underscores", "[a](https://x.com/a_b_c)", "markdownv2", "[a](https://x.com/a_b_c)"},
		{"link escaped url", "[a](https://x.com/foo\\.bar)", "markdownv2", "[a](https://x.com/foo.bar)"},
		{"stray bracket literal", "an [unclosed bracket", "markdownv2", "an [unclosed bracket"},

		// Nested / adjacent entities.
		{"adjacent bold italic", "*b*_i_", "markdownv2", "**b***i*"},
		{"bold then code", "*b*`c`", "markdownv2", "**b**`c`"},

		// The bug.
		{"tonight's bug string", bugIn, "markdownv2", bugWant},

		// HTML conversion.
		{"html bold", "<b>x</b>", "html", "**x**"},
		{"html strong", "<strong>x</strong>", "html", "**x**"},
		{"html italic em", "<i>a</i><em>b</em>", "html", "*a**b*"},
		{"html underline to italic", "<u>x</u>", "html", "*x*"},
		{"html strike", "<s>x</s>", "html", "~~x~~"},
		{"html code", "<code>x</code>", "html", "`x`"},
		{"html entity unescape", "a &amp; b &lt;c&gt;", "html", "a & b <c>"},
		{"html link", `<a href="https://x.com">label</a>`, "html", "[label](https://x.com)"},
		{"html unknown tag stripped", "<span class='y'>keep</span>", "html", "keep"},
		{"html pre with nested code", "<pre><code>line\\.one</code></pre>", "html", "```\nline\\.one\n```"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := ToCommonMark(tc.text, tc.format); got != tc.want {
				t.Errorf("ToCommonMark(%q, %q)\n  got:  %q\n  want: %q", tc.text, tc.format, got, tc.want)
			}
		})
	}
}
