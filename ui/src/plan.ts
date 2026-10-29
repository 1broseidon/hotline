import type { PlanUsage, PlanWindow } from "./generated/contract";

/*
 * An account's plan limits as its agent last reported them (Claude Code
 * after each reply): a line per window, said the way the plan's own page
 * says it, and whether any is near enough its limit to mark the picker.
 */

const NAMES: Record<string, string> = {
	five_hour: "5h",
	seven_day: "Weekly",
	seven_day_opus: "Opus",
	seven_day_sonnet: "Sonnet",
	overage: "Extra",
};

/**
 * The windows worth a line: the 5-hour when reported, the weekly always (a
 * dash until the agent first reports it, as it reports one window at a time),
 * and a model's own only once it nears its limit.
 */
export function shownWindows(plan: PlanUsage): PlanWindow[] {
	const shown = plan.windows.filter((window) => window.kind === "five_hour" || window.kind === "seven_day" || window.status !== "allowed" || (window.usedPercent ?? 0) >= 80);
	if (!shown.some((window) => window.kind === "seven_day")) {
		const at = shown.findIndex((window) => window.kind !== "five_hour");
		shown.splice(at === -1 ? shown.length : at, 0, { kind: "seven_day", status: "allowed" });
	}
	return shown;
}

/** When it resets, as short as a line has room for: "9pm" today, "Fri" this week, "Nov 4" after. */
export function resetShort(resetsAt: number, now: number = Date.now()): string {
	const at = new Date(resetsAt);
	if (resetsAt <= now) return "now";
	if (new Date(now).toDateString() === at.toDateString()) {
		return at.toLocaleTimeString(undefined, { hour: "numeric", minute: at.getMinutes() === 0 ? undefined : "2-digit" }).replace(/\s/g, "").toLowerCase();
	}
	if (resetsAt - now < 6 * 86_400_000) return at.toLocaleDateString(undefined, { weekday: "short" });
	return at.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** A window as a line labels it: "5h", "Weekly", "Opus". */
export function windowName(window: PlanWindow): string {
	return NAMES[window.kind] ?? window.kind.replace(/_/g, " ");
}

/** How much of it is used, as a whole percentage, or how it stands when the agent gave no figure. */
export function windowUse(window: PlanWindow): string {
	if (window.status === "limited") return "limit reached";
	if (window.usedPercent !== undefined) return `${Math.round(window.usedPercent)}%`;
	return window.status === "warning" ? "near the limit" : "in use";
}

/** "resets 9pm", "resets Fri 9pm", "resets Nov 4": as near as the reset is. */
export function resetText(resetsAt: number, now: number = Date.now()): string {
	const at = new Date(resetsAt);
	const time = at.toLocaleTimeString(undefined, { hour: "numeric", minute: at.getMinutes() === 0 ? undefined : "2-digit" }).replace(/:00(?=\s)/, "");
	const days = (resetsAt - now) / 86_400_000;
	if (days < 0) return "reset";
	if (new Date(now).toDateString() === at.toDateString()) return `resets ${time}`;
	if (days < 6) return `resets ${at.toLocaleDateString(undefined, { weekday: "short" })} ${time}`;
	return `resets ${at.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
}

/** Near the limit: one window warned, refused, or past 80% used. */
export function nearLimit(plan: PlanUsage | undefined): boolean {
	return plan?.windows.some((window) => window.status !== "allowed" || (window.usedPercent ?? 0) >= 80) ?? false;
}
