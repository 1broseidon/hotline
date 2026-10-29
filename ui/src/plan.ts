import type { PlanUsage, PlanWindow } from "./generated/contract";

/*
 * A login's plan limits as the desk last read them (out of band, or Claude
 * Code after a reply): a line per window, said the way the plan's own page
 * says it, how fast it is going, and whether any is near enough its limit
 * to mark the picker.
 */

const NAMES: Record<string, string> = {
	five_hour: "5h",
	seven_day: "Weekly",
	monthly: "Monthly",
	seven_day_opus: "Opus",
	seven_day_sonnet: "Sonnet",
	overage: "Extra",
	period: "Credits",
};

/** A login's main windows, a line each whenever read. */
const MAIN = new Set(["five_hour", "seven_day", "monthly"]);

/**
 * The windows worth a line: the 5-hour, the weekly and the monthly when
 * read, the weekly always when there is no monthly (a dash until it is first
 * read, as Claude Code reports one window at a time), and a model's own only
 * once it nears its limit.
 */
export function shownWindows(plan: PlanUsage): PlanWindow[] {
	const shown = plan.windows.filter((window) => MAIN.has(window.kind) || window.status !== "allowed" || (window.usedPercent ?? 0) >= 80);
	if (!shown.some((window) => window.kind === "seven_day" || window.kind === "monthly")) {
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

/** A window as a line labels it: "5h", "Weekly", "Opus", a model's own by its name ("Fable"). */
export function windowName(window: PlanWindow): string {
	return NAMES[window.kind] ?? window.label?.split(" · ").pop() ?? window.kind.replace(/_/g, " ");
}

/** A window as a sentence names it: "Weekly · Fable" for a model's own. */
function windowTitle(window: PlanWindow): string {
	return window.label ?? windowName(window);
}

/** How far into the window now is, 0 to 1, when the window says its length and reset. */
export function elapsedShare(window: PlanWindow, now: number = Date.now()): number | undefined {
	if (window.windowMinutes === undefined || window.resetsAt === undefined || window.windowMinutes <= 0) return undefined;
	const share = 1 - (window.resetsAt - now) / (window.windowMinutes * 60_000);
	return Math.min(1, Math.max(0, share));
}

/**
 * Where the bar's tick sits, as a percentage: how much would be used by now
 * if the window were spread evenly. Fill past it is going faster than the
 * window allows.
 */
export function paceTick(window: PlanWindow, now: number = Date.now()): number | undefined {
	const share = elapsedShare(window, now);
	return share === undefined ? undefined : share * 100;
}

/**
 * When the window runs out at the rate it has been used so far, or
 * undefined when that rate lasts past its reset. A window already used up
 * has run out now.
 */
export function runsOutAt(window: PlanWindow, now: number = Date.now()): number | undefined {
	const share = elapsedShare(window, now);
	const used = window.status === "limited" ? 100 : window.usedPercent;
	if (share === undefined || used === undefined || window.resetsAt === undefined || window.windowMinutes === undefined) return undefined;
	if (used >= 100) return now;
	if (used <= 0 || share <= 0) return undefined;
	const elapsed = share * window.windowMinutes * 60_000;
	const out = now + ((100 - used) / used) * elapsed;
	return out < window.resetsAt ? out : undefined;
}

/** "Runway: runs out ~Fri 3pm" or "Runway: lasts past the reset", when the window says enough to tell. */
export function runwayText(window: PlanWindow, now: number = Date.now()): string | undefined {
	if (elapsedShare(window, now) === undefined || (window.usedPercent === undefined && window.status !== "limited")) return undefined;
	const out = runsOutAt(window, now);
	if (out === undefined) return "Runway: lasts past the reset";
	if (out <= now) return "Runway: used up until the reset";
	return `Runway: runs out ~${aboutWhen(out, now)}`;
}

/** A time as near as a guess deserves: "3:15pm" today, "Fri 3pm" after. */
function aboutWhen(at: number, now: number): string {
	const sameDay = new Date(now).toDateString() === new Date(at).toDateString();
	const step = (sameDay ? 15 : 60) * 60_000;
	const rounded = new Date(Math.round(at / step) * step);
	const time = rounded.toLocaleTimeString(undefined, { hour: "numeric", minute: rounded.getMinutes() === 0 ? undefined : "2-digit" }).replace(/\s/g, "").toLowerCase();
	return sameDay ? time : `${rounded.toLocaleDateString(undefined, { weekday: "short" })} ${time}`;
}

/** The line's hover: "5h: 38%, resets 9pm. Runway: lasts past the reset". */
export function windowHover(window: PlanWindow, now: number = Date.now()): string | undefined {
	if (window.resetsAt === undefined) return undefined;
	const runway = runwayText(window, now);
	return `${windowTitle(window)}: ${windowUse(window)}, ${resetText(window.resetsAt, now)}${runway === undefined ? "" : `. ${runway}`}`;
}

/** The head's account line: "Max subscription", or "subscription" when the plan did not say its name. */
export function planAccount(plan: PlanUsage | undefined): string {
	if (plan === undefined || plan.windows.length === 0) return "";
	return plan.plan === undefined ? "subscription" : `${plan.plan} subscription`;
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
