import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { ToolOutput, ToolStatus } from "../../generated/contract";
import type { Step } from "../../scheduledRuns";
import { ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { joinThoughts, outputText, stepTitle } from "../../stepText";
import { failedRuns, markOf, outcomesOf } from "../../outcomes";
import type { RunLink } from "../../scheduledRuns";
import { durationText, SITES_NAMED, sitesOf, sitesSaid, sourcesOf, turnTook } from "../../turnFacts";
import { Favicons } from "../Favicon";
import { Sources } from "./Sources";

/**
 * The line under a reply: what the turn left behind, in the teammate's own
 * words, each marked by how it went (failed first, in red), then the sites
 * of the web pages it read as a stack of their icons, then how long it took.
 * A helper or a command the turn started that failed is marked failed
 * there too, first of all: the conversation has no row of its own for one,
 * and a failure is what the person must not miss. The sources are not
 * outcomes, since nothing was left behind, so they carry no mark; they are
 * on the line because for an answer of fact they are its evidence, the one
 * part of the path someone who does not code looks for. A turn that left
 * nothing behind and read nothing says only its time. It is always the
 * way in: pressed, it opens the path the turn took, in the pane beside a
 * conversation (`onOpenWork`) or in place in a thread or a run, where the
 * live rows show only if you pressed the mark; either way the path lists
 * the sources by name first. While the turn is still on it there is no line
 * at all — the mark is the line.
 */
export const Steps = memo(function Steps({
	id,
	items,
	live,
	shown,
	onOpenWork,
	open: paneOpen,
}: {
	id: string;
	items: Step[];
	live: boolean;
	shown: boolean;
	onOpenWork?(blockId: string): void;
	open?: boolean;
}) {
	const [toggled, setToggled] = useState(false);
	const inPane = onOpenWork !== undefined;
	const open = inPane ? false : live ? shown : toggled;
	const outcomes = useMemo(() => outcomesOf(items), [items]);
	const failed = useMemo(() => failedWords(failedRuns(items)), [items]);
	const sites = useMemo(() => sitesOf(items), [items]);
	const took = useMemo(() => turnTook(items), [items]);
	const expanded = inPane ? paneOpen === true : open;

	return (
		<div className="my-2">
			{!live && (
				<button
					type="button"
					className="steps-caption"
					title={expanded ? "Hide the path" : "Show the path"}
					aria-expanded={expanded}
					onClick={() => (inPane ? onOpenWork(id) : setToggled(!open))}
				>
					{failed.map((words) => (
						<span key={words} className="outcome" data-mark="failed">
							{words}
						</span>
					))}
					{outcomes.map((outcome) => (
						<span key={outcome.id} className="outcome" data-mark={markOf(outcome)}>
							{outcome.message}
						</span>
					))}
					{sites.length > 0 && <Favicons sites={sites} most={SITES_NAMED} size={16} said={sitesSaid(sites)} />}
					<span className="steps-caption-took">
						{took !== null && took > 0 && durationText(took)}
						{open ? <ChevronDownIcon /> : <ChevronRightIcon />}
					</span>
				</button>
			)}
			{open && (
				<div className="steps ml-[11px]">
					<InPlaceSources items={items} />
					<StepRows items={items} />
				</div>
			)}
		</div>
	);
});

/** The path opened in place lists the pages read by name at its top, as the pane's does. */
function InPlaceSources({ items }: { items: Step[] }) {
	const sources = useMemo(() => sourcesOf(items), [items]);
	return sources.length > 0 ? <Sources sources={sources} /> : null;
}

/**
 * What failed among the helpers and commands a turn started, as the line
 * under the reply says it: "Helper failed: Fetch the READMEs", or "3 helpers
 * failed" for more than one, and each command by its own name.
 */
export function failedWords(runs: RunLink[]): string[] {
	const helpers = runs.filter((run) => run.runKind !== "shell");
	const commands = runs.filter((run) => run.runKind === "shell");
	const said = commands.map((run) => `Command failed: ${run.title}`);
	if (helpers.length === 1) said.unshift(`Helper failed: ${helpers[0]!.title}`);
	if (helpers.length > 1) said.unshift(`${helpers.length} helpers failed`);
	return said;
}

/**
 * Each step as a row, its detail behind a press: a run of thoughts as one
 * "Thinking" row, since harnesses send thinking in pieces that break
 * mid-sentence, and a tool with its output. What the teammate said it was on
 * heads the rows after it, and what it left behind is marked where it said it.
 */
export function StepRows({
	items,
	settled = false,
	openId,
}: {
	items: Step[];
	/** The turn is over: nothing in it is still running. */
	settled?: boolean;
	/** The row to open on arrival, brought into view: the one pressed in the peek. */
	openId?: string | undefined;
}) {
	const rows: ({ kind: "thinking"; id: string; pieces: string[] } | Exclude<Step, { kind: "thought" }>)[] = [];
	for (const item of items) {
		const last = rows[rows.length - 1];
		if (item.kind === "thought") {
			if (last?.kind === "thinking") last.pieces.push(item.text);
			else rows.push({ kind: "thinking", id: item.id, pieces: [item.text] });
		} else rows.push(item);
	}
	return rows.map((row, at) =>
		row.kind === "thinking" ? (
			<Thought key={row.id} id={row.id} text={joinThoughts(row.pieces)} live={!settled && at === rows.length - 1} startOpen={row.id === openId} />
		) : row.kind === "activity" ? (
			row.message !== undefined && (
				<p key={row.id} className="step-heading" data-step-id={row.id}>
					{row.message}
				</p>
			)
		) : row.kind === "link" ? (
			// Only a teammate's own tape holds a helper's line, and that path is
			// read in the pane (Trail), never in place.
			null
		) : row.kind === "outcome" ? (
			<p key={row.id} className="step-outcome outcome" data-step-id={row.id} data-mark={markOf(row)}>
				{row.message}
			</p>
		) : (
			<Tool
				key={row.id}
				id={row.id}
				title={row.title}
				// A harness that never closed a call leaves it running on the tape after the turn ended.
				status={settled && (row.status === "in_progress" || row.status === "pending") ? "completed" : row.status}
				output={row.output}
				startOpen={row.id === openId}
			/>
		),
	);
}

/** A row asked to open on arrival is brought into view once, where its pane can show it. */
function useArrival(startOpen: boolean) {
	const row = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (startOpen) row.current?.scrollIntoView({ block: "nearest" });
	}, [startOpen]);
	return row;
}

/** What the agent was thinking: "Thinking" until pressed, the whole thought after. */
function Thought({
	id,
	text,
	live,
	startOpen,
}: {
	id: string;
	text: string;
	/** Still arriving. */
	live: boolean;
	startOpen: boolean;
}) {
	const [open, setOpen] = useState(startOpen);
	const row = useArrival(startOpen);
	return (
		<div ref={row} data-step-id={id}>
			<button type="button" className="step" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`step-mark ${live ? "beat" : ""}`} style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
				<span className="step-title text-ink-3">Thinking</span>
				{open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />}
			</button>
			{open && <div className="step-out font-sans not-italic">{text}</div>}
		</div>
	);
}

const STATUS_INK: Record<ToolStatus, string> = {
	pending: "var(--ink-4)",
	in_progress: "var(--accent)",
	completed: "var(--ink-4)",
	failed: "var(--danger)",
};

/** A tool call: what it was, how it went, and its output behind a press. */
function Tool({
	id,
	title,
	status,
	output,
	startOpen,
}: {
	id: string;
	title: string;
	status: ToolStatus;
	output: ToolOutput[] | undefined;
	startOpen: boolean;
}) {
	// Some harnesses record an empty text output; that is nothing to open.
	const hasOutput = output !== undefined && output.some((one) => one.type === "diff" || one.text.trim() !== "");
	const [open, setOpen] = useState(startOpen && hasOutput);
	const row = useArrival(startOpen);
	return (
		<div ref={row} data-step-id={id}>
			<button
				type="button"
				className="step"
				aria-expanded={hasOutput ? open : undefined}
				disabled={!hasOutput}
				onClick={() => setOpen((was) => !was)}
			>
				<span
					aria-hidden="true"
					className={`step-mark ${status === "in_progress" ? "beat" : ""}`}
					style={{ background: STATUS_INK[status] }}
				/>
				<span className="step-title" title={title}>
					{stepTitle(title)}
				</span>
				{status === "failed" && <span className="shrink-0 text-danger">failed</span>}
				{status === "in_progress" && <span className="shrink-0 text-ink-3">running</span>}
				{hasOutput && (open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />)}
			</button>
			{open && hasOutput && <StepOutput output={output} />}
		</div>
	);
}

/** A tool's output, cleaned once per output rather than on every streamed word. */
export const StepOutput = memo(function StepOutput({ output }: { output: ToolOutput[] }) {
	const parts = useMemo(
		() => output.map((one) => ({ edit: one.type === "diff", text: outputText(one) })).filter((part) => part.text !== ""),
		[output],
	);
	return (
		<div className="step-out">
			{parts.map((part, at) => (
				<div key={at} className="step-out-part">
					{part.edit ? <EditLines text={part.text} /> : part.text}
				</div>
			))}
		</div>
	);
});

/** An edit: its path, then its lines tinted by whether they went or came, a run of one kind as one block. */
function EditLines({ text }: { text: string }) {
	const [path, ...lines] = text.split("\n");
	const runs: { kind: string; lines: string[] }[] = [];
	for (const line of lines) {
		const kind = line.startsWith("+ ") ? "step-edit-add" : line.startsWith("- ") ? "step-edit-del" : "step-edit-same";
		const last = runs[runs.length - 1];
		if (last?.kind === kind) last.lines.push(line);
		else runs.push({ kind, lines: [line] });
	}
	return (
		<>
			<div className="step-edit-path">{path}</div>
			{runs.map((run, at) => (
				<div key={at} className={run.kind}>
					{run.lines.join("\n")}
				</div>
			))}
		</>
	);
}
