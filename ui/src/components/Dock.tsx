import { startCall } from "../voice/call";
import { type CSSProperties, type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Attachment, RunningSubagent, ThreadId, ThreadSummary } from "../generated/contract";
import { PaneClose } from "../ui/PaneClose";
import { ArrowLeftIcon } from "../icons";
import { carry } from "../serverFiles";
import { sends, useReturned, useSends } from "../sends";
import type { Refill } from "./Conversation";
import {
	clampDock,
	DOCK_MAX,
	DOCK_MIN,
	DOCK_STEP,
	DOCK_WIDTH,
	draggedDock,
	openerOf,
	openerWords,
	pairWith,
	powersOf,
	rowTitle,
	threadOwnerId,
	turnRunning,
	withTurns,
} from "../dock";
import { type Person, usePeople } from "../avatars";
import { useAsides } from "../asides";
import { sideTitle } from "../links";
import { answerCard, sameThread, useThread } from "../tape";
import { nameIn, useJobs, useThreadRows } from "../threadRows";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { wire, type RosterEntry } from "../wire";
import { isWorking, workingState } from "../activity";
import { Composer } from "./Composer";
import { Transcript, type ThreadRef } from "./Transcript";
import { WorkPage } from "./Work";
import { ThreadList } from "./ThreadList";
import { LoopIn } from "./LoopIn";
import { BandKind, PaneHead } from "./thread/Pane";
import { RunPane } from "./thread/RunPane";
import { HandoffPane } from "./thread/HandoffPane";
import { PairPane } from "./thread/PairPane";
import { CommandPane, runLink } from "./thread/CommandPane";
import { CallPane } from "./thread/CallPane";
import { JobPane } from "./thread/JobPane";
import { startedWords, threadFamily } from "../threadKinds";

/**
 * What the right-hand pane shows: the list, one thread open in it, or a
 * teammate's turn of work (`work`), opened from the peek at the line pressed.
 */
export type DockState = {
	open: ThreadRef | null;
	work?: { personaId: string; blockId: string | null; stepId?: string };
};

/**
 * The right-hand pane: this teammate's threads as a list, like the team on the
 * left, and any one of them open in it. A work thread, a pair, a subagent's run
 * and a call are all read the same way, through the one conversation view. A
 * row opens its thread in the pane with a way back to the list; the pane's edge
 * is dragged to size it, and in a window too narrow for it beside the
 * conversation it lies over the conversation instead (`overlay`).
 */
export function Dock({
	state,
	onState,
	onClose,
	entry,
	roster,
	width,
	onWidth,
	overlay,
	onOpenTeammate,
}: {
	state: DockState;
	onState(next: DockState): void;
	onClose(): void;
	/** Opens a teammate's conversation: where a handoff is steered, with whoever handed it over. */
	onOpenTeammate(personaId: string): void;
	/** Whose threads the pane lists; none when nobody is open. */
	entry: RosterEntry | null;
	roster: RosterEntry[];
	width: number;
	onWidth(width: number): void;
	overlay: boolean;
}) {
	const root = useRef<HTMLElement>(null);
	/* Opening the pane moves focus into it, and closing it puts focus back
	 * where it was, the way a menu does. */
	useEffect(() => {
		const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		return () => {
			if (before?.isConnected) before.focus();
		};
	}, []);

	const teammate = entry === null ? null : { id: entry.persona.id, name: entry.persona.name, avatarHash: entry.persona.avatar?.hash };
	const { rows: listed, list, reload } = useThreadRows(entry);
	const listedFor = entry?.persona.id ?? "";
	const rows = useMemo(() => withTurns(listed, roster, listedFor), [listed, roster, listedFor]);
	const jobs = useJobs(entry?.persona.id);
	/* A teammate on a linked desk is in no roster here; its pair's row names it ("Lou on grizzly"). */
	const nameOf = useCallback((personaId: string) => nameIn(roster, rows, personaId), [roster, rows]);
	const people = usePeople(roster);
	/* The row a thread was opened from, so that going back lands on it. */
	const opened = useRef<string | null>(null);
	const at = state.open;
	// Opened from a line in the conversation too, not only from the list; a note keeps its row.
	if (at !== null && !opened.current?.startsWith("note:")) opened.current = `${at.thread.kind}:${at.thread.key}`;
	/* A turn's work, for the teammate it belongs to; another teammate open shows its list. */
	const work = state.work !== undefined && entry !== null && entry.persona.id === state.work.personaId ? state.work : undefined;
	const row = at === null ? undefined : rows.find((one) => sameThread(one.thread, at.thread));
	const owner = roster.find((one) => one.persona.id === threadOwnerId(row, listedFor));
	const back = { open: null };

	return (
		<div className="dock-slot" data-overlay={overlay || undefined} style={{ "--dock-width": `${width}px` } as CSSProperties}>
			<DockEdge width={width} onWidth={onWidth} />
			<aside ref={root} className="dock" aria-label={work !== undefined ? "Work" : "Threads"}>
				{work !== undefined && entry !== null ? (
					<WorkPage
						key={`${work.blockId ?? "now"}:${work.stepId ?? ""}`}
						personaId={work.personaId}
						blockId={work.blockId}
						stepId={work.stepId}
						name={entry.persona.name}
						live={isWorking(entry.session)}
						onClose={onClose}
					/>
				) : at === null && <DockBand onClose={onClose} />}
				{work !== undefined ? null : at === null ? (
					<ThreadList
						key={entry?.persona.id ?? ""}
						entry={entry}
						rows={rows}
						jobs={jobs}
						runs={entry?.subagents ?? NO_RUNS}
						loaded={list !== undefined}
						nameOf={nameOf}
						opened={opened}
						onOpen={(open) => onState({ open })}
					/>
				) : (
					teammate !== null &&
					(at.thread.kind === "call" ? (
						<CallPane key={`call:${at.thread.key}`} open={at} row={row} teammate={teammate} working={turnRunning(owner, at.thread, row)} onChanged={reload} onOpen={(open) => onState({ open })} onBack={() => onState(back)} onClose={onClose} />
					) : at.thread.kind === "job" ? (
						<JobPane key={`job:${at.thread.key}`} open={at} row={row} job={jobs.find((one) => one.id === at.thread.key)} teammate={teammate} working={turnRunning(owner, at.thread, row)} onChanged={reload} onBack={() => onState(back)} onClose={onClose} />
					) : (
						<ThreadView
							key={`${at.thread.kind}:${at.thread.key}`}
							open={at}
							row={row}
							teammate={teammate}
							nameOf={nameOf}
							people={people}
							working={turnRunning(owner, at.thread, row)}
							roster={roster}
							onChanged={reload}
							onOpen={(open) => onState({ open })}
							onBack={() => onState(back)}
							onClose={onClose}
							onOpenTeammate={onOpenTeammate}
						/>
					))
				)}
			</aside>
		</div>
	);
}

const NO_RUNS: RunningSubagent[] = [];

/** The pane's header: what it holds, and the way out. */
function DockBand({ onClose }: { onClose(): void }) {
	return (
		<Band>
			<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Threads</h2>
			<PaneClose onClose={onClose} />
		</Band>
	);
}

/** Focus lands on the pane's first useful thing when a page of it opens. */
function useFocusOnOpen(root: RefObject<HTMLElement | null>, first: string | string[], ready = true) {
	const done = useRef(false);
	const selector = Array.isArray(first) ? first.join("|") : first;
	useEffect(() => {
		if (done.current || !ready) return;
		const dock = root.current?.closest(".dock");
		const target = selector.split("|").map((one) => dock?.querySelector<HTMLElement>(one)).find((one) => one != null);
		if (target === null || target === undefined) return;
		done.current = true;
		target.focus();
	}, [root, selector, ready]);
}

/* ------------------------------------------------------------ one thread */

/**
 * A thread, open in the pane, whatever its kind: under its band a head that
 * says what kind of thread it is and what the person is to it, then the same
 * conversation view the main chat is, and below it a composer only where it
 * can be spoken in. A work
 * thread is the same teammate in a second conversation, answerable, and a
 * parked one says so: saying something in it brings its agent back. Loop in…
 * brings a colleague in for one answer, and End thread ends it, and the
 * handoff into it when a teammate handed it over; once closed the page is the
 * thread, read-only, with a Continue where
 * the composer was. A pair and a subagent's run are read, not spoken
 * in: a run is told its task once and reports once. A call and a job have
 * panes of their own (thread/CallPane, thread/JobPane).
 */
function ThreadView({
	open,
	row,
	teammate,
	nameOf,
	people,
	working,
	roster,
	onChanged,
	onOpen,
	onBack,
	onClose,
	onOpenTeammate,
}: {
	open: ThreadRef;
	/** The team, for Loop in…'s colleagues. */
	roster: RosterEntry[];
	/** What the list knows of it: its state and its title. */
	row: ThreadSummary | undefined;
	teammate: { id: string; name: string; avatarHash?: string | undefined };
	nameOf(personaId: string): string | undefined;
	people: ReadonlyMap<string, Person>;
	/** A turn of this thread is running: see `turnRunning`. */
	working: boolean;
	/** The thread was archived or continued: the list reads itself again. */
	onChanged(): void;
	onOpen(open: ThreadRef): void;
	onBack(): void;
	onClose(): void;
	onOpenTeammate(personaId: string): void;
}) {
	const id = open.thread;
	const { events, streaming, more, earlier, prompt, cancel, interrupt, close, resume } = useThread(id);
	const state = row?.state ?? "live";
	// A teammate's handoff is the two teammates' work: the person speaks in it, and only its result ends it.
	const opener = openerOf(row);
	// Opened from the pane of whoever handed it over, a handoff is still the receiver's: its lines and its bar name them.
	const ownerId = threadOwnerId(row, teammate.id);
	const self = ownerId === teammate.id ? teammate : { id: ownerId, name: nameOf(ownerId) ?? "a teammate", avatarHash: people.get(ownerId)?.hash };
	const powers = powersOf(id.kind, state, opener !== undefined);
	const closed = state === "closed";
	const parked = state === "parked";
	const turning = working && !closed;
	const other = id.kind === "pair" ? pairWith(id.key, self.id) : undefined;
	const withName = open.withName ?? (other === undefined ? undefined : nameOf(other)) ?? "a teammate";
	const title = row !== undefined ? rowTitle(row, self.id, nameOf) : (id.kind === "side" ? sideTitle(open.title) : (open.title ?? (id.kind === "pair" ? `With ${withName}` : "Thread")));
	const from = row === undefined ? "" : openerWords(row, teammate.id, nameOf);
	// What the thread is, under the band: a run is a command when its roster row says so, or once it has ended its own line, and the teammate left it running.
	const ownerEntry = roster.find((one) => one.persona.id === self.id);
	const listedRun = id.kind === "run" ? ownerEntry?.subagents?.find((run) => run.runId === id.key) : undefined;
	const runKind = id.kind === "run" ? (listedRun?.kind ?? runLink(events)?.runKind) : undefined;
	const kind = threadFamily(id.kind, id.kind === "run" ? self.name : opener?.name, runKind);
	// The link is the thread's own line in the conversation that holds it; here the thread is the page.
	const lines = useMemo(() => events.filter((event) => event.kind !== "link"), [events]);
	// A pair draws the asides its two teammates left each other: each teammate's are read from what it was left.
	const leftSelf = useAsides(id.kind === "pair" ? self.id : undefined, ownerEntry ?? null);
	const leftOther = useAsides(id.kind === "pair" ? other : undefined, roster.find((one) => one.persona.id === other) ?? null);
	const pairAsides = useMemo(() => [...(leftSelf ?? []), ...(leftOther ?? [])], [leftSelf, leftOther]);
	const workingIds = id.kind === "pair" ? roster.flatMap((one) => (one.sides ?? []).filter((side) => side.working).map((side) => side.sideId)).join(" ") : "";
	const workingSides = useMemo(() => (workingIds === "" ? [] : workingIds.split(" ")), [workingIds]);
	const [refused, setRefused] = useState<string | null>(null);
	const root = useRef<HTMLElement>(null);
	// The composer is where this page is for; a closed one has Continue.
	useFocusOnOpen(root, "textarea, [data-autofocus]");

	// A pair is read where it is open: the other side's lines are then seen.
	useEffect(() => {
		if (id.kind !== "pair") return;
		const eventIds = events.filter((event) => event.kind === "user" || event.kind === "agent").map((event) => event.id);
		if (eventIds.length > 0) void wire.command("peers.mark_read", { key: id.key, eventIds });
	}, [id.kind, id.key, events]);

	const refuse = (error: unknown) => setRefused(error instanceof Error ? error.message : String(error));
	// A refused message goes back to this thread's field, even when the person has gone elsewhere before the refusal came.
	const conv = `${id.kind}:${id.key}`;
	const [refill, setRefill] = useState<Refill | null>(null);
	const threadSends = useSends(conv);
	useReturned(conv, (back) => setRefill({ text: back.text, attachments: back.attachments, nonce: back.nonce, merge: true }));
	const send = (text: string, attachments: Attachment[]) => {
		setRefused(null);
		sends.send(conv, { text, attachments }, () => carry(attachments).then((carried) => prompt(text, carried)));
	};
	// End thread: closes it, and a handoff into it with it, as the person's decision.
	const end = () => {
		setRefused(null);
		void close().then(onChanged, refuse);
	};
	const resumeIt = () => {
		setRefused(null);
		void resume().then(onChanged, refuse);
	};
	// What a handoff asks the person, the oldest first: the one thing they say in it.
	const asked = opener === undefined ? undefined : lines.find((event): event is Extract<typeof event, { kind: "human_action" }> => event.kind === "human_action" && event.status === "pending");

	// A pair, a command and a subagent are watched, not spoken in: each is its own pane.
	if (id.kind === "pair") {
		const them = other ?? "";
		return (
			<PairPane
				threadKey={id.key}
				self={{ id: self.id, name: self.name, hash: self.avatarHash }}
				other={{ id: them, name: row?.withName ?? withName, hash: people.get(them)?.hash }}
				row={row}
				events={lines}
				asides={pairAsides}
				workingSides={workingSides}
				people={people}
				handoff={open.handoff}
				onBack={onBack}
				onClose={onClose}
			/>
		);
	}
	if (id.kind === "run" && runKind === "shell") {
		return <CommandPane personaId={self.id} runId={id.key} owner={self.name} cwd={ownerEntry?.persona.cwd} run={listedRun} events={events} onStop={cancel} onBack={onBack} onClose={onClose} />;
	}
	if (id.kind === "run") {
		return <RunPane open={open} row={row} run={listedRun} teammate={self} events={events} streaming={streaming} onStop={cancel} onBack={onBack} onClose={onClose} />;
	}

	return (
		<section ref={root} className="dock-page" aria-label={`Thread with ${self.name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back" aria-label="Back" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<BandKind kind={kind} live={turning} state={turning ? "working" : state} tone={row?.end === "failed" ? "failed" : "quiet"} />
                {id.kind === "side" && !closed && opener === undefined && (
                    <button type="button" className="control btn-quiet px-2 text-sm" title="Call this thread" onClick={() => void startCall(undefined, { personaId: self.id, name: self.name, thread: id }).catch(refuse)}>Call</button>
                )}
				{powers.resume && (
					<button type="button" className="control btn-quiet px-2 text-sm" data-autofocus title="Resume this thread" onClick={resumeIt}>
						Continue
					</button>
				)}
				{powers.loopIn && (
					<LoopIn thread={id} owner={self.id} roster={roster} people={people} onRefused={refuse} />
				)}
				{powers.close && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="End this thread" onClick={end}>
						End thread
					</button>
				)}
				{powers.stop && turning && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Stop this turn" onClick={() => void cancel().catch(refuse)}>
						Stop
					</button>
				)}
				<PaneClose onClose={onClose} />
			</Band>
			<PaneHead
				kind={kind}
				title={title}
				meta={
					opener !== undefined
						? [`from ${opener.name}`, `to ${self.name}`, row?.handoffEnded === true ? "the handoff has ended" : `answers go back to ${opener.name}`]
						: [from === "" ? self.name : from, row !== undefined && startedWords(row.startedAt)]
				}
			/>
			<div className="relative flex min-h-0 flex-1 flex-col">
				{opener !== undefined ? (
					<HandoffPane thread={id} events={lines} streaming={streaming} live={turning} receiver={{ id: self.id, name: self.name, hash: self.avatarHash }} sender={opener} people={people} />
				) : (
					<Transcript
						key={`${id.kind}:${id.key}`}
						personaId={self.id}
						thread={id}
						name={self.name}
						avatarHash={self.avatarHash}
						people={people}
						events={lines}
						streaming={streaming}
						live={turning}
						focus={null}
						more={more}
						onEarlier={earlier}
						onOpenThread={onOpen}
					/>
				)}
			</div>
			{(refused ?? threadSends.notice) !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused ?? threadSends.notice}</p>}
			{opener !== undefined && asked === undefined && (
				<HandoffBar opener={opener} teammate={self} ended={row?.handoffEnded === true} people={people} onTalk={() => onOpenTeammate(opener.personaId)} />
			)}
			{opener !== undefined && asked !== undefined ? (
				<AnswerField thread={id} name={self.name} actionId={asked.actionId} onRefused={refuse} />
			) : powers.say ? (
				<>
					{parked && (
						<p className="dock-note selectable">
							Parked: no agent is running. Saying something here picks it back up where it left off.
						</p>
					)}
					<Composer
						embedded
						personaId={`${id.kind}:${id.key}`}
						name={self.name}
						state={workingState(turning)}
						{...(refill !== null ? { refill } : {})}
						replyQuote={null}
						onSend={send}
						onCancel={() => void interrupt().catch(() => undefined)}
						onClearReply={() => undefined}
					/>
				</>
			) : (
				closed && row?.outcome !== undefined && row.outcome !== "" && <p className="dock-note selectable">{row.outcome}</p>
			)}
		</section>
	);
}

/**
 * Above the composer in a thread a teammate handed over: who handed it over,
 * whether the handoff still stands, as the desk records it, and the way to
 * that one's conversation.
 */
function HandoffBar({
	opener,
	teammate,
	ended,
	people,
	onTalk,
}: {
	opener: { personaId: string; name: string };
	teammate: { id: string; name: string; avatarHash?: string | undefined };
	/** The handoff no longer stands: Stop exchange, a revocation or an unlink ended it. */
	ended: boolean;
	people: ReadonlyMap<string, Person>;
	onTalk(): void;
}) {
	return (
		<div className="handoff-bar">
			<span className="handoff-faces" aria-hidden="true">
				<Avatar id={opener.personaId} name={opener.name} size={24} hash={people.get(opener.personaId)?.hash} />
				<Avatar id={teammate.id} name={teammate.name} size={24} hash={teammate.avatarHash} />
			</span>
			<p className="min-w-0 flex-1 text-sm text-ink-3">
				{opener.name} handed this to {teammate.name}.{ended ? " The handoff has ended." : ""}{" "}
				<button type="button" className="link-quiet" onClick={onTalk}>
					Talk to {opener.name} about it ›
				</button>
			</p>
		</div>
	);
}

/** The one thing the person says in a handoff: the answer to what it asked them, written on its card. */
function AnswerField({ thread, name, actionId, onRefused }: { thread: ThreadId; name: string; actionId: string; onRefused(error: unknown): void }) {
	const [note, setNote] = useState("");
	const [sending, setSending] = useState(false);
	const send = () => {
		const said = note.trim();
		if (said === "" || sending) return;
		setSending(true);
		void answerCard(thread, { kind: "human", actionId, status: "done", note: said }).catch((error: unknown) => {
			setSending(false);
			onRefused(error);
		});
	};
	return (
		<div className="handoff-answer">
			<p className="text-xs text-ink-3">Your answer goes to {name} and carries the work on. You can keep talking here after.</p>
			<input
				className="field w-full"
				aria-label={`Answer ${name}`}
				placeholder={`Answer ${name}…`}
				autoComplete="off"
				disabled={sending}
				value={note}
				onChange={(change) => setNote(change.target.value)}
				onKeyDown={(key) => {
					if (key.key !== "Enter") return;
					key.preventDefault();
					send();
				}}
			/>
		</div>
	);
}

/* ------------------------------------------------------------- the edge */

/**
 * The pane's edge: the gutter between it and the conversation, which you
 * drag, the way the team's is. A double-click puts it back to its default
 * width; on the keyboard the arrows step it, and Home and End take it to
 * its narrowest and widest.
 */
function DockEdge({ width, onWidth }: { width: number; onWidth(width: number): void }) {
	const [dragging, setDragging] = useState(false);
	const start = useRef({ x: 0, width });
	useEffect(() => {
		if (!dragging) return;
		document.documentElement.setAttribute("data-resizing", "");
		return () => document.documentElement.removeAttribute("data-resizing");
	}, [dragging]);
	return (
		// biome-ignore lint/a11y/useSemanticElements: a handle that takes focus and arrow keys is ARIA's window splitter, which an <hr> cannot be.
		<div
			role="separator"
			aria-orientation="vertical"
			aria-label="Resize the pane"
			aria-valuemin={DOCK_MIN}
			aria-valuemax={DOCK_MAX}
			aria-valuenow={width}
			tabIndex={0}
			title="Drag to resize"
			className="dock-edge"
			data-dragging={dragging || undefined}
			onPointerDown={(event) => {
				if (event.button !== 0) return;
				event.preventDefault();
				event.currentTarget.setPointerCapture(event.pointerId);
				start.current = { x: event.clientX, width };
				setDragging(true);
			}}
			onPointerMove={(event) => {
				if (!dragging) return;
				onWidth(draggedDock(start.current.width, start.current.x, event.clientX));
			}}
			onPointerUp={() => setDragging(false)}
			onPointerCancel={() => setDragging(false)}
			onDoubleClick={() => onWidth(DOCK_WIDTH)}
			onKeyDown={(event) => {
				if (event.key === "ArrowLeft") onWidth(clampDock(width + DOCK_STEP));
				else if (event.key === "ArrowRight") onWidth(clampDock(width - DOCK_STEP));
				else if (event.key === "Home") onWidth(DOCK_MIN);
				else if (event.key === "End") onWidth(DOCK_MAX);
				else return;
				event.preventDefault();
			}}
		/>
	);
}
