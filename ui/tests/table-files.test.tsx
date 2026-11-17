import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Attachment } from "../src/generated/contract";
import { installDom } from "./dom";

let act: typeof import("react").act;
let SentPreview: typeof import("../src/components/SentPreview").SentPreview;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ SentPreview } = await import("../src/components/SentPreview"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const WEEK = "Day,Sign-ups,Finished setup,Paired a phone\nMon,121,44,19\nTue,158,61,24\nWed,176,70,31\nThu,138,49,22\nFri,239,92,40\nSat,280,104,47\nSun,172,68,29\n";

/** The desk's answers to `file.read`, as base64 of these bytes, in one part; the offsets asked for are kept. */
async function serve(bytes: Uint8Array): Promise<number[]> {
	const { wire } = await import("../src/wire");
	const offsets: number[] = [];
	spyOn(wire, "command").mockImplementation((async (_cmd: string, params: { offset: number }) => {
		offsets.push(params.offset);
		return { name: "file", mimeType: "text/csv", size: bytes.length, offset: 0, data: Buffer.from(bytes).toString("base64") };
	}) as unknown as typeof wire.command);
	return offsets;
}

const utf8 = (text: string) => new TextEncoder().encode(text);

async function open(file: Attachment) {
	await act(async () => root.render(<SentPreview sent={{ personaId: "ada", eventId: "week", index: 0, file }} actions={null} onClose={() => {}} />));
	await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')!;
const fact = () => dialog().querySelector(".open-band-fact")?.textContent;
const sourceKey = () => dialog().querySelector<HTMLButtonElement>("button.sheet-key");
const cellsOf = (selector: string) => [...dialog().querySelectorAll(selector)].map((cell) => cell.textContent);

describe("a CSV opens as a table", () => {
	test("it is the table a Markdown table is drawn as, and the band counts its rows and columns", async () => {
		await serve(utf8(WEEK));
		await open({ kind: "file", name: "week-46.csv", path: "/files/week-46.csv", mimeType: "text/csv", size: WEEK.length });
		const table = dialog().querySelector("article.md.md-doc > .md-table > table");
		expect(table).not.toBeNull();
		expect(cellsOf("thead th")).toEqual(["Day", "Sign-ups", "Finished setup", "Paired a phone"]);
		expect(dialog().querySelectorAll("tbody tr")).toHaveLength(7);
		expect(cellsOf("tbody tr:last-child td")).toEqual(["Sun", "172", "68", "29"]);
		expect(fact()).toBe("week-46.csv · 7 rows × 4 columns");
	});

	test("Source shows its text, and pressed again the table", async () => {
		await serve(utf8(WEEK));
		await open({ kind: "file", name: "week-46.csv", path: "/files/week-46.csv", mimeType: "text/csv", size: WEEK.length });
		expect(sourceKey()?.textContent).toBe("Source");
		await act(async () => sourceKey()!.click());
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
		expect(sourceKey()?.getAttribute("aria-pressed")).toBe("true");
		expect(dialog().querySelector("table")).toBeNull();
		expect(dialog().querySelector("pre")?.textContent).toBe(WEEK);
		expect(fact()).toBe("week-46.csv · 7 rows × 4 columns");
		await act(async () => sourceKey()!.click());
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
		expect(dialog().querySelectorAll("tbody tr")).toHaveLength(7);
	});

	test("a TSV, and a ragged CSV with a BOM, quotes and CRLF, are tables as wide as their widest row", async () => {
		const tsv = "Day\tNote\nMon\tslow, then fast\n";
		await serve(utf8(tsv));
		await open({ kind: "file", name: "notes.tsv", path: "/files/notes.tsv", mimeType: "text/tab-separated-values", size: tsv.length });
		expect(cellsOf("tbody td")).toEqual(["Mon", "slow, then fast"]);
		expect(fact()).toBe("notes.tsv · 1 row × 2 columns");

		const ragged = '﻿name,city\r\n"Lovelace, Ada","London"\r\n"Babbage",London,"said ""hi"""\r\n';
		await serve(utf8(ragged));
		await open({ kind: "file", name: "people.csv", path: "/files/people.csv", mimeType: "text/csv", size: 80 });
		expect(cellsOf("thead th")).toEqual(["name", "city", ""]);
		expect(cellsOf("tbody td")).toEqual(["Lovelace, Ada", "London", "", "Babbage", "London", 'said "hi"']);
		expect(fact()).toBe("people.csv · 2 rows × 3 columns");
	});

	test("past 2 MB, past 5,000 rows, or when it is not UTF-8, it opens as text a page at a time", async () => {
		// Past 2 MB by its size, it is never read whole.
		const offsets = await serve(utf8(WEEK));
		await open({ kind: "file", name: "big.csv", path: "/files/big.csv", mimeType: "text/csv", size: 2 * 1024 * 1024 + 1 });
		expect(dialog().querySelector("table")).toBeNull();
		expect(sourceKey()).toBeNull();
		expect(dialog().textContent).toContain("Text preview");
		expect(offsets).toEqual([0]);
		expect(fact()).toBe("big.csv · 2.0 MB");

		// 5,000 rows under the head are a table, and one more is text.
		const rows = (n: number) => `n\n${Array.from({ length: n }, (_, at) => `${at}\n`).join("")}`;
		await serve(utf8(rows(5000)));
		await open({ kind: "file", name: "most.csv", path: "/files/most.csv", mimeType: "text/csv", size: 30 * 1024 });
		expect(dialog().querySelectorAll("tbody tr")).toHaveLength(5000);
		await act(async () => root.render(null));
		await serve(utf8(rows(5001)));
		await open({ kind: "file", name: "many.csv", path: "/files/many.csv", mimeType: "text/csv", size: 30 * 1024 });
		expect(dialog().querySelector("table")).toBeNull();
		expect(sourceKey()).toBeNull();
		expect(dialog().textContent).toContain("Text preview");
		expect(fact()).toBe("many.csv · 30 KB");

		// Not UTF-8.
		await act(async () => root.render(null));
		await serve(new Uint8Array([...utf8("name,city\nZo"), 0xeb, ...utf8(",Paris\n")]));
		await open({ kind: "file", name: "latin.csv", path: "/files/latin.csv", mimeType: "text/csv", size: 22 });
		expect(dialog().querySelector("table")).toBeNull();
		expect(sourceKey()).toBeNull();
		expect(dialog().querySelector("pre")?.textContent).toBe("name,city\nZo�,Paris\n");
	});
});
