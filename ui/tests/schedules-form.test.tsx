import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

let act: typeof import("react").act;
let Schedules: typeof import("../src/components/Schedules").Schedules;
let wire: typeof import("../src/wire").wire;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let sent: { cmd: string; params: unknown }[];

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	act = (await import("react")).act;
	({ Schedules } = await import("../src/components/Schedules"));
	({ wire } = await import("../src/wire"));
	sent = [];
	spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		return null;
	}) as typeof wire.command);
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
	await act(async () => root.render(<Schedules personaId="ada" jobs={[]} backgroundWork focus={false} />));
	await press("Add a schedule");
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const clock = (hours: number, minutes: number) =>
	new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(2000, 0, 1, hours, minutes));

const buttons = (selector: string) => [...container.querySelectorAll<HTMLButtonElement>(selector)];
const button = (text: string) => buttons("button").find((one) => one.textContent === text);
const press = (text: string) => act(async () => button(text)!.click());
const hint = () => container.querySelector("#job-days-said")?.textContent;
const pressed = () => buttons(".day-toggle").filter((day) => day.getAttribute("aria-pressed") === "true").map((day) => day.textContent);

async function type(selector: string, text: string) {
	const field = container.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
	await act(async () => {
		Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")!.set!.call(field, text);
		field.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}

test("the form offers Once, On days and Every, and On days opens on weekdays at 9:00", async () => {
	expect(buttons("[role=tab]").map((tab) => tab.textContent)).toEqual(["Once", "On days", "Every"]);
	expect(buttons("[role=tab][aria-selected=true]").map((tab) => tab.textContent)).toEqual(["Once"]);
	expect(container.querySelector(".day-toggles")).toBeNull();
	// Nothing scheduled: no empty card above the form.
	expect(container.querySelector(".grouped")).toBeNull();

	await press("On days");
	expect(buttons("[role=tab][aria-selected=true]").map((tab) => tab.textContent)).toEqual(["On days"]);
	expect(buttons(".day-toggle").map((day) => day.textContent)).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
	expect(pressed()).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri"]);
	expect(container.querySelector<HTMLInputElement>("#job-at")!.value).toBe(clock(9, 0));
	// Each day is a real toggle in a group named by its label.
	const group = container.querySelector(".day-toggles")!;
	expect(group.tagName).toBe("FIELDSET");
	expect(document.getElementById(group.getAttribute("aria-labelledby")!)!.textContent).toBe("Days");
	expect(buttons(".day-toggle").every((day) => day.type === "button" && day.tabIndex === 0)).toBe(true);

	await press("Every");
	expect(container.querySelector(".day-toggles")).toBeNull();
	expect(container.querySelector("#job-every")).not.toBeNull();
});

test("the line under the time reads the choice back as its row will, and says when no day is on", async () => {
	await press("On days");
	expect(hint()).toBe(`Weekdays at ${clock(9, 0)}, on this computer’s clock.`);

	await press("Sat");
	await press("Sun");
	expect(hint()).toBe(`Daily at ${clock(9, 0)}, on this computer’s clock.`);

	for (const day of ["Mon", "Tue", "Thu", "Sat", "Sun"]) await press(day);
	expect(pressed()).toEqual(["Wed", "Fri"]);
	await type("#job-at", "6:30pm");
	expect(hint()).toBe(`Wed and Fri at ${clock(18, 30)}, on this computer’s clock.`);

	await type("#job-at", "25:00");
	expect(hint()).toBe("Type a time like 9:30, 14:00 or 2pm.");
	await type("#job-at", "18:30");

	await type("#job-prompt", "Check the staging build");
	expect(button("Add schedule")!.disabled).toBe(false);
	await press("Wed");
	await press("Fri");
	expect(pressed()).toEqual([]);
	expect(hint()).toBe("Pick at least one day.");
	expect(button("Add schedule")!.disabled).toBe(true);

	await press("Wed");
	expect(hint()).toBe(`Wednesdays at ${clock(18, 30)}, on this computer’s clock.`);
	expect(button("Add schedule")!.disabled).toBe(false);
});

test("schedule.create is sent a time and days for On days, milliseconds for Every and a moment for Once", async () => {
	await type("#job-prompt", "Summarise the merged pull requests");
	await press("On days");
	await press("Tue");
	await press("Thu");
	await press("Sat");
	await type("#job-at", "2:30pm");
	await act(async () => container.querySelector<HTMLInputElement>(".check")!.click());
	await press("Add schedule");
	expect(sent).toEqual([
		{
			cmd: "schedule.create",
			params: {
				personaId: "ada",
				kind: "loop",
				at: "14:30",
				days: ["mon", "wed", "fri", "sat"],
				prompt: "Summarise the merged pull requests",
				quiet: true,
			},
		},
	]);

	sent = [];
	await press("Add a schedule");
	await type("#job-prompt", "Look for new issues");
	await press("Every");
	await type("#job-every", "2");
	await press("Add schedule");
	expect(sent).toEqual([{ cmd: "schedule.create", params: { personaId: "ada", kind: "loop", every: 7_200_000, prompt: "Look for new issues" } }]);

	sent = [];
	await press("Add a schedule");
	await type("#job-prompt", "Call Sam");
	const before = Date.now();
	await press("Add schedule");
	const [once] = sent as { cmd: string; params: { kind: string; when: number } }[];
	expect(once?.cmd).toBe("schedule.create");
	expect(Object.keys(once!.params).sort()).toEqual(["kind", "personaId", "prompt", "when"]);
	expect(once!.params.kind).toBe("schedule");
	// An hour from now, on the minute.
	expect(once!.params.when).toBeGreaterThan(before + 58 * 60_000);
	expect(once!.params.when).toBeLessThanOrEqual(before + 60 * 60_000);
});
