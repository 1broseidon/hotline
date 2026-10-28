import type { ToolOutput, WebSource } from "./generated/contract";
import type { Step } from "./scheduledRuns";

/*
 * A turn's work in plain words and counts, read off the steps the tape kept:
 * the web pages it read or found, the files it changed with the lines in and
 * out, how many steps and how many failed, and the one fact worth a glance
 * beside a line ("4 results", "212 passed", "+55 −6", "6.2s"). Nothing is
 * guessed: a step written before the desk recorded a fact simply has none,
 * and the line shows without it. The phone keeps a copy (src/turnFacts.ts).
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

/** The pages a step read or found: the desk's record, or for an older step the page its title names. */
function pagesOf(step: ToolStep): WebSource[] {
	if (step.sources !== undefined) return step.sources;
	const page = pageOf(step);
	return page === null ? [] : [page];
}

/**
 * Every web page of a turn, once: the pages it read first, in the order it
 * read them, then what its searches found and it never opened. A page found
 * and then read is a page read, with whichever title the steps gave it.
 */
export function sourcesOf(items: Step[]): Source[] {
	const pages = new Map<string, Source>();
	for (const step of toolSteps(items)) {
		for (const page of pagesOf(step)) {
			const site = siteOf(page.url);
			if (site === null) continue;
			const known = pages.get(page.url);
			const title = page.title ?? known?.title;
			if (known !== undefined && (known.found !== true || page.found === true)) continue;
			// Read after it was found: it moves to the reading order.
			pages.delete(page.url);
			pages.set(page.url, { url: page.url, site, ...(title !== undefined ? { title } : {}), ...(page.found === true ? { found: true } : {}) });
		}
	}
	const all = [...pages.values()];
	return [...all.filter((one) => one.found !== true), ...all.filter((one) => one.found === true)];
}

/** The sites of the pages read so far, newest last, each once: the composer's favicons. */
export function sitesRead(items: Step[]): string[] {
	const sites: string[] = [];
	for (const source of sourcesOf(items)) if (source.found !== true && !sites.includes(source.site)) sites.push(source.site);
	return sites;
}

/** The distinct sites of a turn's sources, in order: the history line's favicons. */
export function sitesOf(items: Step[]): string[] {
	return [...new Set(sourcesOf(items).map((source) => source.site))];
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

/** The summary's parts, failures apart so they can stand out: "12 steps · 4 sources · 3 files", "1 failed". */
export function summaryWords(summary: TurnSummary, { sources = true }: { sources?: boolean } = {}): { said: string; failed: string | null } {
	const parts = [counted(summary.steps, "step", "steps")];
	if (sources && summary.sources > 0) parts.push(counted(summary.sources, "source", "sources"));
	if (summary.files > 0) parts.push(counted(summary.files, "file", "files"));
	return { said: parts.join(" · "), failed: summary.failed > 0 ? `${summary.failed} failed` : null };
}

/** "+55 −6", with the true minus sign. */
function linesText(added: number, removed: number): string {
	return `+${added} −${removed}`;
}

/** "0.4s", "6.2s", "42s", "1m 04s". */
function durationText(ms: number): string {
	const seconds = Math.max(0, ms) / 1000;
	if (seconds < 10) return `${seconds.toFixed(1)}s`;
	if (seconds < 60) return `${Math.round(seconds)}s`;
	const whole = Math.round(seconds);
	return `${Math.floor(whole / 60)}m ${String(whole % 60).padStart(2, "0")}s`;
}

/** How long a turn's work took, from its first step to its last step's end; null when its steps carry no ends. */
function turnTook(items: Step[]): number | null {
	const steps = toolSteps(items);
	const ends = steps.flatMap((step) => (step.endedTs !== undefined ? [step.endedTs] : []));
	if (ends.length === 0 || items[0] === undefined) return null;
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

