import { describe, expect, test } from "bun:test";
import { elapsedShare, gaugeLabel, gaugeWindows, nearLimit, paceTick, planAccount, resetShort, resetText, runsOutAt, runwayText, shownWindows, windowName, windowNote, windowUse } from "../src/plan";
import type { PlanWindow } from "../src/generated/contract";

describe("a plan's limits", () => {
	test("each window reads as the plan names it, with its use", () => {
		expect(windowName({ kind: "five_hour", status: "allowed" })).toBe("5h");
		expect(windowName({ kind: "seven_day_opus", status: "allowed" })).toBe("Opus");
		expect(windowUse({ kind: "seven_day", usedPercent: 61.4, status: "allowed" })).toBe("61%");
		expect(windowUse({ kind: "seven_day", status: "warning" })).toBe("near the limit");
		expect(windowUse({ kind: "five_hour", usedPercent: 100, status: "limited" })).toBe("limit reached");
	});

	test("the band gauge's outer ring is the weekly window, else the longest, and its inner the 5-hour", () => {
		const five: PlanWindow = { kind: "five_hour", usedPercent: 9, status: "allowed", windowMinutes: 300 };
		const week: PlanWindow = { kind: "seven_day", usedPercent: 39, status: "allowed", windowMinutes: 10_080 };
		const opus: PlanWindow = { kind: "seven_day_opus", usedPercent: 85, status: "allowed", windowMinutes: 10_080 };
		const month: PlanWindow = { kind: "monthly", usedPercent: 12, status: "allowed", windowMinutes: 43_200 };
		const kinds = (windows: PlanWindow[]) => gaugeWindows({ at: 1, windows }).map((window) => window.kind);
		expect(kinds([five, opus, week])).toEqual(["seven_day", "five_hour"]);
		expect(kinds([month, five])).toEqual(["monthly", "five_hour"]);
		expect(kinds([five])).toEqual(["five_hour"]);
		expect(kinds([week])).toEqual(["seven_day"]);
		expect(kinds([{ kind: "overage", usedPercent: 3, status: "allowed" }])).toEqual([]);
		expect(gaugeWindows(undefined)).toEqual([]);
		expect(gaugeLabel(gaugeWindows({ at: 1, windows: [five, week] }))).toBe("Plan usage: 5h 9%, weekly 39%");
		expect(gaugeLabel([{ kind: "five_hour", status: "limited" }])).toBe("Plan usage: 5h limit reached");
	});

	test("near the limit is a warning, a refusal, or 80% used of any window", () => {
		expect(nearLimit(undefined)).toBe(false);
		expect(nearLimit({ at: 1, windows: [{ kind: "five_hour", usedPercent: 40, status: "allowed" }] })).toBe(false);
		expect(nearLimit({ at: 1, windows: [{ kind: "seven_day", usedPercent: 81, status: "allowed" }] })).toBe(true);
		expect(nearLimit({ at: 1, windows: [{ kind: "seven_day", status: "warning" }] })).toBe(true);
	});

	test("a reset reads as near as it is", () => {
		const now = new Date(2026, 9, 29, 10, 0).getTime();
		expect(resetText(new Date(2026, 9, 29, 21, 0).getTime(), now)).toMatch(/^resets 9/);
		expect(resetText(new Date(2026, 9, 31, 21, 0).getTime(), now)).toMatch(/^resets Sat/);
		expect(resetText(new Date(2026, 10, 12, 9, 0).getTime(), now)).toMatch(/^resets Nov 12/);
	});

	test("a line each for the 5-hour and the weekly, and a model's own only once it nears its limit", () => {
		const plan = {
			at: 1,
			windows: [
				{ kind: "five_hour", usedPercent: 38, status: "allowed" as const },
				{ kind: "seven_day", usedPercent: 61, status: "allowed" as const },
				{ kind: "seven_day_opus", usedPercent: 20, status: "allowed" as const },
				{ kind: "seven_day_sonnet", usedPercent: 91, status: "allowed" as const },
			],
		};
		expect(shownWindows(plan).map((one) => one.kind)).toEqual(["five_hour", "seven_day", "seven_day_sonnet"]);
	});

	test("a reset in a line is a time today, a day this week, a date after", () => {
		const now = new Date(2026, 9, 29, 10, 0).getTime();
		expect(resetShort(new Date(2026, 9, 29, 21, 0).getTime(), now)).toMatch(/^9\s?pm$/i);
		expect(resetShort(new Date(2026, 9, 31, 21, 0).getTime(), now)).toBe("Sat");
		expect(resetShort(new Date(2026, 10, 12, 9, 0).getTime(), now)).toBe("Nov 12");
	});

	test("the weekly line is always there, a dash until it is first reported", () => {
		const plan = { at: 1, windows: [{ kind: "five_hour", usedPercent: 12, status: "allowed" as const }] };
		expect(shownWindows(plan)).toEqual([
			{ kind: "five_hour", usedPercent: 12, status: "allowed" },
			{ kind: "seven_day", status: "allowed" },
		]);
	});

	test("a model's own window is named by its model, and a month is a line of its own", () => {
		const fable = { kind: "seven_day_fable", label: "Weekly · Fable", usedPercent: 85, status: "warning" as const };
		expect(windowName(fable)).toBe("Fable");
		expect(windowName({ kind: "monthly", status: "allowed" })).toBe("Monthly");
		// A monthly plan has no weekly line to wait for.
		const monthly = { at: 1, windows: [{ kind: "monthly", usedPercent: 10, status: "allowed" as const }] };
		expect(shownWindows(monthly).map((one) => one.kind)).toEqual(["monthly"]);
	});

	test("the head names the plan when the reading says it", () => {
		expect(planAccount(undefined)).toBe("");
		expect(planAccount({ at: 1, windows: [] })).toBe("");
		// A plan the agent names with no windows read: its name alone.
		expect(planAccount({ at: 1, windows: [], plan: "Google AI Pro" })).toBe("Google AI Pro");
		expect(planAccount({ at: 1, windows: [{ kind: "five_hour", status: "allowed" }] })).toBe("subscription");
		expect(planAccount({ at: 1, plan: "Max", windows: [{ kind: "five_hour", status: "allowed" }] })).toBe("Max subscription");
		expect(planAccount({ at: 1, plan: "ChatGPT Pro", windows: [{ kind: "seven_day", status: "allowed" }] })).toBe("ChatGPT Pro subscription");
	});
});

describe("a window's runway", () => {
	const hour = 3_600_000;
	// Thursday 15 October 2026, noon, local time, a week clear of any clock change.
	const now = new Date(2026, 9, 15, 12, 0).getTime();
	const week = (usedPercent: number, resetsIn: number): PlanWindow => ({ kind: "seven_day", usedPercent, resetsAt: now + resetsIn, windowMinutes: 10_080, status: "allowed" });

	test("the tick is how far into the window now is", () => {
		// Half the week gone.
		expect(paceTick(week(30, 84 * hour), now)).toBeCloseTo(50, 6);
		// Two of five hours gone.
		const fiveHour: PlanWindow = { kind: "five_hour", usedPercent: 10, resetsAt: now + 3 * hour, windowMinutes: 300, status: "allowed" };
		expect(paceTick(fiveHour, now)).toBeCloseTo(40, 6);
		// Never off the bar, and none without the window's length or reset.
		expect(elapsedShare(week(30, 200 * hour), now)).toBe(0);
		expect(elapsedShare(week(30, -hour), now)).toBe(1);
		expect(paceTick({ kind: "seven_day", usedPercent: 30, resetsAt: now + hour, status: "allowed" }, now)).toBeUndefined();
		expect(paceTick({ kind: "seven_day", usedPercent: 30, windowMinutes: 10_080, status: "allowed" }, now)).toBeUndefined();
	});

	test("use ahead of the tick runs out before the reset, and says about when", () => {
		// 60% used in half the week: at that rate the rest goes in another 56 hours.
		const fast = week(60, 84 * hour);
		const out = runsOutAt(fast, now);
		expect(out).toBe(now + 56 * hour);
		// Saturday 8pm.
		expect(runwayText(fast, now)).toMatch(/^Runway: runs out ~Sat 8\s?pm$/i);
	});

	test("use behind the tick lasts past the reset", () => {
		expect(runsOutAt(week(30, 84 * hour), now)).toBeUndefined();
		expect(runwayText(week(30, 84 * hour), now)).toBe("Runway: lasts past the reset");
		// Exactly on the tick runs out at the reset, which is lasting.
		expect(runwayText(week(50, 84 * hour), now)).toBe("Runway: lasts past the reset");
		// Nothing used yet.
		expect(runwayText(week(0, 84 * hour), now)).toBe("Runway: lasts past the reset");
	});

	test("a runway later today is said to the quarter hour, and a used-up window says so", () => {
		// 80% of five hours used in two: the rest goes in half an hour.
		const fiveHour: PlanWindow = { kind: "five_hour", usedPercent: 80, resetsAt: now + 3 * hour, windowMinutes: 300, status: "warning" };
		expect(runwayText(fiveHour, now)).toMatch(/^Runway: runs out ~12:30\s?pm$/i);
		expect(runwayText({ ...fiveHour, usedPercent: 100, status: "limited" }, now)).toBe("Runway: used up until the reset");
	});

	test("the note under a line on the gauge's card says the reset and the runway, and no runway without the window's length", () => {
		expect(windowNote(week(30, 84 * hour), now)).toMatch(/^Resets Mon .+ · Runway: lasts past the reset$/);
		expect(windowNote({ kind: "five_hour", usedPercent: 20, resetsAt: now + hour, status: "allowed" }, now)).not.toContain("Runway");
		expect(windowNote({ kind: "five_hour", usedPercent: 20, status: "allowed" }, now)).toBeUndefined();
	});
});
