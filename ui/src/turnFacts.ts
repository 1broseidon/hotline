import type { ToolOutput, WebSource } from "./generated/contract";
import type { Step } from "./scheduledRuns";

/*
 * A turn's work in plain words and counts, read off the steps the tape kept:
 * the web pages it opened, the files it changed with the lines in and out,
 * how many steps and how many failed, and the one fact worth a glance beside
 * a line ("4 results", "212 passed", "+55 −6", "6.2s"). Nothing is guessed:
 * a step written before the desk recorded a fact simply has none, and the
 * line shows without it, but for the pages a fetch or a browser opened,
 * which its title or its answer names. The phone keeps a copy
 * (src/turnFacts.ts).
 */

export type ToolStep = Extract<Step, { kind: "tool" }>;

/** A web page, with the site it is on as a person names it. */
export type Source = WebSource & { site: string };

/** A file the turn changed: its lines in and out, summed over its edits, when every edit counted them. */
export type FileChange = { path: string; created: boolean; added?: number; removed?: number };

/** The done line: "12 steps · 4 sources · 3 files · 1 failed". */
export type TurnSummary = { steps: number; sources: number; files: number; failed: number };

export function toolSteps(items: Step[]): ToolStep[] {
	return items.filter((item): item is ToolStep => item.kind === "tool");
}

/** "nextjs.org" from "https://www.nextjs.org/blog": the site without its "www.". */
function siteOf(url: string): string | null {
	try {
		const host = new URL(url).hostname.toLowerCase();
		return host === "" ? null : host.replace(/^www\./, "");
	} catch {
		return null;
	}
}

/** The page a fetch reads, from its title: what a running fetch is reading, and for a step written before the desk named its pages, what it read. */
export function pageOf(step: ToolStep): WebSource | null {
	if ((step.toolKind ?? "").toLowerCase() !== "fetch" && !/^web_fetch\b/.test(step.title)) return null;
	const url = /\bhttps?:\/\/[^\s"'`<>()]+/.exec(step.title)?.[0];
	return url !== undefined && siteOf(url) !== null ? { url } : null;
}

/** The teammate's browsers, by the names their tools are called: its computer's, and the desk's (the core's `glance/sources.rs`). */
const BROWSERS = new Set(["computer__browser", "mcp__computer__browser", "browser", "mcp__hotline__browser"]);

/**
 * The page a browser step opened, from its answer, for a step written before
 * the desk recorded it: the computer's browser says "navigated to <address>"
 * or "opened new tab at <address>", and the desk's heads its answer with
 * "Page: <title>" over "URL: <address>". A click names no address, so it
 * opened nothing that can be told.
 */
function browsedOf(step: ToolStep): WebSource | null {
	const name = (step.title.split(/[: ]/)[0] ?? "").toLowerCase();
	if (!BROWSERS.has(name) && !BROWSERS.has((step.toolKind ?? "").toLowerCase())) return null;
	const text = outputOf(step);
	const went = /^(?:navigated to|opened new tab at) (\S+)/.exec(text.trimStart())?.[1]?.replace(/;$/, "");
	if (went !== undefined) return /^https?:\/\//.test(went) && siteOf(went) !== null ? { url: went } : null;
	const page = /^Page: (.*)\nURL: (https?:\/\/\S+)/m.exec(text);
	if (page === null || siteOf(page[2]!) === null) return null;
	const title = page[1]!.trim();
	return { url: page[2]!, ...(title !== "" && title !== "(no title)" ? { title } : {}) };
}

/** The pages a step opened or listed: the desk's record, or for an older step what its title or its answer names. A step that failed opened nothing. */
function pagesOf(step: ToolStep): WebSource[] {
	if (step.sources !== undefined) return step.sources;
	if (step.status === "failed") return [];
	const page = pageOf(step) ?? browsedOf(step);
	return page === null ? [] : [page];
}

/**
 * Whether an address is this machine or the local network: 127.0.0.1 (all
 * of 127.x), localhost and *.localhost, ::1, 0.0.0.0, a *.local name. A page
 * there is something the teammate made or ran, not a source it read. The
 * phone's rule, word for word (src/turnFacts.ts there); a `file:` address
 * has no site, so it is never one either.
 */
export function localAddress(url: string): boolean {
	const at = url.trim();
	if (/^https?:\/\/(?:[^@/?#\s]*@)?\[(?:::1|0*:0*:0*:0*:0*:0*:0*:1)\]/i.test(at)) return true;
	const host = /^https?:\/\/(?:[^@/?#\s]*@)?([^:/?#\s]+)/i.exec(at)?.[1]?.toLowerCase();
	if (!host) return false;
	return host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host === "0.0.0.0" || /^127\.\d+\.\d+\.\d+$/.test(host);
}

/**
 * The web pages a turn opened, each once, in the order it first opened them:
 * what its fetches, its commands and its browsers read. Those are its
 * evidence (George, 2026-11-12). What a search only listed is not a source:
 * the search's own line keeps it, with an eye on those opened. A listing
 * still names a page the turn opened without a title of its own.
 */
export function sourcesOf(items: Step[]): Source[] {
	const listed = new Map<string, string>();
	const opened = new Map<string, Source>();
	for (const step of toolSteps(items)) {
		for (const page of pagesOf(step)) {
			if (page.found === true) {
				if (page.title !== undefined && !listed.has(page.url)) listed.set(page.url, page.title);
				continue;
			}
			const site = siteOf(page.url);
			if (site === null || localAddress(page.url)) continue;
			const known = opened.get(page.url);
			if (known === undefined) opened.set(page.url, { url: page.url, site, ...(page.title !== undefined ? { title: page.title } : {}) });
			else if (known.title === undefined && page.title !== undefined) known.title = page.title;
		}
	}
	return [...opened.values()].map((source) => {
		const title = source.title ?? listed.get(source.url);
		return title === undefined ? source : { ...source, title };
	});
}

/** The distinct sites of a turn's sources, in their order: the icons on the line under its reply. */
export function sitesOf(items: Step[]): string[] {
	return [...new Set(sourcesOf(items).map((source) => source.site))];
}

/** How many sites the line under a reply names, as it stacks as many icons. */
export const SITES_NAMED = 3;

/**
 * The sites in words, since an icon alone is not a name: the whole of it for
 * a screen reader ("Sources: cbsnews.com, apnews.com, bbc.com and 4 more")
 * and a few words for the pointer ("cbsnews.com and 6 more").
 */
export function sitesSaid(sites: string[]): { label: string; tip: string } {
	const named = sites.slice(0, SITES_NAMED);
	const rest = sites.length - named.length;
	const list = rest > 0 ? `${named.join(", ")} and ${rest} more` : named.length > 1 ? `${named.slice(0, -1).join(", ")} and ${named.at(-1)}` : named.join("");
	const tip = sites.length === 2 ? `${sites[0]} and ${sites[1]}` : sites.length > 2 ? `${sites[0]} and ${sites.length - 1} more` : (sites[0] ?? "");
	return { label: `Sources: ${list}`, tip };
}

/** Files changed by the turn's finished steps, each once, its counts summed over its edits. */
export function filesOf(items: Step[]): FileChange[] {
	const files = new Map<string, FileChange & { counted: boolean }>();
	for (const step of toolSteps(items)) {
		if (step.status === "failed") continue;
		for (const file of step.files ?? []) {
			const known = files.get(file.path);
			const counted = file.added !== undefined && file.removed !== undefined;
			if (known === undefined) {
				files.set(file.path, { path: file.path, created: file.created, counted, added: file.added ?? 0, removed: file.removed ?? 0 });
				continue;
			}
			known.created ||= file.created;
			known.counted &&= counted;
			known.added! += file.added ?? 0;
			known.removed! += file.removed ?? 0;
		}
	}
	return [...files.values()].map(({ counted, added, removed, ...file }) => (counted ? { ...file, added: added!, removed: removed! } : file));
}

export function summaryOf(items: Step[]): TurnSummary {
	const steps = toolSteps(items);
	return {
		steps: steps.length,
		sources: sourcesOf(items).length,
		files: filesOf(items).length,
		failed: steps.filter((step) => step.status === "failed").length,
	};
}

const counted = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "+55 −6", with the true minus sign. */
function linesText(added: number, removed: number): string {
	return `+${added} −${removed}`;
}

/** "31ms", "0.4s", "6.2s", "42s", "1m 04s". */
export function durationText(ms: number): string {
	if (ms < 100) return `${Math.max(0, Math.round(ms))}ms`;
	const seconds = ms / 1000;
	if (seconds < 10) return `${seconds.toFixed(1)}s`;
	if (seconds < 60) return `${Math.round(seconds)}s`;
	const whole = Math.round(seconds);
	return `${Math.floor(whole / 60)}m ${String(whole % 60).padStart(2, "0")}s`;
}

/** How long one step took; null for a step still running or written before the desk kept its end. */
export function stepTook(step: ToolStep): string | null {
	return step.endedTs === undefined ? null : durationText(step.endedTs - step.ts);
}

/** How long a turn's work took, from the first thing on its path to the last, a step's end included; null for a turn with no path. */
export function turnTook(items: Step[]): number | null {
	const ends = toolSteps(items).flatMap((step) => (step.endedTs !== undefined ? [step.endedTs] : []));
	if (items[0] === undefined) return null;
	return Math.max(...ends, ...items.map((item) => item.ts)) - items[0].ts;
}

/** The work band's line: "12 steps · 1 failed · 1m 04s". */
export function workDetail(items: Step[]): string {
	const summary = summaryOf(items);
	const took = turnTook(items);
	const parts = [counted(summary.steps, "step", "steps")];
	if (summary.failed > 0) parts.push(`${summary.failed} failed`);
	if (took !== null) parts.push(durationText(took));
	return parts.join(" · ");
}

function outputOf(step: ToolStep): string {
	return (step.output ?? []).map((one: ToolOutput) => (one.type === "text" ? one.text : "")).join("\n");
}

export type Fact = { text: string; tone: "quiet" | "live" | "danger" };

/**
 * The one fact beside a line of steps: "now" while it runs; for a failure,
 * "timeout" when that is what its error says, else "failed"; then the lines
 * an edit put in and took out, a search's results, the tests a run passed,
 * and otherwise how long it took. None when the steps recorded none of it.
 */
export function factOf(steps: ToolStep[], live: boolean): Fact | null {
	if (live) return { text: "now", tone: "live" };
	const failed = steps.filter((step) => step.status === "failed");
	if (failed.length > 0) return { text: failed.some((step) => /\btime(?:d)? ?out\b/i.test(outputOf(step))) ? "timeout" : "failed", tone: "danger" };
	const files = filesOf(steps);
	if (files.length > 0) {
		const counts = files.every((file) => file.added !== undefined);
		return counts ? { text: linesText(files.reduce((n, file) => n + file.added!, 0), files.reduce((n, file) => n + file.removed!, 0)), tone: "quiet" } : null;
	}
	if (steps.some((step) => step.sources?.some((page) => page.found === true))) {
		const results = steps.reduce((n, step) => n + (step.sources?.filter((page) => page.found === true).length ?? 0), 0);
		return { text: counted(results, "result", "results"), tone: "quiet" };
	}
	const passed = steps.map((step) => /\b(\d+) passed\b/.exec(outputOf(step))?.[1]).filter((n) => n !== undefined);
	if (passed.length > 0 && passed.length === steps.length) return { text: `${passed.reduce((n, one) => n + Number(one), 0)} passed`, tone: "quiet" };
	if (steps.length > 0 && steps.every((step) => step.endedTs !== undefined)) {
		return { text: durationText(steps.reduce((n, step) => n + (step.endedTs! - step.ts), 0)), tone: "quiet" };
	}
	return null;
}

