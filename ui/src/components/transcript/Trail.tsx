import { useEffect, useRef, useState } from "react";
import type { ToolOutput, ToolStatus } from "../../generated/contract";
import type { Step } from "../../scheduledRuns";
import { ChevronDownIcon, ChevronRightIcon } from "../../icons";
import { joinThoughts, stepTitle } from "../../stepText";
import { StepOutput } from "./Steps";

/*
 * A turn's work as the right-hand pane reads it: a trail down one line, each
 * step a plain heading (what was done, in the core's few words) over a quiet
 * line of what was run, its time on the right. Opened, a step shows its
 * command and output as two labelled blocks, never one wall of text. Easy to
 * follow at a glance, with everything there when you want it.
 */

type Row =
	| { kind: "thinking"; id: string; ts: number; text: string }
	| Extract<Step, { kind: "tool" }>;

/** Minutes and seconds from the start of the turn: "0:14", "12:03". */
function clock(ms: number): string {
	const total = Math.max(0, Math.round(ms / 1000));
	return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** A service as a person knows it, from a tool's server name. */
function serviceOf(server: string): string {
	const plain = server.replace(/^claude_ai_/i, "");
	const named: Record<string, string> = { github: "GitHub", gmail: "Gmail", linear: "Linear", brainfile: "the board", hotline: "Hotline" };
	return named[plain.toLowerCase()] ?? plain.replace(/[_-]+/g, " ").replace(/^\w/, (c) => c.toUpperCase());
}

/**
 * What a step did, in a few words: the core's (a 0.44 desk writes them), or
 * for a step from before, a plain reading of its kind and tool.
 */
function heading(row: Row, running: boolean): string {
	if (row.kind === "thinking") return running ? "Thinking" : "Thought it through";
	if (row.said !== undefined) return running ? row.said.doing : row.said.one;
	const title = row.title.trim();
	const tool = /^mcp__([^_]+(?:_[^_]+)*?)__(?:([a-z]+)__)?/i.exec(title);
	if (tool) {
		const service = /^(claude_ai_)?prism$/i.test(tool[1]!) && tool[2] ? tool[2] : tool[1]!;
		return `Used ${serviceOf(service)}`;
	}
	switch ((row.toolKind ?? "").toLowerCase()) {
		case "read":
			return "Read a file";
		case "edit":
		case "write":
			return "Edited a file";
		case "search":
			return "Searched the files";
		case "fetch":
			return "Read a web page";
		case "execute":
			return "Ran a command";
	}
	if (/^(?:\S*\/)?(?:ba|z)?sh\s|^python\d*\s|^(?:git|npm|bun|cargo|make|gh)\s/.test(title)) return "Ran a command";
	if (/^toolsearch$/i.test(title)) return "Looked up its tools";
	return stepTitle(title).split(/\s+/).slice(0, 5).join(" ");
}

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
	/** The step to open on arrival, brought into view: the one pressed in the peek. */
	openId?: string | undefined;
}) {
	const rows: Row[] = [];
	for (const item of items) {
		const last = rows[rows.length - 1];
		if (item.kind === "thought") {
			if (last?.kind === "thinking") last.text = joinThoughts([last.text, item.text]);
			else rows.push({ kind: "thinking", id: item.id, ts: item.ts, text: item.text });
		} else rows.push(item);
	}
	const start = items[0]?.ts ?? 0;
	return (
		<ol className="trail">
			{rows.map((row, at) => {
				// A harness that never closed a call leaves it running on the tape after the turn ended.
				const status: ToolStatus =
					row.kind === "thinking"
						? !settled && at === rows.length - 1
							? "in_progress"
							: "completed"
						: settled && (row.status === "in_progress" || row.status === "pending")
							? "completed"
							: row.status;
				return <TrailStep key={row.id} row={row} status={status} at={clock(row.ts - start)} startOpen={row.id === openId} />;
			})}
		</ol>
	);
}

function TrailStep({ row, status, at, startOpen }: { row: Row; status: ToolStatus; at: string; startOpen: boolean }) {
	const running = status === "in_progress" || status === "pending";
	const ran = row.kind === "tool" ? stepTitle(row.title) : null;
	const output = row.kind === "tool" ? row.output : undefined;
	const hasOutput = output?.some((one) => one.type === "diff" || one.text.trim() !== "") ?? false;
	const openable = row.kind === "thinking" ? row.text.trim() !== "" : hasOutput || (ran !== null && ran !== "");
	const [open, setOpen] = useState(startOpen && openable);
	const node = useRef<HTMLLIElement>(null);
	useEffect(() => {
		if (startOpen) node.current?.scrollIntoView({ block: "nearest" });
	}, [startOpen]);
	return (
		<li ref={node} className="trail-step" data-step-id={row.id} data-status={status} data-thought={row.kind === "thinking" || undefined}>
			<button type="button" className="trail-head" aria-expanded={openable ? open : undefined} disabled={!openable} onClick={() => setOpen((was) => !was)}>
				<span aria-hidden="true" className={`trail-dot ${running ? "beat" : ""}`} />
				<span className="trail-words">
					<span className="trail-said">{heading(row, running)}</span>
					{ran !== null && ran !== "" && !open && <span className="trail-ran">{ran}</span>}
				</span>
				<span className="trail-when">{status === "failed" ? "failed" : running ? "now" : at}</span>
				{openable && (open ? <ChevronDownIcon className="trail-chevron" /> : <ChevronRightIcon className="trail-chevron" />)}
			</button>
			{open && (
				<div className="trail-detail">
					{row.kind === "thinking" ? (
						<p className="trail-thought">{row.text}</p>
					) : (
						<>
							{ran !== null && ran !== "" && (
								<section>
									<h4 className="trail-label">Command</h4>
									<pre className="trail-code">{ran}</pre>
								</section>
							)}
							{hasOutput && (
								<section>
									<h4 className="trail-label">{status === "failed" ? "Error" : "Output"}</h4>
									<StepOutput output={readable(output!)} />
								</section>
							)}
						</>
					)}
				</div>
			)}
		</li>
	);
}
