import { describe, expect, test } from "bun:test";
import { nearLimit, resetText, windowName, windowUse } from "../src/plan";

describe("a plan's limits", () => {
	test("each window reads as the plan names it, with its use", () => {
		expect(windowName({ kind: "five_hour", status: "allowed" })).toBe("5-hour");
		expect(windowName({ kind: "seven_day_opus", status: "allowed" })).toBe("Weekly · Opus");
		expect(windowUse({ kind: "seven_day", usedPercent: 61.4, status: "allowed" })).toBe("61%");
		expect(windowUse({ kind: "seven_day", status: "warning" })).toBe("near the limit");
		expect(windowUse({ kind: "five_hour", usedPercent: 100, status: "limited" })).toBe("limit reached");
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
});
