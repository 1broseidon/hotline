import { memo, useMemo, useState } from "react";
import type { ToolOutput, ToolStatus } from "../../generated/contract";
import type { Step } from "../../scheduledRuns";
import { ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { joinThoughts, outputText, stepTitle } from "../../stepText";
import { stepsSummary } from "../../transcriptFolds";

/**
 * The machinery between two messages, as one caption: a count, closed
 * until you open it. While the agent is still on it there is no caption at
 * all — the typing bubble is the caption. In a conversation the caption
 * opens the work in the pane beside it (`onOpen`); in a thread or a run it
 * opens in place, and the live rows show only if you pressed the bubble.
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
	const summary = stepsSummary(items);
	const failed = items.some((one) => one.kind === "tool" && one.status === "failed");

	return (
		<div className="my-2">
			{!live && (
				<button
					type="button"
					className="steps-caption"
					aria-expanded={inPane ? paneOpen === true : open}
					onClick={() => (inPane ? onOpenWork(id) : setToggled(!open))}
				>
					<span className={`truncate ${failed ? "text-danger" : ""}`}>{summary}</span>
					{open ? <ChevronDownIcon /> : <ChevronRightIcon />}
				</button>
			)}
			{open && (
				<div className="steps ml-[11px]">
					<StepRows items={items} />
				</div>
			)}
		</div>
	);
});

/**
 * Each step as a row, its detail behind a press: a run of thoughts as one
 * "Thinking" row, since harnesses send thinking in pieces that break
 * mid-sentence, and a tool with its output.
 */
export function StepRows({ items, settled = false }: { items: Step[]; /** The turn is over: nothing in it is still running. */ settled?: boolean }) {
	const rows: ({ kind: "thinking"; id: string; pieces: string[] } | Extract<Step, { kind: "tool" }>)[] = [];
	for (const item of items) {
		const last = rows[rows.length - 1];
		if (item.kind === "thought") {
			if (last?.kind === "thinking") last.pieces.push(item.text);
			else rows.push({ kind: "thinking", id: item.id, pieces: [item.text] });
		} else rows.push(item);
	}
	return rows.map((row, at) =>
		row.kind === "thinking" ? (
			<Thought key={row.id} id={row.id} text={joinThoughts(row.pieces)} live={!settled && at === rows.length - 1} />
		) : (
			<Tool
				key={row.id}
				id={row.id}
				title={row.title}
				// A harness that never closed a call leaves it running on the tape after the turn ended.
				status={settled && (row.status === "in_progress" || row.status === "pending") ? "completed" : row.status}
				output={row.output}
			/>
		),
	);
}

/** What the agent was thinking: "Thinking" until pressed, the whole thought after. */
function Thought({ id, text, live }: { id: string; text: string; /** Still arriving. */ live: boolean }) {
	const [open, setOpen] = useState(false);
	return (
		<div data-step-id={id}>
			<button type="button" className="step" aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`step-mark ${live ? "beat" : ""}`} style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
				<span className="step-title font-sans italic text-ink-3">Thinking</span>
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
}: {
	id: string;
	title: string;
	status: ToolStatus;
	output: ToolOutput[] | undefined;
}) {
	const [open, setOpen] = useState(false);
	// Some harnesses record an empty text output; that is nothing to open.
	const hasOutput = output !== undefined && output.some((one) => one.type === "diff" || one.text.trim() !== "");
	return (
		<div data-step-id={id}>
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
const StepOutput = memo(function StepOutput({ output }: { output: ToolOutput[] }) {
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
