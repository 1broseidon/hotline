import { type ReactNode, memo, useMemo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { processLabel } from "../activity";
import { chordKeys } from "../chords";
import type { ProcessOutput, SessionProcess } from "../generated/contract";
import { ArrowDownIcon, CloseIcon, SidebarIcon } from "../icons";
import { dmOf, useThread } from "../tape";
import { Band } from "../ui/Band";
import { wire } from "../wire";
import { runEnding, stepRuns } from "./Transcript";
import { factOf, workDetail } from "../turnFacts";
import { glance } from "../glance";
import { Trail } from "./transcript/Trail";
import type { Step } from "../scheduledRuns";

/**
 * What the card shows: a turn's work, by its caption's id, or the turn
 * running now; or, with `processId`, a command the teammate left running.
 */
export type OpenWork = { personaId: string; blockId: string | null; processId?: string };

/** The same work: pressing what opened a card, with it open, closes it. */
export function sameWork(a: OpenWork, b: OpenWork): boolean {
	return a.blockId === b.blockId && a.processId === b.processId;
}

/** How often an open process card reads its output again; the core only answers when asked. */
const OUTPUT_EVERY_MS = 1000;

/**
 * A command the teammate left running, in the work card's place: the whole
 * command, the latest of what it has written, and Stop. Stop is the person's,
 * so the teammate is not woken by the end; the card says it ended when the
 * session stops listing the process, and keeps the last output it read.
 */
export function ProcessWork({
	personaId,
	processId,
	process,
	docked = true,
	onClose,
}: {
	personaId: string;
	processId: string;
	/** The process as the session lists it; none once it has ended. */
	process: SessionProcess | undefined;
	docked?: boolean;
	onClose(): void;
}) {
	const [read, setRead] = useState<ProcessOutput | null>(null);
	const [refused, setRefused] = useState<string | null>(null);
	const running = process !== undefined;
	useEffect(() => {
		if (!running) return;
		let cancelled = false;
		const readOutput = () => {
			void wire
				.command("process.output", { personaId, id: processId })
				.then((next) => !cancelled && setRead(next))
				.catch(() => {});
		};
		readOutput();
		const tick = window.setInterval(readOutput, OUTPUT_EVERY_MS);
		return () => {
			cancelled = true;
			window.clearInterval(tick);
		};
	}, [personaId, processId, running]);
	const stop = () => {
		setRefused(null);
		void wire.command("process.stop", { personaId, id: processId }).catch((error: unknown) => setRefused(error instanceof Error ? error.message : String(error)));
	};
	const heading = process === undefined ? "Ended" : process.state === "stopping" ? "Stopping" : "Running";
	const title = process?.title ?? read?.command;
	const tail = read?.tail ?? "";

	return (
		<WorkCard
			label="Running process"
			heading={heading}
			working={false}
			{...(title !== undefined ? { detail: processLabel(title) } : {})}
			docked={docked}
			following={processId}
			count={tail.length}
			content={tail}
			actions={
				process !== undefined && (
					<button type="button" className="control btn btn-sm" disabled={process.state !== "running"} onClick={stop}>
						Stop
					</button>
				)
			}
			onClose={onClose}
		>
			{read !== null && <p className="selectable mx-2 mb-2 break-words font-mono text-xs text-ink-2">{read.command}</p>}
			{refused !== null && <p className="mx-2 mb-2 text-sm text-danger">{refused}</p>}
			{tail === "" ? (
				<p className="work-empty">{read === null ? "Reading its output." : "Nothing written yet."}</p>
			) : (
				<pre className="selectable mx-2 whitespace-pre-wrap break-words font-mono text-xs">{tail}</pre>
			)}
		</WorkCard>
	);
}

/** Slack under the newest step that still counts as following it. */
const FOLLOW_SLACK = 48;

/** A turn's work: the run a caption names, or the latest, and how it stands. */
function useTurnWork(personaId: string, blockId: string | null, name: string, live: boolean) {
	const { events, streaming } = useThread(dmOf(personaId));
	const runs = useMemo(() => stepRuns(events, streaming), [events, streaming]);
	const latest = runs[runs.length - 1];
	const run = blockId === null ? latest : runs.find((one) => one.id === blockId);
	const running = live && run !== undefined && run === latest;
	const waiting = blockId === null && live && run === undefined;
	const state = running || waiting ? "Working" : run && runEnding(events, run.items) === "stopped" ? "Stopped" : "Done";
	const items = run?.items;
	const content = useMemo(() => ({ items, running, waiting, name }), [items, running, waiting, name]);
	const empty = <p className="work-empty">{waiting ? `${name} is getting started.` : "This turn's steps are no longer on the tape."}</p>;
	return { run, running, state, content, empty };
}

/**
 * The peek: a teammate's turn as a glance, sliding up from behind the
 * composer over the conversation's foot. Four plain lines that follow the
 * newest, nothing in them to open; a line pressed (or the pane key) opens the
 * turn in the right-hand pane, that line open there, and the peek goes. A
 * caption opens its own turn; the work card opens the turn running now. A
 * subagent's run, a side thread or a call is a thread, and opens in the pane.
 */
export function Work({
	open,
	name,
	live,
	docked = true,
	onPick,
	onClose,
}: {
	open: OpenWork;
	name: string;
	/** A turn is running for this teammate. */
	live: boolean;
	docked?: boolean;
	/** Opens the turn in the right-hand pane, at the line pressed or at its top. */
	onPick?: ((stepId: string | null) => void) | undefined;
	onClose(): void;
}) {
	const { run, running, state, content, empty } = useTurnWork(open.personaId, open.blockId, name, live);
	return (
		<WorkCard
			label={`${name}'s work`}
			heading={state}
			working={state === "Working"}
			{...(run !== undefined ? { detail: workDetail(run.items) } : {})}
			docked={docked}
			following={run?.id}
			count={run?.items.length ?? 0}
			content={content}
			actions={
				onPick !== undefined && (
					<button type="button" className="control btn-icon" title="Open in pane" aria-label="Open the work in the pane" onClick={() => onPick(null)}>
						<SidebarIcon />
					</button>
				)
			}
			onClose={onClose}
		>
			{run === undefined ? empty : <GlanceRows items={run.items} running={running} onPick={onPick} />}
		</WorkCard>
	);
}

/**
 * The peek's lines: what was done in a few words, a run of one kind as one
 * line (glance.ts), each a press away from its steps in the pane. The pane
 * shows the steps themselves, each as it was run.
 */
export function GlanceRows({ items, running, onPick }: { items: Step[]; running: boolean; onPick: ((stepId: string | null) => void) | undefined }) {
	const lines = useMemo(() => glance(items, running), [items, running]);
	return lines.map((line) => {
		const fact = factOf(line.steps, line.live);
		return (
			<div key={line.id} data-step-id={line.id}>
				<button type="button" className="step" onClick={() => onPick?.(line.id)}>
					<span
						aria-hidden="true"
						className={`step-mark ${line.live ? "beat" : ""}`}
						style={{ background: line.failed ? "var(--danger)" : line.live ? "var(--accent)" : "var(--ink-4)" }}
					/>
					<span className="step-title">{line.text}</span>
					{fact !== null && (
						<span className="step-fact" data-tone={fact.tone}>
							{fact.text}
						</span>
					)}
				</button>
			</div>
		);
	});
}

/**
 * A turn's work as a page of the right-hand pane, flush and full height like
 * a thread: the room to read it, every step opening to say what it did. The
 * line pressed in the peek arrives open and in view; otherwise the page
 * follows the newest step, as the peek does.
 */
export function WorkPage({
	personaId,
	blockId,
	stepId,
	name,
	live,
	onClose,
}: {
	personaId: string;
	blockId: string | null;
	/** The line pressed in the peek, opened on arrival. */
	stepId?: string | undefined;
	name: string;
	live: boolean;
	onClose(): void;
}) {
	const { run, running, state, content, empty } = useTurnWork(personaId, blockId, name, live);
	return (
		<section className="work-page" aria-label={`${name}'s work`}>
			<WorkBand heading={state} working={state === "Working"} detail={run !== undefined ? workDetail(run.items) : undefined} actions={null} onClose={onClose} />
			<FollowWindow following={run?.id} count={run?.items.length ?? 0} content={content} follow={stepId === undefined}>
				{run === undefined ? empty : <Trail key={stepId ?? ""} items={run.items} settled={!running} openId={stepId} />}
			</FollowWindow>
		</section>
	);
}

/** The card itself: its band, and a window that follows the newest until you scroll back. */
function WorkCard({
	label,
	heading,
	working,
	detail,
	docked,
	following: followKey,
	count,
	content,
	actions,
	onClose,
	children,
}: {
	label: string;
	heading: string;
	working: boolean;
	detail?: string;
	docked: boolean;
	/** What the window is following: a new one starts at its newest. */
	following: string | undefined;
	count: number;
	/** Changes when the visible body changes, including a growing thought or tool output. */
	content: unknown;
	/** Keys in the band, before Close. */
	actions?: ReactNode;
	onClose(): void;
	children: ReactNode;
}) {
	return (
		<aside className={docked ? "work-float work-docked" : "work-float"} aria-label={label}>
			<WorkBand heading={heading} working={working} detail={detail} actions={actions} onClose={onClose} />
			<FollowWindow following={followKey} count={count} content={content}>
				{children}
			</FollowWindow>
		</aside>
	);
}

/** Body updates do not repaint the card's controls. */
const WorkBand = memo(function WorkBand({ heading, working, detail, actions, onClose }: {
	heading: string;
	working: boolean;
	detail: string | undefined;
	actions: ReactNode;
	onClose(): void;
}) {
	return (
		<Band>
			{/* The heading is the run's state, so it reads in the instrument face, as states do.
			 * It is the band's own h2, so a press on it or the space after it drags the window. */}
			<h2 className="flex min-w-0 flex-1 items-center gap-2 truncate pl-1 font-mono text-sm font-medium text-ink">
				{working && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
				<span className="truncate">{heading}</span>
			</h2>
			{detail !== undefined && <span className="instrument min-w-0 truncate pr-1">{detail}</span>}
			{actions}
			<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
				<CloseIcon />
			</button>
		</Band>
	);
});

/**
 * The window a card reads through: it follows the newest line until you
 * scroll back, and offers a way down again.
 */
function FollowWindow({
	following: followKey,
	count,
	content,
	follow = true,
	children,
}: {
	/** What the window is following: a new one starts at its newest. */
	following: string | undefined;
	count: number;
	/** Changes when the visible body changes, including a growing thought or tool output. */
	content: unknown;
	/** Whether it starts at the newest; a page opened at a line it was asked for starts there instead. */
	follow?: boolean;
	children: ReactNode;
}) {
	const frame = useRef<HTMLDivElement>(null);
	const [following, setFollowing] = useState(follow);
	// Follow actual body changes, including text growing within one step.
	// Unrelated parent renders need no layout read.
	// biome-ignore lint/correctness/useExhaustiveDependencies: content and followKey identify the body whose layout changed.
	useLayoutEffect(() => {
		const el = frame.current;
		if (el && following) el.scrollTop = el.scrollHeight;
	}, [content, followKey, following]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new followKey is new work to follow.
	useEffect(() => setFollowing(follow), [followKey]);
	// Opened at a line, with the whole run in view: there is nothing newer to go down to.
	// biome-ignore lint/correctness/useExhaustiveDependencies: measured once the body has laid out.
	useEffect(() => {
		const el = frame.current;
		if (el && !following && el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK) setFollowing(true);
	}, [content, followKey]);

	return (
		<div className="relative flex min-h-0 flex-1 flex-col">
			<div
				ref={frame}
				className="work-window"
				onScroll={(scroll) => {
					const el = scroll.currentTarget;
					const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK;
					if (atEnd !== following) setFollowing(atEnd);
				}}
			>
				{children}
			</div>
			{!following && count > 0 && (
				<button
					type="button"
					className="control btn btn-sm work-newest gap-1"
					onClick={() => {
						setFollowing(true);
						frame.current?.scrollTo({ top: frame.current.scrollHeight, behavior: "smooth" });
					}}
				>
					<ArrowDownIcon />
					Newest
				</button>
			)}
		</div>
	);
}
