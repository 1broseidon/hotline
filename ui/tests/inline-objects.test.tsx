import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// The objects a message carries (docs/design/inline-premium.html rows 1-5),
// drawn by the bubble's own markdown, and expanded over the window.

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Markdown } = await import("../src/components/Markdown");
const { OpenFull, SaidBy } = await import("../src/components/Sheet");
const { FullView } = await import("../src/components/FullView");
const { agoOf, Speakers } = await import("../src/ui/OpenBand");
const { lit, mermaidConfig, factOf } = await import("../src/components/Mermaid");
const { dockBack } = await import("../src/dock");
type FullObject = import("../src/components/Sheet").FullObject;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let teardown: () => Promise<void>;
let clipboard: string[];
let opened: FullObject[];

beforeEach(async () => {
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	Object.assign(globalThis, { MutationObserver: dom.MutationObserver, HTMLElement: dom.HTMLElement });
	clipboard = [];
	opened = [];
	spyOn(dom.navigator.clipboard, "writeText").mockImplementation(async (text) => {
		clipboard.push(text);
	});
	act = (await import("react")).act;
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

/** When Juno said everything here: four minutes ago. */
const SAID = { personaId: "juno", ts: Date.now() - 4 * 60_000 };
const JUNO = { id: "juno", name: "Juno" };

/** A message as Juno's bubble draws it, where the window can open an object in the conversation's place. */
async function say(text: string, full = true) {
	await act(async () =>
		root.render(
			<div className="speech said-them">
				<SaidBy.Provider value={SAID}>
					<OpenFull.Provider value={full ? (object) => opened.push(object) : null}>
						<Markdown text={text} />
					</OpenFull.Provider>
				</SaidBy.Provider>
			</div>,
		),
	);
}

/** An opened object as the view draws it, without the sheet it came from (happy-dom lays nothing out to grow from). */
async function openView(object: import("../src/components/Sheet").FullObject, onClose: () => void = () => {}) {
	const { sheet: _, ...still } = object;
	await act(async () => root.render(null));
	await act(async () =>
		root.render(
			<Speakers.Provider value={(id) => (id === "juno" ? JUNO : undefined)}>
				<FullView object={still} onClose={onClose} />
			</Speakers.Provider>,
		),
	);
	await settle();
	return document.querySelector<HTMLElement>(".full-view")!;
}
const band = (view: HTMLElement) => view.querySelector<HTMLElement>(".open-band")!;

const sheet = (kind: string) => container.querySelector<HTMLElement>(`[data-sheet="${kind}"]`);
const label = (kind: string) => sheet(kind)?.querySelector(".sheet-label")?.textContent;
const keys = (kind: string) => [...(sheet(kind)?.querySelectorAll<HTMLButtonElement>(".sheet-key") ?? [])];
const key = (kind: string, name: string) => keys(kind).find((one) => one.textContent === name)!;
const settle = () => act(async () => new Promise((done) => setTimeout(done, 0)));
const cut = (kind: string) => sheet(kind)?.querySelector<HTMLButtonElement>(".sheet-cut") ?? null;

/** Lay every box out wider than its bubble, as a table or a line too long for it is. */
function tooWide() {
	const proto = dom.HTMLElement.prototype as unknown as HTMLElement;
	const was = { scroll: Object.getOwnPropertyDescriptor(proto, "scrollWidth"), client: Object.getOwnPropertyDescriptor(proto, "clientWidth") };
	Object.defineProperty(proto, "scrollWidth", { configurable: true, get: () => 900 });
	Object.defineProperty(proto, "clientWidth", { configurable: true, get: () => 500 });
	return () => {
		if (was.scroll) Object.defineProperty(proto, "scrollWidth", was.scroll);
		else delete (proto as { scrollWidth?: number }).scrollWidth;
		if (was.client) Object.defineProperty(proto, "clientWidth", was.client);
		else delete (proto as { clientWidth?: number }).clientWidth;
	};
}

const TABLE = ["| Database | License | p50 |", "| --- | --- | --- |", "| Qdrant | Apache 2.0 | 5–8 ms |", "| Weaviate | BSD-3 | 6 ms |", "| Milvus | Apache 2.0 | ~5 ms |"].join("\n");

describe("a table", () => {
	test("sits on one sheet that names nothing, with Copy and no Expand", async () => {
		await say(`Here is how they stack up:\n\n${TABLE}`);
		expect(label("table")).toBeUndefined();
		expect(sheet("table")?.hasAttribute("data-bare")).toBe(true);
		expect(keys("table").map((one) => one.textContent)).toEqual(["Copy"]);
		expect(sheet("table")?.querySelectorAll("tbody tr").length).toBe(3);
		// The bubble may take the column; the 0.47 rule is kept on the sheet.
		expect(container.querySelector(".speech .md-table")).not.toBeNull();
	});

	test("has no fill on any row: plain rows on the sheet, divided by hairlines", async () => {
		await say(TABLE);
		for (const part of sheet("table")!.querySelectorAll("tr, th, td")) expect((part as HTMLElement).getAttribute("style") ?? "").not.toContain("background");
		const css = readFileSync(new URL("../src/inline-objects.css", import.meta.url), "utf8");
		const rules = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].filter(([, selector]) => /\.md-table[^,{]*\b(tr|td|th)\b/.test(selector!));
		expect(rules.length).toBeGreaterThan(0);
		for (const [, selector, body] of rules) expect({ selector: selector!.trim(), fill: /background/.test(body!) }).toEqual({ selector: selector!.trim(), fill: false });
	});

	test("Copy puts the table on the clipboard as the Markdown it was written in", async () => {
		await say(`Here:\n\n${TABLE}\n\nThat's all.`);
		await act(async () => key("table", "Copy").click());
		expect(clipboard).toEqual([TABLE]);
		expect(key("table", "Copied")).toBeDefined();
	});

	test("in a quote, Copy gives the table without the quote's marks", async () => {
		await say(TABLE.split("\n").map((line) => `> ${line}`).join("\n"));
		await act(async () => key("table", "Copy").click());
		expect(clipboard).toEqual([TABLE]);
	});

	test("stands a column of numbers right, and only that one", async () => {
		await say(TABLE);
		const numeric = [...sheet("table")!.querySelectorAll("tbody tr:first-child td")].map((cell) => cell.className === "num");
		expect(numeric).toEqual([false, false, true]);
	});

	test("keeps a column of sentences at a reading width", async () => {
		await say("| Name | Hosting |\n| --- | --- |\n| Qdrant | Qdrant Cloud: 1 GB free forever; clusters from $80/mo, BYOC |\n| Milvus | Zilliz Cloud: free serverless; Hot, Warm and Cold tiers |");
		expect([...sheet("table")!.querySelectorAll("tbody tr:first-child td")].map((cell) => cell.className)).toEqual(["", "wide"]);
	});

	test("draws a source as its site's chip, not an underlined link", async () => {
		await say("| Name | Sources |\n| --- | --- |\n| Qdrant | [qdrant.tech](https://www.qdrant.tech/docs) <https://zilliz.com/cloud> |");
		const chips = [...sheet("table")!.querySelectorAll("a.site-chip")];
		expect(chips.map((chip) => chip.textContent)).toEqual(["qdrant.tech", "zilliz.com"]);
	});

	test("makes room for its sources: a chip is as wide as its words and never a share of its cell, so none runs past it", async () => {
		await say("| Database | Sources |\n| --- | --- |\n| Qdrant | [Qdrant Benchmarks](https://qdrant.tech/benchmarks/) |");
		const chip = sheet("table")!.querySelector<HTMLAnchorElement>("a.site-chip")!;
		expect(chip.querySelector(".site-chip-name")?.textContent).toBe("Qdrant Benchmarks");
		expect(chip.title).toBe("https://qdrant.tech/benchmarks/");
		const css = readFileSync(new URL("../src/inline-objects.css", import.meta.url), "utf8");
		const rules = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].filter(([, selector]) => /\.md-table td \.site-chip\s*$/.test(selector!.trim()));
		expect(rules.map(([, , body]) => /width:\s*max-content/.test(body!))).toContain(true);
		expect(rules.some(([, , body]) => /max-width:\s*\d+%/.test(body!))).toBe(false);
	});

	test("that fits its bubble is whole in it, with no cut and nothing to open", async () => {
		await say(TABLE);
		expect(sheet("table")?.hasAttribute("data-cut")).toBe(false);
		expect(cut("table")).toBeNull();
		await act(async () => sheet("table")!.click());
		expect(opened).toEqual([]);
	});

	test("wider than its bubble is cut, says its rows and columns, and a click anywhere on it opens it", async () => {
		const restore = tooWide();
		try {
			await say(TABLE);
			expect(sheet("table")?.getAttribute("data-cut")).toBe("wide");
			expect(cut("table")?.textContent).toBe("Table · 3 rows · 3 columns");
			await act(async () => sheet("table")!.querySelector("td")!.click());
			await act(async () => cut("table")!.click());
			expect(opened.map((one) => one.kind)).toEqual(["table", "table"]);
		} finally {
			restore();
		}
	});

	test("cut, keeps its head row clear of the fade: the head's height is measured onto the mask's layer that keeps it whole", async () => {
		const restore = tooWide();
		try {
			await say(TABLE);
			const box = sheet("table")!.querySelector<HTMLElement>(".md-table")!;
			expect(box.style.getPropertyValue("--cut-head")).toMatch(/^\d+(\.\d+)?px$/);
		} finally {
			restore();
		}
		const css = readFileSync(new URL("../src/inline-objects.css", import.meta.url), "utf8");
		expect(css).toMatch(/--keep-head:\s*linear-gradient\(black, black\) top \/ 100% var\(--cut-head/);
		for (const cut of ["wide", "wide tall"]) {
			const rule = css.match(new RegExp(`\\.sheet\\[data-cut="${cut}"\\] > \\.md-table \\{([^}]*)\\}`))?.[1] ?? "";
			expect({ cut, keepsHead: /mask:\s*var\(--keep-head\)/.test(rule) }).toEqual({ cut, keepsHead: true });
		}
	});

	test("is never cut where nothing can open it full screen", async () => {
		const restore = tooWide();
		try {
			await say(TABLE, false);
			expect(sheet("table")?.hasAttribute("data-cut")).toBe(false);
			expect(cut("table")).toBeNull();
		} finally {
			restore();
		}
	});
});

describe("a block of code", () => {
	test("sits on one sheet whose head names its language and file, with Copy and no Expand or Wrap", async () => {
		await say("```python setup.py\nfrom qdrant_client import QdrantClient\n```");
		expect(label("code")).toBe("Python · setup.py");
		expect(keys("code").map((one) => one.textContent)).toEqual(["Copy"]);
		expect(sheet("code")!.querySelector(".code-keyword")?.textContent).toBe("from");
		expect(cut("code")).toBeNull();
	});

	test("longer than twelve lines is cut, says how many, and opens on a click", async () => {
		const lines = Array.from({ length: 13 }, (_, at) => `line ${at + 1}`).join("\n");
		await say(`\`\`\`bash\n${lines}\n\`\`\``);
		expect(sheet("code")?.getAttribute("data-cut")).toBe("tall");
		expect(cut("code")?.textContent).toBe("13 lines");
		await act(async () => sheet("code")!.click());
		expect(opened.map((one) => one.kind)).toEqual(["code"]);
		await say("```bash\n" + lines.split("\n").slice(0, 12).join("\n") + "\n```");
		expect(cut("code")).toBeNull();
	});

	test("of one line too long for the bubble wraps in it, never cut; of more lines, it is cut and says so", async () => {
		const restore = tooWide();
		try {
			const long = `echo ${"a".repeat(300)}`;
			await say(`\`\`\`bash\n${long}\n\`\`\``);
			expect(sheet("code")?.hasAttribute("data-cut")).toBe(false);
			expect(cut("code")).toBeNull();
			const pre = sheet("code")!.querySelector("pre")!;
			expect(pre.hasAttribute("data-wrap")).toBe(true);
			expect(pre.textContent).toBe(long);
			await say(`\`\`\`bash\n${long}\necho done\n\`\`\``);
			expect(sheet("code")?.getAttribute("data-cut")).toBe("wide");
			expect(cut("code")?.textContent).toBe("2 lines, some long");
			expect(sheet("code")!.querySelector("pre")!.hasAttribute("data-wrap")).toBe(false);
		} finally {
			restore();
		}
	});

	test("carries its line numbers for the open view, which alone shows them", async () => {
		await say("```bash\none\ntwo\nthree\n```");
		const pre = sheet("code")!.querySelector("pre")!;
		expect([...pre.querySelectorAll(".code-line")].map((line) => line.getAttribute("data-n"))).toEqual(["1", "2", "3"]);
		const css = readFileSync(new URL("../src/inline-objects.css", import.meta.url), "utf8");
		const gutters = [...css.matchAll(/([^{}]+)\{[^}]*content:\s*attr\(data-n\)/g)].map(([, selector]) => selector!.trim());
		expect(gutters).toEqual([".full-view-object .code-line::before"]);
	});

	test("Copy takes the code as written, never its line numbers", async () => {
		const code = ["a = 1", "b = 2", "c = 3", "d = 4", "e = 5", "f = 6"].join("\n");
		await say(`\`\`\`python\n${code}\n\`\`\``);
		await act(async () => key("code", "Copy").click());
		expect(clipboard).toEqual([code]);
	});
});

describe("a quote", () => {
	test("is a plain quote with a neutral edge, no mark and no label", async () => {
		await say("From their docs:\n\n> Qdrant memory-maps the index.");
		expect(container.querySelector("blockquote")?.textContent?.trim()).toBe("Qdrant memory-maps the index.");
		expect(container.querySelector(".callout")).toBeNull();
		expect(container.querySelector("blockquote svg")).toBeNull();
	});

	for (const [marker, name] of [["WARNING", "Warning"], ["CAUTION", "Caution"]] as const) {
		test(`marked [!${marker}] is an amber callout with the warning mark, its kind its accessible name`, async () => {
			await say(`> [!${marker}]\n> A network disk turns the 5 ms p50 into 40.`);
			const callout = container.querySelector<HTMLElement>(".callout")!;
			expect(callout.dataset["alert"]).toBe(marker.toLowerCase());
			expect(callout.querySelector("svg.callout-icon")).not.toBeNull();
			expect(callout.getAttribute("aria-label")).toBe(name);
			expect(callout.querySelector(".callout-label")).toBeNull();
			expect(callout.querySelector(".callout-words p")?.textContent).toBe("A network disk turns the 5 ms p50 into 40.");
			expect(callout.textContent).not.toContain("[!");
		});
	}

	test("marked [!NOTE], [!TIP] or [!IMPORTANT] is its words as prose, without its marker: only a warning is a callout", async () => {
		for (const marker of ["NOTE", "TIP", "IMPORTANT"]) {
			await say(`> [!${marker}]\n>\n> Worth knowing.`);
			expect(container.querySelector(".callout")).toBeNull();
			expect(container.querySelector("blockquote")).toBeNull();
			expect([...container.querySelectorAll(".md p")].map((p) => p.textContent)).toEqual(["Worth knowing."]);
		}
	});
});

describe("headings and lists", () => {
	test("every level of heading is one heading, never a document's h1 to h6", async () => {
		await say("# Worth your attention\n\n### And this");
		expect([...container.querySelectorAll(".md-heading")].map((one) => one.textContent)).toEqual(["Worth your attention", "And this"]);
		expect(container.querySelector("h1, h2, h3, h4, h5, h6")).toBeNull();
	});

	test("a numbered list counts on from where it said it starts", async () => {
		await say("3. Start the container\n4. Create a collection");
		expect(container.querySelector("ol")?.getAttribute("style")).toContain("counter-reset: md-item 2");
	});
});

describe("a diagram", () => {
	test("that branches sits on one sheet whose head names its kind", async () => {
		const source = "flowchart LR\n  A[App] --> B[Embed query]\n  A --> C[Cache]";
		await say(`How a query flows:\n\n\`\`\`mermaid\n${source}\n\`\`\``);
		expect(label("diagram")).toBe("flowchart");
		// Not drawn here (mermaid's script is never run under test: tests/preload.ts), so its source shows, and keys wait for the drawing.
		expect(sheet("diagram")!.querySelector(".diagram-source")?.textContent).toBe(source);
		expect(keys("diagram")).toEqual([]);
	});

	test("that is only a straight line is said as one line of steps, with no sheet", async () => {
		await say("```mermaid\nflowchart LR\n  A[App] --> B[Embed query] --> C(Qdrant HNSW)\n  C --> D{Top k}\n```");
		expect(sheet("diagram")).toBeNull();
		const chain = container.querySelector(".flow-chain");
		expect([...(chain?.querySelectorAll(".flow-step") ?? [])].map((one) => one.textContent)).toEqual(["App", "Embed query", "Qdrant HNSW", "Top k"]);
		expect(chain?.getAttribute("aria-label")).toBe("App, then Embed query, then Qdrant HNSW, then Top k");
	});

	test("with a labelled edge, a loop or a style keeps its drawing", async () => {
		const { chainOf } = await import("../src/components/FlowChain");
		expect(chainOf("flowchart LR\n A -->|yes| B")).toBeNull();
		expect(chainOf("flowchart LR\n A --> B\n B --> A")).toBeNull();
		expect(chainOf("flowchart LR\n A --> B\n style A fill:#f00")).toBeNull();
		expect(chainOf("sequenceDiagram\n A->>B: hi")).toBeNull();
		expect(chainOf("graph TD\n A --> B")).toEqual(["A", "B"]);
	});

	const colours = {
		dark: true,
		ink: "#eef0f3",
		ink3: "#8b9099",
		line: "#ffffff17",
		lineStrong: "#ffffff29",
		canvas: "#15181b",
		raised: "#1c1f23",
		pop: "#24272b",
		accent: "#5fd3a5",
		accentSoft: "#5fd3a529",
	};

	test("is themed from the window's tokens, and nothing in the theme is the accent", () => {
		const config = mermaidConfig(colours);
		expect(config.theme).toBe("base");
		expect(config.securityLevel).toBe("strict");
		expect(config.themeVariables.primaryColor).toBe(colours.pop);
		expect(config.themeVariables.primaryTextColor).toBe(colours.ink);
		expect(config.themeVariables.lineColor).toBe(colours.ink3);
		const values = Object.values(config.themeVariables).map(String);
		expect(values.filter((value) => value === colours.accent || value === colours.accentSoft)).toEqual([]);
		expect(values.every((value) => !value.includes("oklch"))).toBe(true);
	});

	test("lights a node in the accent only where the source marks it", () => {
		const plain = "flowchart LR\n  A[App] --> B[Qdrant]";
		expect(lit(plain, colours)).toBe(plain);

		const marked = "flowchart LR\n  A[App] --> B[Qdrant]:::hot\n  classDef hot fill:#f96,stroke:#333\n  style A fill:#f9f\n  classDef default fill:#fff\n  linkStyle 0 stroke:#f00";
		expect(lit(marked, colours).split("\n")).toEqual([
			"flowchart LR",
			"  A[App] --> B[Qdrant]:::hot",
			`  classDef hot fill:${colours.accentSoft},stroke:${colours.accent},color:${colours.ink}`,
			`  style A fill:${colours.accentSoft},stroke:${colours.accent},color:${colours.ink}`,
			"  classDef default fill:#fff",
			"  linkStyle 0 stroke:#f00",
		]);
	});
});

describe("a bare link", () => {
	test("says its site and last step, and keeps the whole address as its href", async () => {
		await say("The rest: https://qdrant.tech/documentation/quickstart/ and https://example.com");
		const links = [...container.querySelectorAll<HTMLAnchorElement>(".md a")];
		expect(links.map((one) => [one.textContent, one.getAttribute("href")])).toEqual([
			["qdrant.tech/…/quickstart", "https://qdrant.tech/documentation/quickstart/"],
			["example.com", "https://example.com"],
		]);
	});
});

describe("the open view", () => {
	test("a cut table opens with its fact, the sentence that introduced it, who said it and the sheet it grows from", async () => {
		const restore = tooWide();
		await say(`Here are the **three** we looked at:\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		restore();
		expect(opened.length).toBe(1);
		const { sheet: from, ...rest } = opened[0]!;
		expect(rest).toEqual({ kind: "table", text: TABLE, fact: "Table · 3 rows × 3 columns", caption: "Here are the three we looked at:", from: SAID });
		expect(from).toBe(sheet("table")!);
	});

	test("wears the band: ← back, the teammate's face and name, when, the object as a fact and Copy with its word; no title, no Close", async () => {
		const restore = tooWide();
		await say(`Here are the three we looked at:\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		restore();
		const view = await openView(opened[0]!);
		const head = band(view);
		expect(head.querySelector(".band")).not.toBeNull();
		expect(head.querySelector("button")?.getAttribute("aria-label")).toBe("Back to the conversation");
		expect(head.querySelector(".avatar")).not.toBeNull();
		expect(head.querySelector(".open-band-name")?.textContent).toBe("Juno");
		expect([...head.querySelectorAll(".instrument")].map((one) => one.textContent)).toEqual(["· 4 min ago", "Table · 3 rows × 3 columns"]);
		expect([...head.querySelectorAll(".sheet-key")].map((one) => one.textContent)).toEqual(["Copy"]);
		expect(view.querySelector(".full-view-title, .full-view-head")).toBeNull();
		expect(view.getAttribute("aria-label")).toBe("Table · 3 rows × 3 columns");
		// Drawn whole and never cut.
		expect(view.querySelectorAll("tbody tr").length).toBe(3);
		expect(view.querySelector(".sheet-cut")).toBeNull();
	});

	test("captions the object with the sentence that introduced it, after the teammate's name, and has no caption when none did", async () => {
		const restore = tooWide();
		await say(`Here are the three we looked at:\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		await say(TABLE);
		await act(async () => cut("table")!.click());
		await say(`- one\n- two\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		restore();
		expect(opened.map((one) => one.caption)).toEqual(["Here are the three we looked at:", undefined, undefined]);
		const view = await openView(opened[0]!);
		expect(view.querySelector(".full-view-caption")?.textContent).toBe("Juno: Here are the three we looked at:");
		expect(view.querySelector(".full-view-caption-who")?.textContent).toBe("Juno:");
		const bare = await openView(opened[1]!);
		expect(bare.querySelector(".full-view-caption")).toBeNull();
	});

	test("puts what it stands in front of out of reach while open, and gives it back on closing", async () => {
		const restore = tooWide();
		await say(`Here:\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		restore();
		const { sheet: _, ...still } = opened[0]!;
		await act(async () => root.render(null));
		await act(async () =>
			root.render(
				<div>
					<div className="behind">the conversation</div>
					<FullView object={still} onClose={() => {}} />
				</div>,
			),
		);
		const behind = () => container.querySelector<HTMLElement>(".behind")!;
		expect(behind().inert).toBe(true);
		await act(async () => root.render(<div><div className="behind">the conversation</div></div>));
		expect(behind().inert).toBe(false);
	});

	test("← and Escape go back", async () => {
		const restore = tooWide();
		await say(`Here:\n\n${TABLE}`);
		await act(async () => cut("table")!.click());
		restore();
		let closed = 0;
		const view = await openView(opened[0]!, () => closed++);
		await act(async () => band(view).querySelector<HTMLButtonElement>("button")!.click());
		await settle();
		expect(closed).toBe(1);
		await openView(opened[0]!, () => closed++);
		await act(async () => document.dispatchEvent(new dom.KeyboardEvent("keydown", { key: "Escape", bubbles: true }) as unknown as Event));
		await settle();
		expect(closed).toBe(2);
	});

	test("cut code opens as the fence it was, its language, file and length the band's fact, and Copy takes the code alone", async () => {
		const restore = tooWide();
		await say("```python setup.py\nprint(1)\nprint(2)\n```");
		await act(async () => cut("code")!.click());
		restore();
		const { sheet: _, from: __, ...rest } = opened[0]!;
		expect(rest).toEqual({ kind: "code", text: '```python title="setup.py"\nprint(1)\nprint(2)\n```', fact: "Python · setup.py · 2 lines", caption: undefined });
		const view = await openView(opened[0]!);
		await act(async () => band(view).querySelector<HTMLButtonElement>(".sheet-key")!.click());
		expect(clipboard).toEqual(["print(1)\nprint(2)"]);
	});

	test("a picture opens under the same band: ← back, who and when, its fact and keys with their words, and no Close", async () => {
		const { Viewer } = await import("../src/ui/Viewer");
		let closed = 0;
		await act(async () =>
			root.render(
				<Speakers.Provider value={(id) => (id === "juno" ? JUNO : undefined)}>
					<SaidBy.Provider value={SAID}>
						<Viewer src="blob:picture" alt="mochi.png" fact="mochi.png · 2.1 MB" onClose={() => closed++} actions={<button type="button" className="line-key">Save</button>} />
					</SaidBy.Provider>
				</Speakers.Provider>,
			),
		);
		const head = document.querySelector<HTMLElement>(".viewer .open-band")!;
		expect(head.querySelector(".open-band-name")?.textContent).toBe("Juno");
		expect([...head.querySelectorAll(".instrument")].map((one) => one.textContent)).toEqual(["· 4 min ago", "mochi.png · 2.1 MB"]);
		expect(head.querySelector('[aria-label="Close"]')).toBeNull();
		expect(head.querySelector(".line-key")?.textContent).toBe("Save");
		await act(async () => head.querySelector<HTMLButtonElement>('[aria-label="Back to the conversation"]')!.click());
		expect(closed).toBe(1);
	});

	test("a diagram's fact is its kind as a word", () => {
		expect(factOf("flowchart TD\n  A --> B")).toBe("Flowchart");
		expect(factOf("graph LR\n  A --> B")).toBe("Flowchart");
		expect(factOf("sequenceDiagram\n  A->>B: hi")).toBe("Sequence");
	});

	test("says when as the band does: just now, minutes, hours, yesterday, then the date", () => {
		const now = Date.UTC(2026, 10, 5, 12);
		expect(agoOf(now - 20_000, now)).toBe("just now");
		expect(agoOf(now - 4 * 60_000, now)).toBe("4 min ago");
		expect(agoOf(now - 3 * 3_600_000, now)).toBe("3 h ago");
		expect(agoOf(now - 30 * 3_600_000, now)).toBe("yesterday");
		expect(agoOf(now - 5 * 86_400_000, now)).not.toMatch(/ago|yesterday/);
	});

	test("Escape in the pane steps from a thread to the list, then out", () => {
		expect(dockBack({ open: { thread: { kind: "side", key: "t1" }, title: "Load test" } })).toEqual({ open: null });
		expect(dockBack({ open: null })).toBeNull();
	});
});
