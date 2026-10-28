import { useEffect, useMemo, useRef, useState } from "react";
import type { ToolOutput } from "../../generated/contract";
import type { Step } from "../../scheduledRuns";
import { ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { type Glance, glance } from "../../glance";
import { openLink } from "../../native";
import { joinThoughts, stepTitle } from "../../stepText";
import { type FileChange, type Source, factOf, filesOf, sourcesOf } from "../../turnFacts";
import { Favicon } from "../Favicon";
import { StepOutput } from "./Steps";

/*
 * A turn's work as the right-hand pane reads it, simple where it can be and
 * rich only where it matters: the web pages it read as cards (the one place
 * an icon earns its room), the files it changed with the lines in and out,
 * then its steps as plain lines in the core's few words, a run of one kind
 * as one line, with the one fact worth a glance at the right. A line opens
 * to what was run and what came back, as labelled blocks; a failure stands
 * out in red.
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
	return (
		<div className="work-sections">
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
							<TrailLine key={line.id} line={line} startOpen={line.ids.includes(openId ?? "")} />
						))}
					</ol>
				</section>
			)}
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
				<span className="truncate">{source.site}</span>
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

/** One line of steps: what was done, the fact at the right, and opened, each step's command and what came back. */
function TrailLine({ line, startOpen }: { line: Glance; startOpen: boolean }) {
	const fact = factOf(line.steps, line.live);
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
			<button type="button" className="trail-head" aria-expanded={openable ? open : undefined} disabled={!openable} onClick={() => setOpen((was) => !was)}>
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
					{thought.trim() !== "" && (
						<section>
							<h4 className="trail-label">Thinking</h4>
							<p className="trail-thought">{thought}</p>
						</section>
					)}
					{line.steps.map((step) => {
						const ran = stepTitle(step.title);
						return (
							<div key={step.id} className="trail-run" data-step-id={step.id}>
								{ran !== "" && (
									<section>
										<h4 className="trail-label">Command</h4>
										<pre className="trail-code">{ran}</pre>
									</section>
								)}
								{hasOutput(step.output) && (
									<section>
										<h4 className="trail-label">{step.status === "failed" ? "Error" : "Output"}</h4>
										<StepOutput output={readable(step.output!)} />
									</section>
								)}
							</div>
						);
					})}
				</div>
			)}
		</li>
	);
}

function hasOutput(output: ToolOutput[] | undefined): boolean {
	return output?.some((one) => one.type === "diff" || one.text.trim() !== "") ?? false;
}

