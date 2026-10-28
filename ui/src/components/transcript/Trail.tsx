import { useEffect, useMemo, useRef, useState } from "react";
import type { ToolOutput } from "../../generated/contract";
import type { Step } from "../../scheduledRuns";
import { ChevronDownIcon, ChevronRightIcon, EyeIcon } from "../../icons";
import { type Glance, glance } from "../../glance";
import { openLink } from "../../native";
import { joinThoughts, stepTitle } from "../../stepText";
import { type FileChange, type Source, type ToolStep, durationText, factOf, filesOf, pageOf, sourcesOf, stepTook, toolSteps } from "../../turnFacts";
import { Favicon, Favicons } from "../Favicon";
import { StepOutput } from "./Steps";

/*
 * A turn's work as the right-hand pane reads it, simple where it can be and
 * rich only where it matters. Its shape first: one thin bar, a segment a
 * step as long as it took, so where the time went shows without reading.
 * Then the web pages it read as cards (the one place an icon earns its
 * room), an eye on those it opened, the files it changed with the lines in
 * and out, and its steps as plain lines in the core's few words, a run of
 * one kind as one line, with the one fact worth a glance at the right. A
 * line opens to each step as a person reads it: a search as its query and
 * what it found, a page read as its title, a command as what it was for,
 * the command, and the first lines of its output; a failure stands out in
 * red.
 */

/** Output as a person reads it: no markdown fence around it, and JSON laid out. */
function readable(output: ToolOutput[]): ToolOutput[] {
	return output.map((one) => {
		if (one.type !== "text") return one;
		let text = one.text.trim().replace(/^```[\w-]*\n/, "").replace(/\n?```$/, "");
		if (/^[[{]/.test(text)) {
			try {
				text = JSON.stringify(JSON.parse(text), null, 2);
			} catch {
				// Not JSON after all: as it came.
			}
		}
		return { ...one, text };
	});
}

/** The pane's open line and the bar's segment light together: the line id under the pointer. */
type Hot = { id: string | null; set(id: string | null): void };

export function Trail({
	items,
	settled = false,
	openId,
}: {
	items: Step[];
	/** The turn is over: nothing in it is still running. */
	settled?: boolean;
	/** The line to open on arrival, brought into view: the one pressed in the peek. */
	openId?: string | undefined;
}) {
	const sources = useMemo(() => sourcesOf(items), [items]);
	const files = useMemo(() => filesOf(items), [items]);
	const lines = useMemo(() => glance(items, !settled), [items, settled]);
	const steps = lines.reduce((n, line) => n + line.steps.length, 0);
	const [hotId, setHot] = useState<string | null>(null);
	const hot: Hot = { id: hotId, set: setHot };
	return (
		<div className="work-sections">
			<Shape items={items} lines={lines} live={!settled} hot={hot} />
			{sources.length > 0 && <Sources sources={sources} />}
			{files.length > 0 && (
				<section aria-label="Files changed">
					<h3 className="work-heading">Files changed · {files.length}</h3>
					<ul className="file-list">
						{files.map((file) => (
							<FileRow key={file.path} file={file} />
						))}
					</ul>
				</section>
			)}
			{lines.length > 0 && (
				<section aria-label="Steps">
					<h3 className="work-heading">Steps{steps > 0 ? ` · ${steps}` : ""}</h3>
					<ol className="trail">
						{lines.map((line) => (
							<TrailLine key={line.id} line={line} sources={sources} startOpen={line.ids.includes(openId ?? "")} hot={hot} />
						))}
					</ol>
				</section>
			)}
		</div>
	);
}

/** A pause between steps longer than this is the agent thinking, and shows as a gap of its own. */
const THINKING_MS = 400;

/** Now, every quarter second while something runs, so a running time counts up. */
function useNow(running: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!running) return;
		const timer = setInterval(() => setNow(Date.now()), 250);
		return () => clearInterval(timer);
	}, [running]);
	return now;
}

/**
 * The turn's shape: one bar, a segment a step, each as long as it took, and
 * the pauses between them for thinking. A search is tinted, a failure red,
 * the step running now beats. A segment and its line light up together.
 */
function Shape({ items, lines, live, hot }: { items: Step[]; lines: Glance[]; live: boolean; hot: Hot }) {
	const now = useNow(live);
	const steps = toolSteps(items);
	const first = items[0];
	if (steps.length === 0 || first === undefined) return null;
	const lineOf = new Map(lines.flatMap((line) => line.steps.map((step) => [step.id, line.id] as const)));
	const segments: { key: string; grow: number; tone: string; line?: string }[] = [];
	let end = first.ts;
	for (const step of steps) {
		const gap = step.ts - end;
		if (gap > THINKING_MS) segments.push({ key: `gap-${step.id}`, grow: gap, tone: "think" });
		const stop = step.endedTs ?? (step.status === "in_progress" && live ? now : step.ts);
		const tone = step.status === "failed" ? "failed" : step.status === "in_progress" && live ? "live" : step.sources?.some((page) => page.found === true) ? "web" : "done";
		const line = lineOf.get(step.id);
		segments.push({ key: step.id, grow: Math.max(stop - step.ts, 1), tone, ...(line !== undefined ? { line } : {}) });
		end = Math.max(end, stop);
	}
	// To the last thing the turn did, as the pane's header counts it.
	const total = Math.max(end, ...items.map((item) => item.ts), live ? now : 0) - first.ts;
	return (
		<div className="shape" aria-hidden="true">
			<div className="shape-bar">
				{segments.map((segment) => (
					// biome-ignore lint/a11y/noStaticElementInteractions: a pointer's hover only; the bar is hidden from assistive tech, and its line says the same.
					<span
						key={segment.key}
						data-tone={segment.tone}
						data-hot={(segment.line !== undefined && segment.line === hot.id) || undefined}
						style={{ flexGrow: segment.grow }}
						onMouseEnter={segment.line !== undefined ? () => hot.set(segment.line!) : undefined}
						onMouseLeave={segment.line !== undefined ? () => hot.set(null) : undefined}
					/>
				))}
			</div>
			<div className="shape-ticks">
				<span>0s</span>
				<span>{durationText(total)}</span>
			</div>
		</div>
	);
}

/** How many sources show before the rest are asked for: a turn most often reads a few. */
const SOURCES_SHOWN = 4;

/** The pages the turn read, then those it only found: the first few as cards, the rest a press away. */
function Sources({ sources }: { sources: Source[] }) {
	const [all, setAll] = useState(false);
	const more = sources.length - SOURCES_SHOWN;
	return (
		<section aria-label="Sources">
			<h3 className="work-heading">Sources · {sources.length}</h3>
			<div className="source-grid">
				{(all ? sources : sources.slice(0, SOURCES_SHOWN)).map((source) => (
					<SourceCard key={source.url} source={source} />
				))}
			</div>
			{more > 0 && (
				<button type="button" className="source-more" aria-expanded={all} onClick={() => setAll((was) => !was)}>
					{all ? "Show fewer" : `Show ${more} more`}
				</button>
			)}
		</section>
	);
}

/**
 * A page's title as words: a search result's title can arrive as the page's
 * markdown, a logo's image and link and all, which reads as noise on a card.
 */
export function plainTitle(title: string | undefined): string | undefined {
	if (title === undefined) return undefined;
	const words = title
		// A link's address may be cut off with the title: to its end, then.
		.replace(/!\[[^\]]*\](\([^)]*\)?)?/g, "")
		.replace(/\[([^\]]*)\]\([^)]*\)?/g, "$1")
		.replace(/[[\]]/g, "")
		.replace(/\s+/g, " ")
		.trim();
	return words === "" ? undefined : words;
}

/** "/blog/next-15-4" for a page with no title of its own: where on the site it is. */
function pathOf(url: string): string {
	try {
		const at = new URL(url);
		const path = decodeURIComponent(at.pathname).replace(/\/$/, "");
		return path === "" ? at.hostname.replace(/^www\./, "") : path;
	} catch {
		return url;
	}
}

/** The green eye on a page the agent opened, as against one a search only found. */
function Read() {
	return (
		<span className="source-read" title="Read" aria-label="Read">
			<EyeIcon />
		</span>
	);
}

/** A page the turn read or found: its site's icon and name, and its title, a press away from the page. */
function SourceCard({ source }: { source: Source }) {
	const title = plainTitle(source.title);
	const titled = title !== undefined;
	return (
		<a
			href={source.url}
			className="source-card"
			data-found={source.found || undefined}
			onClick={(click) => {
				click.preventDefault();
				void openLink(source.url);
			}}
		>
			<span className="source-site">
				<Favicon site={source.site} size={16} />
				<span className="min-w-0 flex-1 truncate">{source.site}</span>
				{source.found !== true && <Read />}
			</span>
			<span className={titled ? "source-title" : "source-title source-path"}>{titled ? title : pathOf(source.url)}</span>
		</a>
	);
}

/** A file the turn changed, its name bright after a quiet folder, and its lines in and out. */
function FileRow({ file }: { file: FileChange }) {
	const at = file.path.lastIndexOf("/");
	const folder = at < 0 ? "" : file.path.slice(0, at + 1);
	const name = file.path.slice(at + 1);
	return (
		<li className="file-row" title={file.path}>
			<span className="file-path">
				<span className="file-folder">{folder}</span>
				<span className="file-name">{name}</span>
			</span>
			{file.added !== undefined && file.removed !== undefined ? (
				<span className="file-lines">
					<span className="file-added">+{file.added}</span> <span className="file-removed">−{file.removed}</span>
				</span>
			) : (
				file.created && <span className="file-lines">new</span>
			)}
		</li>
	);
}

/** One line of steps: what was done, the fact at the right, and opened, each step as a person reads it. */
function TrailLine({ line, sources, startOpen, hot }: { line: Glance; sources: Source[]; startOpen: boolean; hot: Hot }) {
	const now = useNow(line.live);
	const newest = line.steps[line.steps.length - 1];
	// The step running now counts its seconds up.
	const fact = line.live && newest !== undefined ? { text: durationText(now - newest.ts), tone: "live" as const } : factOf(line.steps, line.live);
	const thought = line.thoughts.length > 0 ? joinThoughts(line.thoughts) : "";
	const openable = thought.trim() !== "" || line.steps.some((step) => stepTitle(step.title) !== "" || hasOutput(step.output));
	const [open, setOpen] = useState(startOpen && openable);
	const node = useRef<HTMLLIElement>(null);
	useEffect(() => {
		if (startOpen) node.current?.scrollIntoView({ block: "nearest" });
	}, [startOpen]);
	const status = line.live ? "in_progress" : line.failed ? "failed" : "completed";
	return (
		<li ref={node} className="trail-step" data-step-id={line.id} data-status={status}>
			<button
				type="button"
				className="trail-head"
				data-hot={hot.id === line.id || undefined}
				aria-expanded={openable ? open : undefined}
				disabled={!openable}
				onClick={() => setOpen((was) => !was)}
				onMouseEnter={() => hot.set(line.id)}
				onMouseLeave={() => hot.set(null)}
			>
				<span aria-hidden="true" className={`trail-dot ${line.live ? "beat" : ""}`} />
				<span className="trail-said">{line.text}</span>
				{fact !== null && (
					<span className="trail-fact" data-tone={fact.tone}>
						{fact.text}
					</span>
				)}
				{openable && (open ? <ChevronDownIcon className="trail-chevron" /> : <ChevronRightIcon className="trail-chevron" />)}
			</button>
			{open && (
				<div className="trail-detail">
					{thought.trim() !== "" && <Thought text={thought} />}
					{line.steps.map((step) => (
						<StepDetail key={step.id} step={step} sources={sources} />
					))}
				</div>
			)}
		</li>
	);
}

/** The thinking before a line: two lines of it, all of it on a press. */
function Thought({ text }: { text: string }) {
	const [all, setAll] = useState(false);
	return (
		<button type="button" className="trail-thought" data-all={all || undefined} title={all ? undefined : "Show all"} onClick={() => setAll((was) => !was)}>
			{text}
		</button>
	);
}

/** A step opened, by what it was: a search, a page read, or a command or anything else. */
function StepDetail({ step, sources }: { step: ToolStep; sources: Source[] }) {
	const found = step.sources?.filter((page) => page.found === true) ?? [];
	if (found.length > 0) return <SearchRow step={step} found={found.map((page) => sources.find((one) => one.url === page.url) ?? { ...page, site: siteName(page.url) })} sources={sources} />;
	const page = step.sources?.find((one) => one.found !== true) ?? pageOf(step);
	if (page != null && step.status !== "failed") return <PageRow step={step} url={page.url} sources={sources} />;
	return <RunBlock step={step} />;
}

function siteName(url: string): string {
	try {
		return new URL(url).hostname.replace(/^www\./, "");
	} catch {
		return url;
	}
}

/** "goldfish care" from "mcp__hotline__web_search: goldfish care": what was searched for. */
function queryOf(title: string): string {
	const query = title.replace(/^(?:mcp__\w+?__)?web_?search\b:?\s*/i, "").trim();
	return query === "" ? stepTitle(title) : query;
}

/** A search: its query, the sites it turned up, how many and how long; pressed, its results, an eye on those read. */
function SearchRow({ step, found, sources }: { step: ToolStep; found: Source[]; sources: Source[] }) {
	const [open, setOpen] = useState(false);
	const took = stepTook(step);
	const read = new Set(sources.filter((one) => one.found !== true).map((one) => one.url));
	return (
		<div className="trail-search">
			<button type="button" className="trail-row" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span className="trail-query">{queryOf(step.title)}</span>
				<Favicons sites={[...new Set(found.map((one) => one.site))]} size={14} />
				<span className="trail-meta">{[`${found.length}`, took].filter((part) => part !== null).join(" · ")}</span>
			</button>
			{open && (
				<ul className="trail-hits">
					{found.map((hit) => (
						<li key={hit.url}>
							<a
								href={hit.url}
								className="trail-hit"
								onClick={(click) => {
									click.preventDefault();
									void openLink(hit.url);
								}}
							>
								<Favicon site={hit.site} size={14} />
								<span className="min-w-0 flex-1 truncate">{plainTitle(hit.title) ?? pathOf(hit.url)}</span>
								{read.has(hit.url) && <Read />}
							</a>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

/** A page read: its site's icon, its title, and how long it took; pressed, the page. */
function PageRow({ step, url, sources }: { step: ToolStep; url: string; sources: Source[] }) {
	const known = sources.find((one) => one.url === url);
	const site = known?.site ?? siteName(url);
	const took = stepTook(step);
	return (
		<a
			href={url}
			className="trail-row"
			onClick={(click) => {
				click.preventDefault();
				void openLink(url);
			}}
		>
			<Favicon site={site} size={14} />
			<span className="trail-page">{plainTitle(known?.title) ?? site}</span>
			{took !== null && <span className="trail-meta">{took}</span>}
		</a>
	);
}

/**
 * What a command said of itself, when its harness wrote its result as JSON
 * ({ command, description, exit_code, output }): what it was for, how it
 * ended, and what it printed. Null for any other output.
 */
export function ranOf(step: ToolStep): { why?: string; exit?: number; printed: string } | null {
	const text = (step.output ?? []).map((one) => (one.type === "text" ? one.text : "")).join("").trim();
	if (!text.startsWith("{")) return null;
	let result: { output?: unknown; stdout?: unknown; stderr?: unknown; exit_code?: unknown; exitCode?: unknown; description?: unknown };
	try {
		result = JSON.parse(text) as typeof result;
	} catch {
		// The desk keeps a long output cut short, mid-string: read its fields as far as they go.
		result = cutResult(text);
	}
	const printed = [result.output ?? result.stdout, result.stderr].filter((part): part is string => typeof part === "string" && part !== "");
	const exit = result.exit_code ?? result.exitCode;
	if (printed.length === 0 && typeof exit !== "number") return null;
	return {
		...(typeof result.description === "string" && result.description.trim() !== "" ? { why: result.description.trim() } : {}),
		...(typeof exit === "number" ? { exit } : {}),
		printed: printed.join("\n"),
	};
}

/** A JSON string's characters as far as they were kept, unescaped; undefined when the field is not there. */
function cutField(text: string, name: string): string | undefined {
	const raw = new RegExp(`"${name}":\\s*"((?:[^"\\\\]|\\\\.)*)`).exec(text)?.[1];
	if (raw === undefined) return undefined;
	try {
		return JSON.parse(`"${raw.replace(/\\$/, "")}"`) as string;
	} catch {
		return raw;
	}
}

/** The fields of a command's result whose JSON was cut short. */
function cutResult(text: string): { output?: string; description?: string; exit_code?: number } {
	const output = cutField(text, "output");
	const description = cutField(text, "description");
	const exit = /"exit_code":\s*(-?\d+)/.exec(text)?.[1];
	return {
		...(output !== undefined ? { output: `${output}…` } : {}),
		...(description !== undefined ? { description } : {}),
		...(exit !== undefined ? { exit_code: Number(exit) } : {}),
	};
}

/** The command a step ran, on one line, without the shell's name in front. */
function commandOf(title: string): string {
	return stepTitle(title).replace(/^(?:Run|(?:ba|z)?sh:|shell:)\s+/, "");
}

/**
 * A command, or any other step: what it was for when the agent said, how it
 * ended and how long it took, the command on a line, and the first lines of
 * what came back, the rest a press away. A failure leads with its error.
 */
function RunBlock({ step }: { step: ToolStep }) {
	const ran = ranOf(step);
	const failed = step.status === "failed" || (ran?.exit !== undefined && ran.exit !== 0);
	const took = stepTook(step);
	const meta = [ran?.exit !== undefined ? `exit ${ran.exit}` : null, took].filter((part) => part !== null).join(" · ");
	const command = commandOf(step.title);
	const output = step.output ?? [];
	const diffs = output.filter((one) => one.type === "diff");
	const text = ran?.printed ?? output.map((one) => (one.type === "text" ? one.text : "")).join("\n");
	return (
		<div className="trail-run" data-failed={failed || undefined}>
			{ran?.why !== undefined && (
				<div className="trail-why">
					<span className="min-w-0 flex-1">{ran.why}</span>
					{meta !== "" && <span className="trail-meta">{meta}</span>}
				</div>
			)}
			{command !== "" && (
				<div className="trail-cmd-row">
					<div className="trail-cmd" title={command}>
						{command}
					</div>
					{ran?.why === undefined && meta !== "" && <span className="trail-meta">{meta}</span>}
				</div>
			)}
			{diffs.length > 0 && <StepOutput output={diffs} />}
			{text.trim() !== "" && <Printed text={readableText(text)} failed={failed} />}
		</div>
	);
}

function readableText(text: string): string {
	const [one] = readable([{ type: "text", text }]);
	return one?.type === "text" ? one.text : text;
}

/** How many lines of output show before the rest are asked for. */
const PRINTED_SHOWN = 4;

/** What a step printed: its first lines fading out, all of it on a press; an error is labelled and shown whole. */
function Printed({ text, failed }: { text: string; failed: boolean }) {
	const [all, setAll] = useState(false);
	const lines = text.split("\n");
	const long = !failed && lines.length > PRINTED_SHOWN;
	return (
		<>
			{failed && <h4 className="trail-label">Error</h4>}
			<pre className="trail-out" data-failed={failed || undefined} data-clipped={(long && !all) || undefined}>
				{long && !all ? lines.slice(0, PRINTED_SHOWN).join("\n") : text}
			</pre>
			{long && (
				<button type="button" className="trail-all" onClick={() => setAll((was) => !was)}>
					{all ? "Show less" : `Show all ${lines.length} lines`}
				</button>
			)}
		</>
	);
}

function hasOutput(output: ToolOutput[] | undefined): boolean {
	return output?.some((one) => one.type === "diff" || one.text.trim() !== "") ?? false;
}
