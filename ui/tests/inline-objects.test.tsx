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
const { OpenFull } = await import("../src/components/Sheet");
const { FullView } = await import("../src/components/FullView");
const { lit, mermaidConfig } = await import("../src/components/Mermaid");
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

/** A message as a teammate's bubble draws it, where the window can take an object full screen. */
async function say(text: string, full = true) {
	await act(async () =>
		root.render(
			<div className="speech said-them">
				<OpenFull.Provider value={full ? (object) => opened.push(object) : null}>
					<Markdown text={text} />
				</OpenFull.Provider>
			</div>,
		),
	);
}

const sheet = (kind: string) => container.querySelector<HTMLElement>(`[data-sheet="${kind}"]`);
const label = (kind: string) => sheet(kind)?.querySelector(".sheet-label")?.textContent;
const keys = (kind: string) => [...(sheet(kind)?.querySelectorAll<HTMLButtonElement>(".sheet-key") ?? [])];
const key = (kind: string, name: string) => keys(kind).find((one) => one.textContent === name)!;
const settle = () => act(async () => new Promise((done) => setTimeout(done, 0)));

const TABLE = ["| Database | License | p50 |", "| --- | --- | --- |", "| Qdrant | Apache 2.0 | 5–8 ms |", "| Weaviate | BSD-3 | 6 ms |", "| Milvus | Apache 2.0 | ~5 ms |"].join("\n");

describe("a table", () => {
	test("sits on one sheet that names nothing, with Expand and Copy", async () => {
		await say(`Here is how they stack up:\n\n${TABLE}`);
		expect(label("table")).toBeUndefined();
		expect(sheet("table")?.hasAttribute("data-bare")).toBe(true);
		expect(keys("table").map((one) => one.textContent)).toEqual(["Expand", "Copy"]);
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

	test("has no Expand key where nothing can open it full screen", async () => {
		await say(TABLE, false);
		expect(keys("table").map((one) => one.textContent)).toEqual(["Copy"]);
	});
});

describe("a block of code", () => {
	test("sits on one sheet whose head names its language and file, with Wrap, Expand and Copy", async () => {
		await say("```python setup.py\nfrom qdrant_client import QdrantClient\n```");
		expect(label("code")).toBe("python · setup.py");
		expect(keys("code").map((one) => one.textContent)).toEqual(["Wrap", "Expand", "Copy"]);
		expect(sheet("code")!.querySelector(".code-keyword")?.textContent).toBe("from");
	});

	test("numbers its lines only from six lines up", async () => {
		await say("```bash\none\ntwo\nthree\nfour\nfive\n```");
		expect(sheet("code")!.querySelector("pre")!.hasAttribute("data-numbered")).toBe(false);
		await say("```bash\none\ntwo\nthree\nfour\nfive\nsix\n```");
		const pre = sheet("code")!.querySelector("pre")!;
		expect(pre.hasAttribute("data-numbered")).toBe(true);
		expect([...pre.querySelectorAll(".code-line")].map((line) => line.getAttribute("data-n"))).toEqual(["1", "2", "3", "4", "5", "6"]);
	});

	test("Wrap folds long lines and folds them back; unwrapped, a line is kept whole to scroll", async () => {
		const long = `echo ${"a".repeat(300)}`;
		await say(`\`\`\`bash\n${long}\n\`\`\``);
		const pre = () => sheet("code")!.querySelector("pre")!;
		expect(pre().hasAttribute("data-wrap")).toBe(false);
		expect(pre().textContent).toBe(long);
		await act(async () => key("code", "Wrap").click());
		expect(pre().hasAttribute("data-wrap")).toBe(true);
		expect(key("code", "Wrap").getAttribute("aria-pressed")).toBe("true");
		await act(async () => key("code", "Wrap").click());
		expect(pre().hasAttribute("data-wrap")).toBe(false);
		expect(key("code", "Wrap").getAttribute("aria-pressed")).toBe("false");
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
		test(`marked [!${marker}] is an amber callout with the warning mark and its label`, async () => {
			await say(`> [!${marker}]\n> A network disk turns the 5 ms p50 into 40.`);
			const callout = container.querySelector<HTMLElement>(".callout")!;
			expect(callout.dataset["alert"]).toBe(marker.toLowerCase());
			expect(callout.querySelector("svg.callout-icon")).not.toBeNull();
			expect(callout.querySelector(".callout-label")?.textContent).toBe(name);
			expect(callout.querySelector(".callout-words p")?.textContent).toBe("A network disk turns the 5 ms p50 into 40.");
			expect(callout.textContent).not.toContain("[!");
		});
	}

	test("marked [!NOTE], [!TIP] or [!IMPORTANT] is a labelled callout with no warning mark", async () => {
		for (const [marker, name] of [["NOTE", "Note"], ["TIP", "Tip"], ["IMPORTANT", "Important"]] as const) {
			await say(`> [!${marker}]\n>\n> Worth knowing.`);
			const callout = container.querySelector<HTMLElement>(".callout")!;
			expect(callout.dataset["alert"]).toBe(marker.toLowerCase());
			expect(callout.querySelector("svg")).toBeNull();
			expect(callout.querySelector(".callout-label")?.textContent).toBe(name);
			// The marker's paragraph stood alone, so it goes with the marker.
			expect([...callout.querySelectorAll(".callout-words p")].map((p) => p.textContent)).toEqual(["Worth knowing."]);
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
		// Not drawn here (bun cannot load mermaid's browser build), so its source shows, and keys wait for the drawing.
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

describe("full screen", () => {
	test("Expand hands the window the table, which the full view draws with the same table and no Expand of its own", async () => {
		await say(`Here:\n\n${TABLE}`);
		await act(async () => key("table", "Expand").click());
		expect(opened).toEqual([{ kind: "table", text: TABLE, title: "Database · License · p50" }]);

		let closed = 0;
		await act(async () => root.render(<FullView object={opened[0]!} onClose={() => closed++} />));
		await settle();
		const view = document.querySelector(".full-view");
		expect(view?.getAttribute("aria-label")).toBe("Database · License · p50");
		expect(view?.querySelectorAll("tbody tr").length).toBe(3);
		expect([...(view?.querySelectorAll(".sheet-key") ?? [])].map((one) => one.textContent)).toEqual(["Copy", "Copy"]);
		await act(async () => view!.querySelector<HTMLButtonElement>(".full-view-close")!.click());
		expect(closed).toBe(1);
		await act(async () => root.render(<FullView object={opened[0]!} onClose={() => closed++} />));
		await act(async () => document.dispatchEvent(new dom.KeyboardEvent("keydown", { key: "Escape", bubbles: true }) as unknown as Event));
		expect(closed).toBe(2);
	});

	test("a block of code expands as the fence it was, and Copy there takes the code alone", async () => {
		await say("```python setup.py\nprint(1)\n```");
		await act(async () => key("code", "Expand").click());
		expect(opened).toEqual([{ kind: "code", text: '```python title="setup.py"\nprint(1)\n```', title: "python · setup.py" }]);
		await act(async () => root.render(<FullView object={opened[0]!} onClose={() => {}} />));
		await act(async () => document.querySelector<HTMLButtonElement>(".full-view-head .sheet-key")!.click());
		expect(clipboard).toEqual(["print(1)"]);
	});

	test("Escape in the pane steps from a thread to the list, then out", () => {
		expect(dockBack({ open: { thread: { kind: "side", key: "t1" }, title: "Load test" } })).toEqual({ open: null });
		expect(dockBack({ open: null })).toBeNull();
	});
});
