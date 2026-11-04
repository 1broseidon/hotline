import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// The objects a message carries (docs/design/inline-premium.html rows 1-5),
// drawn by the bubble's own markdown, and opened in the pane.

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Markdown } = await import("../src/components/Markdown");
const { OpenInPane } = await import("../src/components/Sheet");
const { Dock } = await import("../src/components/Dock");
const { lit, mermaidConfig } = await import("../src/components/Mermaid");
const { dockBack, withObject } = await import("../src/dock");
type PaneObject = import("../src/components/Sheet").PaneObject;
type DockState = import("../src/components/Dock").DockState;

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let teardown: () => Promise<void>;
let clipboard: string[];
let opened: PaneObject[];

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

/** A message as a teammate's bubble draws it, where the pane can take an object. */
async function say(text: string, pane = true) {
	await act(async () =>
		root.render(
			<div className="speech said-them">
				<OpenInPane.Provider value={pane ? (object) => opened.push(object) : null}>
					<Markdown text={text} />
				</OpenInPane.Provider>
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
	test("sits on one sheet whose head counts its rows and columns, with Open and Copy", async () => {
		await say(`Here is how they stack up:\n\n${TABLE}`);
		expect(label("table")).toBe("Table · 3 rows · 3 columns");
		expect(keys("table").map((one) => one.textContent)).toEqual(["Open", "Copy"]);
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

	test("has no Open key where there is no pane to open it in", async () => {
		await say(TABLE, false);
		expect(keys("table").map((one) => one.textContent)).toEqual(["Copy"]);
	});
});

describe("a block of code", () => {
	test("sits on one sheet whose head names its language and file, with Wrap and Copy", async () => {
		await say("```python setup.py\nfrom qdrant_client import QdrantClient\n```");
		expect(label("code")).toBe("python · setup.py");
		expect(keys("code").map((one) => one.textContent)).toEqual(["Wrap", "Copy"]);
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
	test("sits on one sheet whose head names its kind", async () => {
		await say("How a query flows:\n\n```mermaid\nflowchart LR\n  A[App] --> B[Embed query]\n```");
		expect(label("diagram")).toBe("Diagram · flowchart");
		// Not drawn here (bun cannot load mermaid's browser build), so its source shows, and keys wait for the drawing.
		expect(sheet("diagram")!.querySelector(".diagram-source")?.textContent).toBe("flowchart LR\n  A[App] --> B[Embed query]");
		expect(keys("diagram")).toEqual([]);
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

describe("open in the pane", () => {
	const props = { onOpenTeammate() {}, entry: null, roster: [], width: 360, onWidth() {}, overlay: false };

	test("Open hands the pane the table, which draws it with the same sheet and no Open of its own; close goes back", async () => {
		await say(`Here:\n\n${TABLE}`);
		await act(async () => key("table", "Open").click());
		expect(opened).toEqual([{ kind: "table", text: TABLE }]);

		const thread = { open: null } satisfies DockState;
		const state = withObject(thread, opened[0]!);
		const states: (DockState | null)[] = [];
		let closed = 0;
		await act(async () => root.render(<Dock {...props} state={state} onState={(next) => states.push(next)} onClose={() => closed++} />));
		await settle();
		expect(container.querySelector(".dock")?.getAttribute("aria-label")).toBe("Table");
		expect(label("table")).toBe("Table · 3 rows · 3 columns");
		expect(keys("table").map((one) => one.textContent)).toEqual(["Copy"]);

		await act(async () => container.querySelector<HTMLButtonElement>('.dock button[aria-label="Close"]')!.click());
		expect(states).toEqual([thread]);
		expect(closed).toBe(0);
	});

	test("Escape steps back from an object to what the pane showed, or out when it showed nothing", () => {
		const object: PaneObject = { kind: "diagram", text: "```mermaid\ngraph TD; A-->B\n```" };
		const thread: DockState = { open: { thread: { kind: "side", key: "t1" }, title: "Load test" } };
		expect(dockBack(withObject(thread, object))).toEqual(thread);
		expect(dockBack(withObject(null, object))).toBeNull();
		// Opened over another object, the way back is still what was there first.
		expect(dockBack(withObject(withObject(thread, object), { kind: "table", text: TABLE }))).toEqual(thread);
		expect(dockBack(thread)).toEqual({ open: null });
		expect(dockBack({ open: null })).toBeNull();
	});
});
