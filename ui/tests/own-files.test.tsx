import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { TranscriptEvent } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Transcript } = await import("../src/components/Transcript");

const picture = { kind: "image", name: "boarding-pass.png", path: "/tmp/boarding-pass.png", mimeType: "image/png", size: 193473, width: 800, height: 600 } as const;
const pdf = { kind: "file", name: "itinerary.pdf", path: "/tmp/itinerary.pdf", mimeType: "application/pdf", size: 30816 } as const;

function transcript(events: TranscriptEvent[]): string {
	return renderToStaticMarkup(<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} />);
}

describe("what you send", () => {
	test("a picture is drawn over your bubble in its own shape with its name and size under it, and a document is a row that says what it is", () => {
		const html = transcript([{ kind: "user", id: "u1", ts: 1, text: "Here's the pass and the itinerary.", attachments: [picture, pdf] }]);
		expect(html).toContain("picture-frame");
		expect(html).toContain("aspect-ratio:800 / 600");
		expect(html).toContain('<figcaption class="picture-caption"><span class="chip-name">boarding-pass.png</span><span class="shrink-0"> · 189 KB</span></figcaption>');
		expect(html).toContain('<span class="file-mark" aria-hidden="true">PDF</span><span class="file-name">itinerary.pdf</span>');
		expect(html).toContain("30 KB");
		expect(html.indexOf("own-files")).toBeLessThan(html.indexOf("said-me"));
		expect(html).toContain("Here&#x27;s the pass and the itinerary.");
	});

	test("a line of only files has no empty bubble", () => {
		const html = transcript([{ kind: "user", id: "u2", ts: 1, text: "", attachments: [picture] }]);
		expect(html).toContain("picture-frame");
		expect(html).not.toContain("said-me");
	});
});
