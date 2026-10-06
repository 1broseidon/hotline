import { type CSSProperties, type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Attachment, ThreadId, ThreadSummary } from "../generated/contract";
import { chordKeys } from "../chords";
import { ArrowLeftIcon, ChevronDownIcon, ChevronRightIcon, CloseIcon } from "../icons";
import { carry } from "../serverFiles";
import {
	clampDock,
	DOCK_MAX,
	DOCK_MIN,
	DOCK_STEP,
	DOCK_WIDTH,
	draggedDock,
	groupThreads,
	type LinkEvent,
	openerOf,
	openerWords,
	pairWith,
	powersOf,
	relativeTime,
	rowOpener,
	rowTitle,
	type ThreadDot,
	threadDot,
	withLinks,
} from "../dock";
import { type Person, usePeople } from "../avatars";
import { sideTitle } from "../links";
import { answerCard, dmOf, sameThread, useThread } from "../tape";
import { Avatar } from "../ui/Avatar";
import { Band } from "../ui/Band";
import { Scroll } from "../ui/Scroll";
import { wire, type RosterEntry } from "../wire";
import { Composer } from "./Composer";
import { Transcript, type ThreadRef } from "./Transcript";

/** What the right-hand pane shows: the list, or one thread open in it. */
export type DockState = { open: ThreadRef | null };

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
	const nameOf = useCallback((personaId: string) => roster.find((one) => one.persona.id === personaId)?.persona.name, [roster]);
	const people = usePeople(roster);
	const { rows, list, reload } = useThreadRows(entry);
	/* The row a thread was opened from, so that going back lands on it. */
	const opened = useRef<string | null>(null);
	const at = state.open;

	return (
		<div className="dock-slot" data-overlay={overlay || undefined} style={{ "--dock-width": `${width}px` } as CSSProperties}>
			<DockEdge width={width} onWidth={onWidth} />
			<aside ref={root} className="dock" aria-label="Threads">
				{at === null && <DockBand onClose={onClose} />}
				{at === null ? (
					<ThreadList teammate={teammate} rows={rows} loaded={list !== undefined} nameOf={nameOf} opened={opened} onOpen={(open) => onState({ open })} />
				) : (
					teammate !== null && (
						<ThreadView
							key={`${at.thread.kind}:${at.thread.key}`}
							open={at}
							row={rows.find((row) => sameThread(row.thread, at.thread))}
							teammate={teammate}
							nameOf={nameOf}
							people={people}
							working={entry !== null && workingNow(entry, at)}
							onChanged={reload}
							onOpen={(open) => onState({ open })}
							onBack={() => onState({ open: null })}
							onClose={onClose}
							onOpenTeammate={onOpenTeammate}
						/>
					)
				)}
			</aside>
		</div>
	);
}

/** Whether a turn of this thread is running, as the roster, which hears of it first, says. */
function workingNow(entry: RosterEntry, open: ThreadRef): boolean {
	if (open.thread.kind === "side") return entry.sides?.find((side) => side.sideId === open.thread.key)?.working ?? false;
	if (open.thread.kind === "run") return entry.subagents?.some((run) => run.runId === open.thread.key) ?? false;
	return false;
}

/** The pane's header: what it holds, and the way out. */
function DockBand({ onClose }: { onClose(): void }) {
	return (
		<Band>
			<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Threads</h2>
			<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
				<CloseIcon />
			</button>
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

/* ------------------------------------------------------------ the list */

/**
 * The teammate's threads of every kind: what `thread.list` says, read again
 * whenever a thread starts, ends, parks or changes hands, and brought up to
 * what the links on the conversation say in between.
 */
/** Each teammate's last list, shown at once on the way back while it is read again. */
const lastLists = new Map<string, ThreadSummary[]>();

function useThreadRows(entry: RosterEntry | null): { rows: ThreadSummary[]; list: ThreadSummary[] | undefined; reload(): void } {
	const personaId = entry?.persona.id;
	const [list, setList] = useState<ThreadSummary[] | undefined>(() => (personaId === undefined ? undefined : lastLists.get(personaId)));
	const [revision, setRevision] = useState(0);
	const { events } = useThread(personaId === undefined ? null : dmOf(personaId));
	const links = useMemo(() => events.filter((event): event is LinkEvent => event.kind === "link"), [events]);
	// A thread that started, parked or ended is a link rewritten; a turn starting or a card landing is the roster's word.
	const signature =
		links.map((link) => `${link.thread}:${link.state}`).join(",") +
		"|" +
		(entry?.sides ?? []).map((side) => `${side.sideId}:${side.working ? 1 : 0}`).join(",") +
		"|" +
		(entry?.subagents ?? []).map((run) => run.runId).join(",") +
		(entry?.waiting === true ? "|waiting" : "");
	useEffect(() => {
		if (personaId === undefined) return;
		let cancelled = false;
		void wire
			.command("thread.list", { personaId })
			.then((next) => {
				lastLists.set(personaId, next);
				if (!cancelled) setList(next);
			})
			.catch(() => !cancelled && setList((was) => was ?? []));
		return () => {
			cancelled = true;
		};
	}, [personaId, signature, revision]);
	// A different teammate starts from what was last read for them, if anything; set during render so the old list never flashes.
	const [shownFor, setShownFor] = useState(personaId);
	if (shownFor !== personaId) {
		setShownFor(personaId);
		setList(personaId === undefined ? undefined : lastLists.get(personaId));
	}
	const rows = useMemo(() => withLinks(list ?? [], links, personaId ?? ""), [list, links, personaId]);
	return { rows, list, reload: useCallback(() => setRevision((one) => one + 1), []) };
}

const STATE_WORDS: Record<ThreadDot, string> = {
	running: "running",
	waiting: "waiting on you",
	parked: "parked",
	closed: "closed",
	idle: "open",
};

function ThreadList({
	teammate,
	rows,
	loaded,
	nameOf,
	opened,
	onOpen,
}: {
	teammate: { id: string; name: string; avatarHash?: string | undefined } | null;
	rows: ThreadSummary[];
	loaded: boolean;
	nameOf(personaId: string): string | undefined;
	/** The row a thread was last opened from. */
	opened: RefObject<string | null>;
	onOpen(open: ThreadRef): void;
}) {
	const root = useRef<HTMLDivElement>(null);
	const idOf = (row: ThreadSummary) => `${row.thread.kind}:${row.thread.key}`;
	const { open, closed } = groupThreads(rows);
	const [folded, setFolded] = useState(() => !closed.some((row) => idOf(row) === opened.current));
	// Relative times age while the list is open.
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), 30_000);
		return () => window.clearInterval(tick);
	}, []);
	// Back from a thread, the row it was opened from has focus again; else the switch.
	useFocusOnOpen(root, ["[data-restore]", ".dock .control"], loaded || teammate === null);

	const row = (one: ThreadSummary) => {
		const dot = threadDot(one);
		const title = rowTitle(one, teammate?.id ?? "", nameOf);
		const said = one.state === "closed" ? (one.outcome ?? one.preview ?? "") : (one.preview ?? "");
		// A teammate's handoff sits among the thread's own work, marked by who it came from.
		const from = rowOpener(one);
		const line = from === "" ? said : said === "" ? from : `${from} · ${said}`;
		return (
			<button
				key={idOf(one)}
				type="button"
				className="rail-row dock-row"
				data-thread={idOf(one)}
				data-restore={opened.current === idOf(one) || undefined}
				aria-label={`${title}${from === "" ? "" : `, ${from}`}, ${STATE_WORDS[dot]}`}
				onClick={() => {
					opened.current = idOf(one);
					onOpen({ thread: one.thread, title });
				}}
			>
				<span aria-hidden="true" className={`dock-dot ${dot === "running" ? "beat" : ""}`} data-state={dot} />
				<span className="min-w-0 flex-1">
					<span className="block h-[18px] truncate font-medium text-ink">{title}</span>
					<span className="block h-4 truncate text-sm text-ink-3">{line === "" ? STATE_WORDS[dot] : line}</span>
				</span>
				<span className="shrink-0 self-start pt-0.5 text-xs text-ink-3">{relativeTime(one.updatedAt, now)}</span>
			</button>
		);
	};

	return (
		<Scroll>
			<div ref={root} className="px-2 pb-3">
				{teammate === null ? (
					<p className="px-2 py-3 text-sm text-ink-3">Pick a teammate to see their threads.</p>
				) : !loaded ? null : open.length + closed.length === 0 ? (
					<p className="px-2 py-3 text-sm text-ink-3">
						No threads with {teammate.name} yet. Start a side thread from the conversation's More menu.
					</p>
				) : (
					<>
						{open.map(row)}
						{closed.length > 0 && (
							<div className={open.length > 0 ? "mt-2" : ""}>
								<button
									type="button"
									className="control btn-quiet w-full justify-start gap-1.5 px-2 text-sm"
									aria-expanded={!folded}
									onClick={() => setFolded((was) => !was)}
								>
									{folded ? <ChevronRightIcon className="text-ink-3" /> : <ChevronDownIcon className="text-ink-3" />}
									Closed
									<span className="text-ink-3">{closed.length}</span>
								</button>
								{!folded && closed.map(row)}
							</div>
						)}
					</>
				)}
			</div>
		</Scroll>
	);
}

/* ------------------------------------------------------------ one thread */

/** Where a handed-off exchange came from and where its reply goes, for the one who opened it from the line that says so. */
export function HandoffNote({ handoff }: { handoff: NonNullable<ThreadRef["handoff"]> }) {
	return (
		<details open className="mx-4 mb-3 text-sm text-ink-3">
			<summary className="cursor-pointer">Handed off from {handoff.name}</summary>
			<dl className="selectable mt-2 space-y-2 break-words">
				<div><dt className="eyebrow">Sender</dt><dd>{handoff.name} · {handoff.personaId}</dd></div>
				<div><dt className="eyebrow">Request</dt><dd>{handoff.requestId}</dd></div>
				<div>
					<dt className="eyebrow">Reply goes to</dt>
					<dd>{handoff.name} in this originating exchange, even if they have moved on.</dd>
					<dd className="mt-1 font-mono text-xs">{handoff.threadKey}</dd>
				</div>
			</dl>
		</details>
	);
}

/**
 * A thread, open in the pane, whatever its kind: the same conversation view the
 * main chat is, and below it a composer only where it can be spoken in. A work
 * thread is the same teammate in a second conversation, answerable, and a
 * parked one says so: saying something in it brings its agent back. Archive
 * ends it; once closed the page is the thread, read-only, with a Continue where
 * the composer was. A pair, a subagent's run and a call are read, not spoken
 * in: a run is told its task once and reports once.
 */
function ThreadView({
	open,
	row,
	teammate,
	nameOf,
	people,
	working,
	onChanged,
	onOpen,
	onBack,
	onClose,
	onOpenTeammate,
}: {
	open: ThreadRef;
	/** What the list knows of it: its state and its title. */
	row: ThreadSummary | undefined;
	teammate: { id: string; name: string; avatarHash?: string | undefined };
	nameOf(personaId: string): string | undefined;
	people: ReadonlyMap<string, Person>;
	/** A turn of this thread is running, as the roster says. */
	working: boolean;
	/** The thread was archived or continued: the list reads itself again. */
	onChanged(): void;
	onOpen(open: ThreadRef): void;
	onBack(): void;
	onClose(): void;
	onOpenTeammate(personaId: string): void;
}) {
	const id = open.thread;
	const { events, streaming, more, earlier, prompt, cancel, close, resume } = useThread(id);
	const state = row?.state ?? "live";
	// A teammate's handoff is the two teammates' work, read along and steered through whoever handed it over.
	const opener = openerOf(row);
	const powers = powersOf(id.kind, state, opener !== undefined);
	const closed = state === "closed";
	const parked = state === "parked";
	const turning = (working || row?.working === true) && !closed;
	const other = id.kind === "pair" ? pairWith(id.key, teammate.id) : undefined;
	const withName = open.withName ?? (other === undefined ? undefined : nameOf(other)) ?? "a teammate";
	const title = row !== undefined ? rowTitle(row, teammate.id, nameOf) : (id.kind === "side" ? sideTitle(open.title) : (open.title ?? (id.kind === "pair" ? `With ${withName}` : "Thread")));
	const from = row === undefined ? "" : openerWords(row);
	// The link is the thread's own line in the conversation that holds it; here the thread is the page.
	const lines = useMemo(() => events.filter((event) => event.kind !== "link"), [events]);
	const [refused, setRefused] = useState<string | null>(null);
	const root = useRef<HTMLDivElement>(null);
	// The composer is where this page is for; a closed one has Continue.
	useFocusOnOpen(root, "textarea, [data-autofocus]");

	// A pair is read where it is open: the other side's lines are then seen.
	useEffect(() => {
		if (id.kind !== "pair") return;
		const eventIds = events.filter((event) => event.kind === "user" || event.kind === "agent").map((event) => event.id);
		if (eventIds.length > 0) void wire.command("peers.mark_read", { key: id.key, eventIds });
	}, [id.kind, id.key, events]);

	const refuse = (error: unknown) => setRefused(error instanceof Error ? error.message : String(error));
	const send = (text: string, attachments: Attachment[]) => {
		setRefused(null);
		void carry(attachments).then((carried) => prompt(text, carried)).catch(refuse);
	};
	const archive = () => {
		setRefused(null);
		void close().then(onChanged, refuse);
	};
	const resumeIt = () => {
		setRefused(null);
		void resume().then(onChanged, refuse);
	};
	// A pair's chair is whichever of the key's two ids this teammate is: theirs sit on the right. In a handoff, this teammate's.
	const speakers =
		id.kind === "pair"
			? { me: teammate.name, them: withName, mine: (id.key.split("~")[0] === teammate.id ? "user" : "agent") as "user" | "agent" }
			: opener !== undefined
				? { me: teammate.name, them: opener.name, mine: "agent" as const }
				: undefined;
	// What a handoff asks the person, the oldest first: the one thing they say in it.
	const asked = opener === undefined ? undefined : lines.find((event): event is Extract<typeof event, { kind: "human_action" }> => event.kind === "human_action" && event.status === "pending");

	return (
		<div ref={root} className="dock-page" role="region" aria-label={`Thread with ${teammate.name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back to threads" aria-label="Back to threads" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<h2 className="flex min-w-0 flex-1 items-center gap-2 truncate pl-1 text-lg font-semibold">
					{turning && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
					<span className="truncate">{title}</span>
				</h2>
				{powers.resume && (
					<button type="button" className="control btn-quiet px-2 text-sm" data-autofocus title="Bring this thread back, with what it remembers" onClick={resumeIt}>
						Continue
					</button>
				)}
				{powers.close && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Archive this thread" onClick={archive}>
						Archive
					</button>
				)}
				{powers.stop && turning && (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Stop this turn" onClick={() => void cancel().catch(refuse)}>
						Stop
					</button>
				)}
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			{from !== "" && opener === undefined && <p className="dock-note selectable">{from}</p>}
			{open.handoff !== undefined && <HandoffNote handoff={open.handoff} />}
			<div className="relative flex min-h-0 flex-1 flex-col">
				<Transcript
					key={`${id.kind}:${id.key}`}
					personaId={teammate.id}
					thread={id}
					name={teammate.name}
					avatarHash={teammate.avatarHash}
					people={people}
					events={lines}
					streaming={streaming}
					live={turning}
					focus={null}
					more={more}
					onEarlier={earlier}
					onOpenThread={onOpen}
					{...(speakers !== undefined ? { speakers } : {})}
				/>
			</div>
			{refused !== null && <p className="dock-note selectable" style={{ color: "var(--warn)" }}>{refused}</p>}
			{opener !== undefined ? (
				asked !== undefined ? (
					<AnswerField thread={id} name={teammate.name} actionId={asked.actionId} onRefused={refuse} />
				) : (
					<HandoffBar opener={opener} teammate={teammate} people={people} onTalk={() => onOpenTeammate(opener.personaId)} />
				)
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
						name={teammate.name}
						state={turning ? "thinking" : "ready"}
						replyQuote={null}
						onSend={send}
						onCancel={() => void cancel().catch(() => undefined)}
						onClearReply={() => undefined}
					/>
				</>
			) : (
				closed && row?.outcome !== undefined && row.outcome !== "" && <p className="dock-note selectable">{row.outcome}</p>
			)}
		</div>
	);
}

/**
 * Where the composer is in a thread a teammate handed over: whose work it is,
 * and the way to the conversation where it is steered.
 */
function HandoffBar({
	opener,
	teammate,
	people,
	onTalk,
}: {
	opener: { personaId: string; name: string };
	teammate: { id: string; name: string; avatarHash?: string | undefined };
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
				{opener.name} handed this to {teammate.name}. You can read along.{" "}
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
			<p className="text-xs text-ink-3">Your answer goes to {name}. The thread goes back to read-only after.</p>
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
