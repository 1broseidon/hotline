import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
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
	await act(async () => root.render(<Schedules personaId="ada" jobs={jobs} backgroundWork focus={false} />));
	const details = [...container.querySelectorAll(".group-row-detail")].map((row) => row.textContent ?? "");
	expect(details[0]).toStartWith(`Weekdays at ${nine} · Added by you · `);
	expect(details[1]).toStartWith("Every 1h · ");
	expect(details[2]).toStartWith("Once · ");
});
