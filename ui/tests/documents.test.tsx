import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { MarkdownDocument } = await import("../src/components/Markdown");
const { isMarkdown } = await import("../src/components/SentPreview");

describe("a markdown file opens as a document", () => {
	test("a markdown file is known by its type or its name's ending, and a text file is not one", () => {
		expect(isMarkdown({ name: "notes.md", mimeType: "application/octet-stream" })).toBe(true);
		expect(isMarkdown({ name: "README.MARKDOWN", mimeType: "" })).toBe(true);
		expect(isMarkdown({ name: "plan", mimeType: "text/markdown" })).toBe(true);
		expect(isMarkdown({ name: "notes.txt", mimeType: "text/plain" })).toBe(false);
		expect(isMarkdown({ name: "md", mimeType: "text/plain" })).toBe(false);
	});

	test("its headings and rules stand as written, where a bubble makes them bold text", () => {
		const html = renderToStaticMarkup(<MarkdownDocument text={"# Voice model notes\n\n## Moshi\n\nFast.\n\n---\n\n- [x] tried it"} />);
		expect(html).toContain("<h1>Voice model notes</h1>");
		expect(html).toContain("<h2>Moshi</h2>");
		expect(html).toContain("<hr/>");
		expect(html).toContain('type="checkbox"');
	});

	test("it runs no HTML and loads no remote picture", () => {
		const html = renderToStaticMarkup(<MarkdownDocument text={'<script>alert(1)</script>\n\n<b onclick="x">bold</b>\n\n![a chart](https://tracker.example/pixel.png)'} />);
		expect(html).not.toContain("<script");
		expect(html).not.toContain("onclick");
		expect(html).not.toContain("<img");
		expect(html).toContain("a chart");
	});
});
