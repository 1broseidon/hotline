import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { Root } from "react-dom/client";
import type { Attachment, TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

// The transcript's own lines as docs/design/inline-premium.html rows 7-18 draw them.

const lines = readFileSync(new URL("../src/inline-lines.css", import.meta.url), "utf8");

let act: typeof import("react").act;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restore: () => void;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	// A picture's bytes are still on their way: what holds its place is drawn.
	const { wire } = await import("../src/wire");
	const command = spyOn(wire, "command").mockImplementation((() => new Promise(() => {})) as unknown as typeof wire.command);
	restore = () => command.mockRestore();
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	await teardown();
});

async function draw(events: TranscriptEvent[], extra: Partial<Parameters<typeof Transcript>[0]> = {}) {
	await act(async () => root.render(<Transcript personaId="quill" name="Quill" events={events} streaming={[]} live={false} focus={null} {...extra} />));
}

const sent = (id: string, ts: number, file: Attachment): TranscriptEvent => ({ kind: "agent", id, ts, text: "", attachments: [file] });
const file = (name: string, size: number): Attachment => ({ kind: "file", name, path: `/sent/${name}`, size });
const picture = (name: string, size = 421_888): Attachment => ({ kind: "image", name, path: `/sent/${name}`, mimeType: "image/png", size, width: 1200, height: 2400 });

describe("files", () => {
	test("a turn's files, sent one message each, are one block with a count, their size and Save all, and each row's keys wait for hover or focus", async () => {
		await draw([sent("f1", 1, file("index.html", 18 * 1024)), sent("f2", 2, file("styles.css", 21 * 1024)), sent("f3", 3, file("mark.svg", 2 * 1024))]);
		const blocks = container.querySelectorAll(".file-block");
		expect(blocks).toHaveLength(1);
		const head = blocks[0]!.querySelector(".file-block-head")!;
		expect(head.querySelector(".line-label")?.textContent).toBe("3 files · 41 KB");
		expect([...head.querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Save all"]);
		const rows = [...blocks[0]!.querySelectorAll(".file-line")];
		expect(rows.map((row) => [row.querySelector(".file-mark")?.textContent, row.querySelector(".file-name")?.textContent, row.querySelector(".file-size")?.textContent])).toEqual([
			["HTML", "index.html", "18 KB"],
			["CSS", "styles.css", "21 KB"],
			["SVG", "mark.svg", "2 KB"],
		]);
		// Every row's keys are in the place they show from, hidden but still reachable by Tab.
		for (const row of rows) {
			const keys = row.querySelector(".file-keys")!;
			expect([...keys.querySelectorAll("button")].map((button) => button.getAttribute("aria-label") ?? button.textContent)).toEqual(["Open", "Save", "Show in folder"]);
		}
		expect(lines).toMatch(/\.file-keys \{[^}]*opacity: 0;/);
		expect(lines).toContain(".file-line:hover .file-keys,\n\t.file-line:focus-within .file-keys {\n\t\topacity: 1;");
	});

	test("one file is its row alone, and words between two files keep them apart", async () => {
		await draw([sent("f1", 1, file("notes.md", 900)), { kind: "agent", id: "said", ts: 2, text: "And the other one:" }, sent("f2", 3, file("report.pdf", 4000))]);
		const blocks = [...container.querySelectorAll(".file-block")];
		expect(blocks).toHaveLength(2);
		expect(blocks.map((block) => block.querySelector(".file-block-head"))).toEqual([null, null]);
		expect(blocks[0]!.querySelector(".file-mark")?.textContent).toBe("MD");
	});
});

describe("pictures", () => {
	test("one picture is capped, cropped from the top, with its name and size under it", async () => {
		await draw([sent("p1", 1, picture("boarding-pass.png"))]);
		const figure = container.querySelector("figure.picture")!;
		const parts = [...figure.children].map((child) => child.className.split(" ")[0]);
		expect(parts).toEqual(["picture-frame", "picture-caption"]);
		expect(figure.querySelector(".picture-caption")?.textContent).toBe("boarding-pass.png · 412 KB");
		expect(lines).toMatch(/\.picture-frame \{[^}]*max-height: 260px;/);
		expect(lines).toMatch(/\.picture-frame img \{[^}]*object-position: top center;/);
	});

	test("several from one turn are a strip of three whose last tile counts the rest, and lays them all out when pressed", async () => {
		await draw(["a", "b", "c", "d", "e"].map((name, at) => sent(name, at + 1, picture(`${name}.png`))));
		const strip = container.querySelector(".picture-strip")!;
		expect(container.querySelectorAll(".picture-strip")).toHaveLength(1);
		expect(strip.getAttribute("aria-label")).toBe("5 pictures");
		expect(strip.getAttribute("data-tiles")).toBe("3");
		const tiles = [...strip.querySelectorAll<HTMLButtonElement>(".picture-tile")];
		expect(tiles).toHaveLength(3);
		expect(tiles[2]!.querySelector(".picture-more")?.textContent).toBe("+2");
		await act(async () => tiles[2]!.click());
		expect(strip.getAttribute("data-tiles")).toBe("all");
		expect(strip.querySelectorAll(".picture-tile")).toHaveLength(5);
		expect(strip.querySelector(".picture-more")).toBeNull();
	});
});

test("a plan's segments are only done, lit or empty, and its head counts what is done", async () => {
	const plan: TranscriptEvent = {
		kind: "plan", id: "plan", ts: 1,
		entries: [
			{ content: "Pick an image tag", status: "completed" },
			{ content: "Write the collection setup", status: "completed" },
			{ content: "Check the client's port", status: "in_progress" },
			{ content: "Add a load test", status: "pending" },
		],
	};
	await draw([plan]);
	expect([...container.querySelectorAll(".plan-segment")].map((segment) => segment.className)).toEqual([
		"plan-segment plan-segment-done",
		"plan-segment plan-segment-done",
		"plan-segment plan-segment-now",
		"plan-segment plan-segment-todo",
	]);
	expect(container.querySelector(".plan-head")?.textContent).toBe("Plan2 of 4 done");
	// Nothing under way lights nothing: there is no part-filled segment.
	await draw([{ ...plan, entries: plan.entries.map((entry) => (entry.status === "in_progress" ? { ...entry, status: "pending" } : entry)) }]);
	expect([...container.querySelectorAll(".plan-segment")].map((segment) => segment.className.replace("plan-segment ", ""))).toEqual(["plan-segment-done", "plan-segment-done", "plan-segment-todo", "plan-segment-todo"]);
});

test("the line under a reply is what the turn left behind, failed first, then the time; with nothing left, only the time (epic-42)", async () => {
	const tool = (id: string, ts: number, endedTs: number, status: "completed" | "failed"): TranscriptEvent => ({
		kind: "tool", id, ts, endedTs, toolCallId: id, title: "Fetch https://qdrant.tech/documentation", toolKind: "fetch", status,
	});
	const turn = (outcomes: TranscriptEvent[]): TranscriptEvent[] => [
		{ kind: "user", id: "u", ts: 0, text: "Which vector database?" },
		tool("t1", 1_000, 9_000, "completed"),
		tool("t2", 9_000, 20_000, "failed"),
		...outcomes,
		tool("t4", 30_000, 39_000, "completed"),
		{ kind: "agent", id: "a", ts: 40_000, text: "My pick for a small team: **Qdrant**." },
		{ kind: "turn", id: "end", ts: 40_001, stopReason: "end_turn" },
	];
	// Nothing left behind: a failed step is the path's, not the line's, and no count is said.
	await draw(turn([]));
	let line = container.querySelector(".steps-caption")!;
	expect(line.textContent).toBe("38s");
	expect(line.querySelector(".outcome")).toBeNull();
	expect(line.getAttribute("title")).toBe("Show the path");
	expect(line.querySelector(".favicon, .favicons, img")).toBeNull();
	const outcome = (id: string, ts: number, type: string, message: string, more: object = {}): TranscriptEvent => ({ kind: "outcome", id, ts, type, message, key: id, ...more });
	await draw(
		turn([
			outcome("o1", 21_000, "sent", "Shared your address with 4 firms"),
			outcome("o2", 22_000, "made", "Aftercare form, 18 of 22 answers", { state: "partial" }),
			outcome("o3", 23_000, "sent", "Could not reach the school office", { state: "failed" }),
			outcome("o4", 24_000, "open", "Waiting on quotes: 1 of 4"),
			outcome("o5", 25_000, "yours", "Sign page 3"),
			outcome("o6", 26_000, "open", "Web server stopped", { closedAt: 27_000 }),
			outcome("o7", 27_000, "booked", "Booked Dr. Patel, Tue 3:30"),
		]),
	);
	line = container.querySelector(".steps-caption")!;
	const said = [...line.querySelectorAll(".outcome")].map((one) => [one.getAttribute("data-mark"), one.textContent]);
	expect(said).toEqual([
		["failed", "Could not reach the school office"],
		["done", "Shared your address with 4 firms"],
		["partial", "Aftercare form, 18 of 22 answers"],
		["open", "Waiting on quotes: 1 of 4"],
		["yours", "Sign page 3"],
		["plain", "Booked Dr. Patel, Tue 3:30"],
	]);
	expect(line.querySelector(".steps-caption-took")?.textContent).toBe("38s");
	// An open one closed later as what it became is an ordinary outcome where it was said.
	await draw(turn([outcome("o4", 24_000, "made", "Three quotes in", { closedAt: 50_000 })]));
	expect([...container.querySelectorAll(".steps-caption .outcome")].map((one) => [one.getAttribute("data-mark"), one.textContent])).toEqual([["done", "Three quotes in"]]);
});

test("threads, subagents and calls are one quiet line: a state dot, the title, the kind, what came of it, and Open, with no glyph", async () => {
	const opened: string[] = [];
	await draw(
		[
			{ kind: "link", id: "l1", ts: 1, thread: "s1", threadKind: "side", title: "Load test the local Qdrant", state: "closed", end: "agent", outcome: "p50 6 ms, p99 18 ms" },
			{ kind: "link", id: "l2", ts: 2, thread: "r1", threadKind: "run", title: "Benchmark sweep", state: "closed", end: "done", elapsedMs: 95_000 },
			{ kind: "link", id: "l3", ts: 3, thread: "c1", threadKind: "call", title: "Weekly recap", state: "parked" },
		],
		{ onOpenThread: (ref) => opened.push(ref.thread.key) },
	);
	const rows = [...container.querySelectorAll<HTMLButtonElement>("button.link-row")];
	expect(rows.map((row) => row.getAttribute("data-kind"))).toEqual(["thread", "subagent", "call"]);
	expect(rows.map((row) => [row.querySelector(".link-row-title")?.textContent, row.querySelector(".link-row-sub")?.textContent, row.querySelector(".link-row-dot")?.getAttribute("data-tone")])).toEqual([
		["Load test the local Qdrant", "Thread· p50 6 ms, p99 18 ms", "done"],
		["Benchmark sweep", "Subagent· Done in 1 min 35 s", "done"],
		["Weekly recap", "Call· Parked", "quiet"],
	]);
	for (const row of rows) {
		expect(row.querySelector("svg")).toBeNull();
		expect(row.querySelector(".link-row-open")?.textContent).toBe("Open ›");
		await act(async () => row.click());
	}
	expect(opened).toEqual(["s1", "r1", "c1"]);
	expect(container.querySelector(".rule-line:not(.tape-stamp)")).toBeNull();
});

test("a reply's quote names who said it and says their words without the Markdown they were written in", async () => {
	await draw([
		{ kind: "agent", id: "pick", ts: 1, text: "My pick for a small team: **Qdrant**. One static `Rust` binary, see [the docs](https://qdrant.tech)." },
		{ kind: "user", id: "ask", ts: 2, text: "Ok, how do I run it locally?", replyTo: "pick" },
	]);
	const quote = container.querySelector(".said-me .quote")!;
	expect(quote.querySelector(".quote-who")?.textContent).toBe("Quill");
	expect(quote.querySelector(".quote-said")?.textContent).toBe("My pick for a small team: Qdrant. One static Rust binary, see the docs.");
	expect(quote.textContent).not.toContain("**");
	expect(quote.textContent).not.toContain("`");
	// A cut that lands inside an emphasis still never shows its marks.
	await draw([
		{ kind: "agent", id: "long", ts: 1, text: `${"word ".repeat(118)}**bold that runs past the cut**` },
		{ kind: "user", id: "ask", ts: 2, text: "Hm?", replyTo: "long" },
	]);
	expect(container.querySelector(".quote-said")?.textContent).not.toContain("**");
});

test("reactions are chips on the line's edge with a count, yours ringed", async () => {
	await draw([
		{ kind: "agent", id: "done", ts: 1, text: "Done. The load test is in the notes.", reactions: ["🎉"] },
		{ kind: "user", id: "r1", ts: 2, text: "> Done. The load test is in the notes.\n👍", replyTo: "done" },
		{ kind: "user", id: "r2", ts: 3, text: "> Done. The load test is in the notes.\n👍", replyTo: "done" },
	]);
	const chips = [...container.querySelectorAll(".reactions .reaction")];
	expect(chips.map((chip) => [chip.textContent, chip.hasAttribute("data-mine")])).toEqual([
		["🎉", false],
		["👍2", true],
	]);
	expect(lines).toMatch(/\.reaction\[data-mine\] \{[^}]*var\(--accent\)/);
});

describe("errors", () => {
	test("a plain report with nothing to press is one line: what happened and why, its details a click away", async () => {
		await draw([{ kind: "notice", id: "n", ts: 1, level: "error", text: "Claude Code stopped: The usage limit resets at 9 PM." }]);
		expect(container.querySelector(".error-card")).toBeNull();
		const line = container.querySelector<HTMLDetailsElement>("details.error-line")!;
		expect(line.querySelector(".error-line-title")?.textContent).toBe("Claude Code stopped");
		expect(line.querySelector(".error-line-said")?.textContent).toBe("The usage limit resets at 9 PM.");
		expect(line.hasAttribute("open")).toBe(false);
		expect(line.textContent).not.toContain("The activity could not finish");
	});

	test("a report that is only JSON says its message; one with no head says the reason whole", async () => {
		const { errorDetails } = await import("../src/components/ErrorCard");
		expect(errorDetails('{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}')).toMatchObject({ title: "Turn failed", summary: "Overloaded" });
		expect(errorDetails("The model refused the request because the context is too long for it to read in one go")).toMatchObject({
			title: "Turn failed",
			summary: "The model refused the request because the context is too long for it to read in one go",
		});
	});
});

describe("asks", () => {
	const permission: Extract<TranscriptEvent, { kind: "permission" }> = {
		kind: "permission", id: "perm", ts: 1, requestId: "req", title: "`docker image prune -a`",
		options: [
			{ optionId: "once", name: "Allow once", kind: "allow_once" },
			{ optionId: "always", name: "Always allow", kind: "allow_always" },
			{ optionId: "no", name: "Deny", kind: "reject_once" },
		],
	};

	test("a permission says who wants to run what, the command in a well, and its answers at one weight", async () => {
		await draw([permission]);
		const card = container.querySelector(".ask-card")!;
		expect(card.querySelector(".ask-head")?.textContent).toContain("Quill wants to run");
		expect(card.querySelector(".ask-head .avatar")?.textContent).toBe("Q");
		expect(card.querySelector(".ask-command")?.textContent).toBe("$docker image prune -a");
		const answers = [...card.querySelectorAll<HTMLButtonElement>(".card-actions button")];
		expect(answers.map((button) => button.textContent)).toEqual(["Allow once", "Always allow", "Deny"]);
		expect(answers.map((button) => button.className)).toEqual(["control btn", "control btn", "control btn"]);
		expect(card.querySelector(".btn-primary")).toBeNull();
	});

	test("settled asks of every kind fold to one line with a state chip", async () => {
		await draw([
			{ ...permission, decision: "expired" },
			{ kind: "credential_ask", id: "cred", ts: 2, actionId: "cred", site: "https://porkbun.com", reason: "To list your domains.", credentialKind: "login", name: "PORKBUN", status: "declined", expiresAt: 10, existing: [], armed: false },
			{ kind: "human_action", id: "q", ts: 3, actionId: "q", reason: "Archive the 58 newsletters?", question: true, status: "done" },
		]);
		const settled = [...container.querySelectorAll(".ask-settled")];
		expect(settled.map((line) => [line.querySelector(".ask-chip")?.textContent, line.querySelector(".ask-settled-what")?.textContent, line.querySelector(".ask-settled-who")?.textContent])).toEqual([
			["Expired", "docker image prune -a", "Quill"],
			["Declined", "Sign in to porkbun.com", "Quill"],
			["Yes", "Archive the 58 newsletters?", "Quill"],
		]);
		expect(container.querySelectorAll("button, input, .card, .ask-card")).toHaveLength(0);
	});

	test("Needs you keeps its amber card and label, adds the asker's face and name, and Yes stays the mint action", async () => {
		await draw([{ kind: "human_action", id: "q", ts: 1, actionId: "q", reason: "Archive the 58 newsletters older than 30 days?", question: true, status: "pending" }]);
		const card = container.querySelector(".card.card-live")!;
		const head = card.querySelector(".eyebrow.needs-you-head")!;
		expect(head.firstElementChild?.className).toBe("avatar");
		expect(head.textContent).toBe("QNeeds you · Quill");
		const yes = [...card.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Yes")!;
		expect(yes.className).toBe("control btn-primary");
		expect([...card.querySelectorAll("button")].filter((button) => button.classList.contains("btn-primary"))).toHaveLength(1);
	});

	test("a sign-in shows a lock, the site and why, with one secure key", async () => {
		await draw([{ kind: "credential_ask", id: "cred", ts: 1, actionId: "cred", site: "https://porkbun.com/", reason: "To list your domains.", credentialKind: "login", name: "PORKBUN", status: "pending", expiresAt: Date.now() + 60_000, existing: [], armed: false }]);
		const card = container.querySelector(".ask-card")!;
		expect(card.querySelector(".sign-in-lock svg")).not.toBeNull();
		expect(card.querySelector(".sign-in-site")?.textContent).toBe("porkbun.com");
		expect(card.querySelector(".sign-in-why")?.textContent).toBe("To list your domains.");
		expect([...card.querySelectorAll(".btn-primary")].map((button) => button.textContent)).toEqual(["Sign in securely"]);
		expect(card.querySelector("input")).toBeNull();
	});
});

test("a chapter is a labeled pill on the rule", async () => {
	await draw([{ kind: "chapter", id: "ch", ts: 1, backendId: "hotline", title: "Vector DB notes" }]);
	const pill = container.querySelector(".chapter-rule .chapter-pill")!;
	expect(pill.querySelector(".line-label")?.textContent).toBe("Chapter");
	expect(pill.querySelector(".chapter-title")?.textContent).toBe("Vector DB notes");
});

test("a bubble has one toolbar: a table pointed at adds no keys of its own, and its one Copy takes the table until the pointer is on the words again", async () => {
	const copied: string[] = [];
	spyOn(navigator.clipboard, "writeText").mockImplementation(async (text: string) => {
		copied.push(text);
	});
	const table = "| Database | p50 |\n| --- | --- |\n| Qdrant | 5 ms |";
	const { OpenFull } = await import("../src/components/Sheet");
	await act(async () =>
		root.render(
			<OpenFull.Provider value={() => {}}>
				<Transcript personaId="quill" name="Quill" events={[{ kind: "agent", id: "t", ts: 1, text: `How they stack up:\n\n${table}` }]} streaming={[]} live={false} focus={null} />
			</OpenFull.Provider>,
		),
	);
	const bar = container.querySelector(".bubble-actions")!;
	const copy = () => [...bar.querySelectorAll<HTMLButtonElement>("button.bubble-action")].at(-1)!;
	const more = () => [...bar.querySelectorAll(".bubble-actions-more button")].map((key) => key.textContent);
	expect(more()).toEqual([]);
	expect(copy().title).toBe("Copy");
	// No key of the table's own on the sheet: the bubble's toolbar is the one belt.
	expect(container.querySelector(".sheet .sheet-key")).toBeNull();

	const sheet = container.querySelector<HTMLElement>(".sheet")!;
	await act(async () => sheet.dispatchEvent(new (window as unknown as { MouseEvent: typeof MouseEvent }).MouseEvent("mouseover", { bubbles: true, relatedTarget: document.body })));
	expect(more()).toEqual([]);
	expect(copy().title).toBe("Copy table");
	// Leaving the table for the toolbar keeps its Copy there to be reached.
	await act(async () => bar.dispatchEvent(new (window as unknown as { MouseEvent: typeof MouseEvent }).MouseEvent("mouseover", { bubbles: true, relatedTarget: sheet })));
	expect(copy().title).toBe("Copy table");
	await act(async () => copy().click());
	expect(copied).toEqual([table]);

	// Copy takes the table as it stands when pressed, not when it was pointed
	// at: a reply still streaming in grows under the pointer.
	const grown = `${table}\n| Weaviate | 6 ms |`;
	await act(async () =>
		root.render(
			<OpenFull.Provider value={() => {}}>
				<Transcript personaId="quill" name="Quill" events={[{ kind: "agent", id: "t", ts: 1, text: `How they stack up:\n\n${grown}` }]} streaming={[]} live={false} focus={null} />
			</OpenFull.Provider>,
		),
	);
	await act(async () => copy().click());
	expect(copied.at(-1)).toBe(grown);

	// From the keyboard: the sheet takes the focus, and ⌘C copies it.
	const focused = container.querySelector<HTMLElement>(".sheet")!;
	expect(focused.tabIndex).toBe(0);
	expect(focused.getAttribute("aria-label")).toBe("Table");
	await act(async () => focused.dispatchEvent(new (window as unknown as { KeyboardEvent: typeof KeyboardEvent }).KeyboardEvent("keydown", { key: "c", metaKey: true, bubbles: true })));
	expect(copied.length).toBe(3);
	expect(copied.at(-1)).toBe(grown);

	const words = container.querySelector<HTMLElement>(".speech .md > p")!;
	await act(async () => words.dispatchEvent(new (window as unknown as { MouseEvent: typeof MouseEvent }).MouseEvent("mouseover", { bubbles: true, relatedTarget: sheet })));
	expect(more()).toEqual([]);
});
