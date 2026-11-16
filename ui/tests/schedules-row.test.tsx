import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Person } from "../src/avatars";
import type { ScheduledJob } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

let act: typeof import("react").act;
let Schedules: typeof import("../src/components/Schedules").Schedules;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	act = (await import("react")).act;
	({ Schedules } = await import("../src/components/Schedules"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const people = new Map<string, Person>([
	["huck", { name: "Huck" }],
	["quill", { name: "Quill" }],
]);

const job = (id: string, extra: Partial<ScheduledJob>): ScheduledJob => ({
	id,
	personaId: "ada",
	kind: "loop",
	prompt: `job ${id}`,
	operatorCreated: true,
	nextAt: Date.now() + 3_600_000,
	createdAt: 1,
	...extra,
});

test("a calendar loop's row says its days and time, and an interval loop's its interval", async () => {
	const nine = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(2000, 0, 1, 9, 0));
	const jobs = [
		job("weekdays", { every: 86_400_000, calendar: { at: "09:00", days: ["mon", "tue", "wed", "thu", "fri"] } }),
		job("hourly", { every: 3_600_000 }),
		job("once", { kind: "schedule", when: Date.now() + 3_600_000 }),
	];
	await act(async () => root.render(<Schedules personaId="ada" jobs={jobs} backgroundWork focus={false} people={people} />));
	const details = [...container.querySelectorAll(".group-row-detail")].map((row) => row.textContent ?? "");
	expect(details[0]).toStartWith(`Weekdays at ${nine} · Added by you · `);
	expect(details[1]).toStartWith("Every 1h · ");
	expect(details[2]).toStartWith("Once · ");
});

test("a loop whose last run was skipped says when it was due and why, and no line says so once the skip is gone", async () => {
	const slot = new Date(2026, 10, 13, 8, 30).getTime();
	const due = `${new Intl.DateTimeFormat(undefined, { weekday: "short" }).format(slot)} ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(slot)}`;
	const calendar = { at: "08:30", days: ["mon", "wed", "fri"] as ("mon" | "wed" | "fri")[] };
	const skipped = job("prs", { every: 86_400_000, calendar, skipped: { slot, at: slot + 3 * 3_600_000 } });
	await act(async () => root.render(<Schedules personaId="ada" jobs={[skipped, job("issues", { every: 7_200_000 })]} backgroundWork focus={false} people={people} />));
	const lines = (row: Element) => [...row.querySelectorAll(".group-row-detail")].map((line) => line.textContent);
	const [first, second] = [...container.querySelectorAll(".group-row:not(.group-row-add)")];
	expect(lines(first!)).toHaveLength(2);
	expect(lines(first!)[1]).toBe(`Skipped ${due}, over 2 hours late`);
	expect(lines(second!)).toHaveLength(1);

	const { skipped: _cleared, ...ran } = skipped;
	await act(async () => root.render(<Schedules personaId="ada" jobs={[ran]} backgroundWork focus={false} people={people} />));
	expect(container.textContent).not.toContain("Skipped");
});

test("a row's detail is whole parts that wrap, the dot between them drawn and the text keeping its own for a reader", async () => {
	const nine = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(2000, 0, 1, 9, 0));
	const jobs = [job("weekdays", { every: 86_400_000, calendar: { at: "09:00", days: ["mon", "tue", "wed", "thu", "fri"] }, nextAt: Date.now() + 11 * 3_600_000 })];
	await act(async () => root.render(<Schedules personaId="ada" jobs={jobs} backgroundWork focus={false} people={people} />));
	const detail = container.querySelector(".group-row-detail.schedule-detail")!;
	// One wrapping list of parts and nothing else: no text of its own to wrap at.
	expect([...detail.childNodes].map((node) => (node as Element).className)).toEqual(["schedule-detail-parts"]);
	const parts = [...detail.firstElementChild!.children];
	// Each part's separator is visually hidden text; the one seen is drawn by
	// index.css in the part's clipped padding, so no line starts with it.
	expect(parts.map((part) => [...part.querySelectorAll(".sr-only")].map((hidden) => hidden.textContent))).toEqual([[], [" · "], [" · "]]);
	expect(parts.map((part) => [...part.childNodes].filter((node) => node.nodeType === 3).map((node) => node.textContent).join(""))).toEqual([
		`Weekdays at ${nine}`,
		"Added by you",
		"in 11h",
	]);
	expect(detail.textContent).toBe(`Weekdays at ${nine} · Added by you · in 11h`);
	// The skipped line and other rows' details are plain lines, as before.
	expect(container.querySelectorAll(".schedule-detail")).toHaveLength(1);
});

test("a one-time job whose run waits on a colleague's reply says whom on in place of when it runs next", async () => {
	const waiting = job("once", { kind: "schedule", when: Date.now() - 60_000, nextAt: Date.now() - 60_000, awaiting: ["huck", "quill"] });
	await act(async () => root.render(<Schedules personaId="ada" jobs={[waiting]} backgroundWork focus={false} people={people} />));
	expect(container.querySelector(".schedule-detail")?.textContent).toBe("Once · Added by you · Waiting on Huck, Quill");
});
