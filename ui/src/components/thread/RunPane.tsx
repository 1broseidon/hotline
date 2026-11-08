import { useEffect, useMemo, useState } from "react";
import type { RunningSubagent, ThreadEnd, ThreadSummary, TranscriptEvent } from "../../generated/contract";
import { stopwatch } from "../../dock";
import { ArrowLeftIcon } from "../../icons";
import type { Block } from "../../scheduledRuns";
import { useThread } from "../../tape";
import { startedWords, threadFamily } from "../../threadKinds";
import { toBlocks, withStreaming } from "../../transcriptFolds";
import { Band } from "../../ui/Band";
import { PaneClose } from "../../ui/PaneClose";
import { Markdown } from "../Markdown";
import type { ThreadRef } from "../Transcript";
import { TrailSteps, useHot } from "../transcript/Trail";
import { LiveStream, PaneHead, ResultCard, TaskCard } from "./Pane";

/*
 * A subagent's run, open in the right-hand pane (docs/design/thread-panes.html,
 * Subagent): read, never spoken in. The band says how it stands, how many
 * steps it has taken and for how long, with Stop while it works when the desk
 * can stop it. Under the head, the task its teammate handed it; then its
 * steps and words in the order they came, following the newest; and, once it
 * has reported, its last words as the result its teammate was sent.
 */

/** How a run stands, as the band says it. */
export type RunState = "Working" | "Done" | "Stopped" | "Failed";

export function runStateOf(live: boolean, end: ThreadEnd | undefined): RunState {
	if (live) return "Working";
	if (end === "failed") return "Failed";
	if (end === "cancelled" || end === "stopped") return "Stopped";
	return "Done";
}

/** How many steps (tool calls) a run has taken: a step is written again as it ends, and counted once. */
export function stepCount(events: readonly TranscriptEvent[]): number {
	return new Set(events.flatMap((event) => (event.kind === "tool" ? [event.toolCallId] : []))).size;
}

type Link = Extract<TranscriptEvent, { kind: "link" }>;

/**
 * A run's stream as the pane reads it: the task it was handed (its first
 * line from the person's side), its steps and words in the order they came,
 * and, once it has reported, its last message, which is what it reported: the
 * last words it wrote, with the bubbles that message was laid out in.
 */
export function runStream(blocks: Block[], reported: boolean): { task: string | undefined; blocks: Block[]; result: string | undefined } {
	const at = blocks.findIndex((block) => block.kind === "event" && block.event.kind === "user");
	const first = blocks[at];
	const task = first?.kind === "event" && first.event.kind === "user" ? first.event.text : undefined;
	// Its own link heads the stream, and a turn's end says nothing here: the band says how it went.
	const shown = blocks.filter((block, index) => index !== at && !(block.kind === "event" && (block.event.kind === "link" || block.event.kind === "turn")));
	const last = shown[shown.length - 1];
	if (!reported || last?.kind !== "event" || last.event.kind !== "agent") return { task, blocks: shown, result: undefined };
	// A message laid out in several bubbles is written as several lines, the later ones "<id>-2", "<id>-3".
	const newest = last.event.id;
	let from = shown.length - 1;
	for (;;) {
		const before = shown[from - 1];
		if (before?.kind !== "event" || before.event.kind !== "agent" || !newest.startsWith(`${before.event.id}-`)) break;
		from -= 1;
	}
	const result = shown
		.slice(from)
		.map((block) => (block.kind === "event" && block.event.kind === "agent" ? block.event.text : ""))
		.join("\n\n")
		.trim();
	return result === "" ? { task, blocks: shown, result: undefined } : { task, blocks: shown.slice(0, from), result };
}

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/** Now, each second while the run works, so its time counts up. */
function useSecond(running: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!running) return;
		const tick = window.setInterval(() => setNow(Date.now()), 1000);
		return () => window.clearInterval(tick);
	}, [running]);
	return now;
}

export function RunPane({
	open,
	row,
	run,
	teammate,
	onBack,
	onClose,
}: {
	open: ThreadRef;
	/** What the list knows of it, once read. */
	row: ThreadSummary | undefined;
	/** Its roster row while it runs: its start, its model, whether it can be stopped. */
	run: RunningSubagent | undefined;
	/** The teammate whose run it is, and whom it reports to. */
	teammate: { id: string; name: string; avatarHash?: string | undefined };
	onBack(): void;
	onClose(): void;
}) {
	const id = open.thread;
	const { events, streaming, cancel } = useThread(id);
	const link = useMemo(() => events.findLast((event): event is Link => event.kind === "link"), [events]);
	const saved = link?.state ?? row?.state;
	const live = saved === undefined ? run !== undefined : saved !== "closed";
	const state = runStateOf(live, link?.end ?? row?.end);
	const blocks = useMemo(() => withStreaming(toBlocks(events, true), streaming), [events, streaming]);
	const read = useMemo(() => runStream(blocks, state === "Done"), [blocks, state]);
	const { task, blocks: shown, result } = read;
	const steps = stepCount(events);
	const startedAt = run?.startedAt ?? row?.startedAt ?? link?.ts;
	const now = useSecond(live);
	const took = startedAt === undefined ? undefined : live ? now - startedAt : (link?.elapsedMs ?? (row === undefined ? undefined : row.updatedAt - startedAt));
	const endedAt = startedAt !== undefined && took !== undefined ? startedAt + took : row?.updatedAt;
	const title = run?.title ?? row?.title ?? link?.title ?? open.title ?? "Subagent";
	const [refused, setRefused] = useState<string | null>(null);
	const [stopping, setStopping] = useState(false);
	const hot = useHot();
	const stop = () => {
		setRefused(null);
		setStopping(true);
		void cancel().catch((error: unknown) => {
			setStopping(false);
			setRefused(error instanceof Error ? error.message : String(error));
		});
	};
	const counted = [`${steps} ${steps === 1 ? "step" : "steps"}`, took === undefined ? null : stopwatch(took)].filter((part) => part !== null).join(" · ");
	const at = endedAt === undefined ? "" : ` at ${clock.format(new Date(endedAt))}`;
	const foot = live ? `Read only. Reports to ${teammate.name}.` : state === "Done" ? `Reported to ${teammate.name}${at}.` : `${state}${at}.`;

	return (
		<section className="dock-page run-pane" aria-label={`Subagent of ${teammate.name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<h2 className="flex min-w-0 flex-1 items-center gap-2 truncate pl-1 font-mono text-sm font-medium text-ink">
					<span aria-hidden="true" className={`run-dot ${live ? "beat" : ""}`} data-state={state} />
					<span className="truncate">{state}</span>
				</h2>
				<span className="instrument shrink-0 pr-1">{counted}</span>
				{live && run?.stoppable === true && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Stop this subagent" disabled={stopping} onClick={stop}>
						Stop
					</button>
				)}
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead
				kind={threadFamily("run", teammate.name)}
				title={title}
				meta={[`sent by ${teammate.name}`, run?.model, startedAt !== undefined && startedWords(startedAt)]}
			/>
			{task !== undefined && <TaskCard eyebrow="Task" from={{ personaId: teammate.id, name: teammate.name, hash: teammate.avatarHash }} text={task} />}
			<LiveStream following={id.key} count={shown.length} content={read} live={live}>
				{shown.map((block) => (
					<StreamBlock key={block.kind === "event" ? block.event.id : block.id} block={block} live={live} hot={hot} />
				))}
				{result !== undefined && <ResultCard eyebrow={`Result · sent to ${teammate.name}`} aside={took === undefined ? undefined : stopwatch(took)} text={result} />}
				{shown.length === 0 && result === undefined && <p className="work-empty">{live ? "Starting." : "Nothing was written."}</p>}
			</LiveStream>
			{refused !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused}</p>}
			<p className="run-foot">{foot}</p>
		</section>
	);
}

/** One part of the stream: a run of steps, each opening to what it did, or the run's words. */
function StreamBlock({ block, live, hot }: { block: Block; live: boolean; hot: ReturnType<typeof useHot> }) {
	if (block.kind === "steps") return <TrailSteps items={block.items} settled={!live || block.open !== true} hot={hot} />;
	if (block.kind !== "event") return null;
	const event = block.event;
	switch (event.kind) {
		case "agent":
			return (
				<div className="run-say selectable">
					<Markdown text={event.text} />
				</div>
			);
		case "user":
			// Its teammate told it more after the task.
			return <p className="run-told selectable">{event.text}</p>;
		case "notice":
			return <p className="run-notice selectable" data-level={event.level}>{event.text}</p>;
		default:
			return null;
	}
}
