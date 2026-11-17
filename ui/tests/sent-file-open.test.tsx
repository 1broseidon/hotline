import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Attachment, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

let act: typeof import("react").act;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let desks: typeof import("../src/desks");
let dom: import("happy-dom").Window;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
/** What the page asked of the shell, in order. */
let shell: { cmd: string; args: Record<string, unknown> }[];

beforeEach(async () => {
	({ dom, teardown } = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} }));
	Object.assign(globalThis, { ResizeObserver: dom.ResizeObserver });
	shell = [];
	// The window runs in the shell, which answers the page's commands; the
	// list of desks and the event plugin are the window's own business.
	Object.assign(dom, {
		__hotlineDesk: { platform: "macos", origin: "", token: "" },
		__TAURI_INTERNALS__: {
			invoke: async (cmd: string, args: Record<string, unknown>) => {
				if (cmd === "desk_list") return [];
				if (cmd.startsWith("plugin:")) return null;
				shell.push({ cmd, args });
				return cmd === "transfer_begin" ? "download-1" : null;
			},
		},
	});
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	desks = await import("../src/desks");
	// This computer's desk, whatever a file before this one left.
	desks.replaceDesks([]);
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	desks.replaceDesks([]);
	await teardown();
});

const WORD = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const EXCEL = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const POWERPOINT = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

const file = (name: string, mimeType: string): Attachment => ({ kind: "file", name, path: `/data/files/ada/pack/${name}`, mimeType, size: 1024 });

/** The week's pack as a teammate sends it: one of each kind of row. */
const PACK: Attachment[] = [
	file("summary.md", "text/markdown"),
	file("notes.txt", "text/plain"),
	file("app.ts", "text/plain"),
	file("invoice-1182.pdf", "application/pdf"),
	file("brief.docx", WORD),
	file("q3.xlsx", EXCEL),
	file("pitch.pptx", POWERPOINT),
	file("exports.zip", "application/zip"),
	file("dump.bin", "application/octet-stream"),
	file("brief.docm", "application/vnd.ms-word.document.macroenabled.12"),
];

async function draw(events: TranscriptEvent[]) {
	await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} />));
}

const sent = (attachments: Attachment[]): TranscriptEvent => ({ kind: "agent", id: "pack", ts: 1, text: "Here is the week 46 pack.", attachments });

/** A row by its file's name. */
function row(name: string): HTMLElement {
	const found = [...container.querySelectorAll<HTMLElement>(".file-line")].find((line) => line.querySelector(".file-name")?.textContent === name);
	if (found === undefined) throw new Error(`no row for ${name}`);
	return found;
}

const keys = (name: string) => [...row(name).querySelectorAll("button")];
const open = (name: string) => keys(name).find((key) => key.textContent === "Open");

describe("where Open on a teammate's file goes", () => {
	test("a PDF and a Word, Excel or PowerPoint file open outside Hotline, and their Open says so with an arrow", async () => {
		await draw([sent(PACK)]);
		for (const name of ["invoice-1182.pdf", "brief.docx", "q3.xlsx", "pitch.pptx"]) {
			const key = open(name);
			expect(key?.getAttribute("title")).toBe("Opens outside Hotline");
			expect(key?.querySelector("svg path")?.getAttribute("d")).toBe("M6.5 3.5h-2a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1v-2M9 3.5h3.5V7M12.5 3.5 7.5 8.5");
		}
	});

	test("text and Markdown keep a plain Open, which opens the viewer", async () => {
		const { wire } = await import("../src/wire");
		spyOn(wire, "command").mockImplementation((async () => ({ name: "notes.txt", mimeType: "text/plain", size: 5, offset: 0, data: btoa("hello") })) as unknown as typeof wire.command);
		await draw([sent(PACK)]);
		for (const name of ["summary.md", "notes.txt", "app.ts"]) {
			const key = open(name);
			expect(key?.getAttribute("title")).toBeNull();
			expect(key?.querySelector("svg")).toBeNull();
		}
		await act(async () => open("notes.txt")!.click());
		expect(document.querySelector('[role="dialog"] pre')?.textContent).toBe("hello");
		expect(shell).toEqual([]);
	});

	test("a file the desk cannot show has no Open, only Save and Show in folder", async () => {
		await draw([sent(PACK)]);
		for (const name of ["exports.zip", "dump.bin", "brief.docm"]) {
			expect(keys(name).map((key) => key.getAttribute("aria-label") ?? key.textContent)).toEqual(["Save", "Show in folder"]);
		}
	});

	test("on this computer's desk an Office file is handed to the shell to open by its kept path", async () => {
		await draw([sent(PACK)]);
		await act(async () => open("brief.docx")!.click());
		expect(shell).toEqual([{ cmd: "open_sent_file", args: { path: "/data/files/ada/pack/brief.docx" } }]);
		expect(document.querySelector('[role="dialog"]')).toBeNull();
	});

	test("on a server desk an Office file is brought down and opened the way a PDF is", async () => {
		desks.replaceDesks([{ id: "srv", name: "Build box", kind: "remote", origin: "http://127.0.0.1:1", token: "t" }]);
		desks.setActiveDesk("srv");
		const { wire } = await import("../src/wire");
		const reads: unknown[] = [];
		spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
			reads.push({ cmd, ...params });
			return params["offset"] === 0 ? { data: btoa("PK\x03\x04"), next: 4 } : { data: btoa("rest") };
		}) as unknown as typeof wire.command);
		await draw([sent(PACK)]);
		for (const name of ["invoice-1182.pdf", "pitch.pptx"]) {
			shell = [];
			reads.length = 0;
			await act(async () => open(name)!.click());
			const index = PACK.findIndex((one) => one.name === name);
			expect(reads).toEqual([
				{ cmd: "file.read", personaId: "ada", eventId: "pack", index, offset: 0 },
				{ cmd: "file.read", personaId: "ada", eventId: "pack", index, offset: 4 },
			]);
			expect(shell.map(({ cmd, args }) => [cmd, cmd === "transfer_write" ? args["data"] : args])).toEqual([
				["transfer_begin", { name, open: true }],
				["transfer_write", btoa("PK\x03\x04")],
				["transfer_write", btoa("rest")],
				["transfer_end", { id: "download-1", finished: true }],
			]);
		}
	});

	test("your own files open as they always have, whatever their kind", async () => {
		await draw([{ kind: "user", id: "u1", ts: 1, text: "For you.", attachments: [file("exports.zip", "application/zip"), file("brief.docx", WORD)] }]);
		for (const name of ["exports.zip", "brief.docx"]) {
			const key = open(name);
			expect(key).toBeDefined();
			expect(key?.getAttribute("title")).toBeNull();
			expect(keys(name)).toHaveLength(1);
		}
		await act(async () => open("exports.zip")!.click());
		expect(shell).toEqual([{ cmd: "open_sent_file", args: { path: "/data/files/ada/pack/exports.zip" } }]);
	});
});

describe("files an earlier desk kept, which named a file by its ending alone", () => {
	/** Such a file as the tape holds it: the type the desk before 0.50 gave its name. */
	const OLD: Attachment[] = [
		file("Panel.tsx", "application/octet-stream"),
		file("app.ts", "video/mp2t"),
		file("index.ts", "video/vnd.dlna.mpeg-tts"),
		file("main.go", "application/octet-stream"),
		file("Dockerfile", "application/octet-stream"),
		file("exports.zip", "application/zip"),
		file("dump.bin", "application/octet-stream"),
		file("brief.docm", "application/vnd.ms-word.document.macroenabled.12"),
	];

	test("an old .tsx and an old .ts keep their Open, and it opens the viewer", async () => {
		const { wire } = await import("../src/wire");
		spyOn(wire, "command").mockImplementation((async () => ({ name: "Panel.tsx", mimeType: "application/octet-stream", size: 22, offset: 0, data: btoa("export const Panel = 1") })) as unknown as typeof wire.command);
		await draw([sent(OLD)]);
		for (const name of ["Panel.tsx", "app.ts", "index.ts", "main.go", "Dockerfile"]) {
			const key = open(name);
			expect(key).toBeDefined();
			expect(key?.getAttribute("title")).toBeNull();
		}
		for (const name of ["Panel.tsx", "app.ts"]) {
			await act(async () => open(name)!.click());
			expect(document.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe(name);
			expect(document.querySelector('[role="dialog"] pre')?.textContent).toBe("export const Panel = 1");
			await act(async () => document.querySelector<HTMLButtonElement>('[role="dialog"] button[aria-label="Back to the conversation"]')!.click());
		}
		expect(shell).toEqual([]);
	});

	test("an old .zip, .bin and .docm still have no Open", async () => {
		await draw([sent(OLD)]);
		for (const name of ["exports.zip", "dump.bin", "brief.docm"]) {
			expect(open(name)).toBeUndefined();
			expect(keys(name).map((key) => key.getAttribute("aria-label") ?? key.textContent)).toEqual(["Save", "Show in folder"]);
		}
	});
});
