import { type CSSProperties, type RefObject, useCallback, useEffect, useRef, useState } from "react";
import type { Attachment, RunningSide, SideThreadSummary, TranscriptEvent } from "../generated/contract";
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
	groupSides,
	relativeTime,
	type SideState,
	sideState,
} from "../dock";
import { useSide } from "../tape";
import { Band } from "../ui/Band";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { Composer } from "./Composer";
import { FollowWindow, SidePieceRows, sidePieces } from "./Work";

/** What the right-hand pane shows: the list, or one side thread open in it. */
export type DockState = { side: { sideId: string; title: string } | null };

/**
 * The right-hand pane: this teammate's side threads as a list, like the
 * team on the left. A row opens its thread in
 * the pane with a way back to the list; the pane's edge is dragged to size
 * it, and in a window too narrow for it beside the conversation it lies over
 * the conversation instead (`overlay`).
 */
export function Dock({
	state,
	onState,
	onClose,
	teammate,
	sides,
	width,
	onWidth,
	overlay,
}: {
	state: DockState;
	onState(next: DockState): void;
	onClose(): void;
	/** Whose side threads the Threads view lists; none when nobody is open. */
	teammate: { id: string; name: string } | null;
	/** That teammate's side threads that are live now, from the roster. */
	sides: RunningSide[];
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

	const { list, reload } = useSideList(teammate?.id, sides);
	/* The row a thread was opened from, so that going back lands on it. */
	const opened = useRef<string | null>(null);
	const atList = state.side === null;

	return (
		<div className="dock-slot" data-overlay={overlay || undefined} style={{ "--dock-width": `${width}px` } as CSSProperties}>
			<DockEdge width={width} onWidth={onWidth} />
			<aside ref={root} className="dock" aria-label="Side threads">
				{atList && <DockBand onClose={onClose} />}
				{state.side === null ? (
					<SideList teammate={teammate} sides={sides} list={list} opened={opened} onOpen={(side) => onState({ side })} />
				) : (
					<SideThread
						key={state.side.sideId}
						side={state.side}
						name={teammate?.name ?? "The teammate"}
						working={sides.find((one) => one.sideId === state.side?.sideId)?.working ?? false}
						onChanged={reload}
						onBack={() => onState({ side: null })}
						onClose={onClose}
					/>
				)}
			</aside>
		</div>
	);
}

/** The pane's header: what it holds, and the way out. */
function DockBand({ onClose }: { onClose(): void }) {
	return (
		<Band>
			<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Side threads</h2>
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

/** The teammate's side threads, read again whenever a live one starts, ends or changes hands. */
function useSideList(personaId: string | undefined, sides: RunningSide[]): { list: SideThreadSummary[] | undefined; reload(): void } {
	const [list, setList] = useState<SideThreadSummary[] | undefined>(undefined);
	const [revision, setRevision] = useState(0);
	const signature = sides.map((side) => `${side.sideId}:${side.working ? 1 : 0}`).join(",");
	useEffect(() => {
		if (personaId === undefined) return;
		let cancelled = false;
		void wire
			.command("side.list", { personaId })
			.then((next) => !cancelled && setList(next))
			.catch(() => !cancelled && setList((was) => was ?? []));
		return () => {
			cancelled = true;
		};
	}, [personaId, signature, revision]);
	useEffect(() => setList(undefined), [personaId]);
	return { list, reload: useCallback(() => setRevision((one) => one + 1), []) };
}

const STATE_WORDS: Record<SideState, string> = {
	running: "running",
	waiting: "waiting on you",
	parked: "parked",
	archived: "archived",
	idle: "open",
};

function SideList({
	teammate,
	sides,
	list,
	opened,
	onOpen,
}: {
	teammate: { id: string; name: string } | null;
	sides: RunningSide[];
	list: SideThreadSummary[] | undefined;
	/** The row a thread was last opened from. */
	opened: RefObject<string | null>;
	onOpen(side: { sideId: string; title: string }): void;
}) {
	const root = useRef<HTMLDivElement>(null);
	const [folded, setFolded] = useState(() => !list?.some((side) => side.sideId === opened.current && side.status === "archived"));
	// Relative times age while the list is open.
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), 30_000);
		return () => window.clearInterval(tick);
	}, []);
	// Back from a thread, the row it was opened from has focus again; else the switch.
	useFocusOnOpen(root, ["[data-restore]", ".dock .control"], list !== undefined || teammate === null);
	const { open, archived } = groupSides(list ?? []);

	const row = (side: SideThreadSummary) => {
		const state = sideState({ ...side, working: side.working || sides.some((one) => one.sideId === side.sideId && one.working) });
		const line = side.status === "archived" ? (side.result ?? side.preview ?? "") : (side.preview ?? "");
		return (
			<button
				key={side.sideId}
				type="button"
				className="rail-row dock-row"
				data-side-id={side.sideId}
				data-restore={opened.current === side.sideId || undefined}
				aria-label={`${side.title}, ${STATE_WORDS[state]}`}
				onClick={() => {
					opened.current = side.sideId;
					onOpen({ sideId: side.sideId, title: side.title });
				}}
			>
				<span aria-hidden="true" className={`dock-dot ${state === "running" ? "beat" : ""}`} data-state={state} />
				<span className="min-w-0 flex-1">
					<span className="block h-[18px] truncate font-medium text-ink">{side.title}</span>
					<span className="block h-4 truncate text-sm text-ink-3">{line === "" ? STATE_WORDS[state] : line}</span>
				</span>
				<span className="shrink-0 self-start pt-0.5 text-xs text-ink-3">{relativeTime(side.status === "archived" ? (side.archivedAt ?? side.lastAt) : side.lastAt, now)}</span>
			</button>
		);
	};

	return (
		<Scroll>
			<div ref={root} className="px-2 pb-3">
				{teammate === null ? (
					<p className="px-2 py-3 text-sm text-ink-3">Pick a teammate to see their side threads.</p>
				) : list === undefined ? null : list.length === 0 ? (
					<p className="px-2 py-3 text-sm text-ink-3">
						No side threads with {teammate.name} yet. Type <span className="font-mono">/side</span> and a task in the composer to start one.
					</p>
				) : (
					<>
						{open.map(row)}
						{archived.length > 0 && (
							<div className={open.length > 0 ? "mt-2" : ""}>
								<button
									type="button"
									className="control btn-quiet w-full justify-start gap-1.5 px-2 text-sm"
									aria-expanded={!folded}
									onClick={() => setFolded((was) => !was)}
								>
									{folded ? <ChevronRightIcon className="text-ink-3" /> : <ChevronDownIcon className="text-ink-3" />}
									Archived
									<span className="text-ink-3">{archived.length}</span>
								</button>
								{!folded && archived.map(row)}
							</div>
						)}
					</>
				)}
			</div>
		</Scroll>
	);
}

/* --------------------------------------------------------- a side thread */

/**
 * A side thread, open in the pane: the same teammate in a second
 * conversation. What the person said, what the teammate said and did, and
 * below it a composer, because unlike a run this one is answerable. A parked
 * thread is answerable too, and says so: saying something in it brings its
 * agent back. Archive ends it; once archived the page is the thread,
 * read-only, with a Continue where the composer was.
 */
function SideThread({
	side,
	name,
	working,
	onChanged,
	onBack,
	onClose,
}: {
	side: { sideId: string; title: string };
	name: string;
	/** A turn of this thread is running, as the roster says. */
	working: boolean;
	/** The thread was archived or continued: the list reads itself again. */
	onChanged(): void;
	onBack(): void;
	onClose(): void;
}) {
	const { events, streaming, loaded } = useSide(side.sideId);
	const marker = events.find((event): event is Extract<TranscriptEvent, { kind: "side" }> => event.kind === "side");
	const archived = marker?.status === "archived";
	const parked = marker?.status === "parked";
	const title = marker?.title ?? side.title;
	const pieces = sidePieces(events, streaming);
	const [refused, setRefused] = useState<string | null>(null);
	const root = useRef<HTMLDivElement>(null);
	// The composer is where this page is for; an archived one has Continue.
	useFocusOnOpen(root, "textarea, [data-autofocus]");

	const refuse = (error: unknown) => setRefused(error instanceof Error ? error.message : String(error));
	const send = (text: string, attachments: Attachment[]) => {
		setRefused(null);
		void carry(attachments)
			.then((carried) => wire.command("side.prompt", { sideId: side.sideId, text, ...(carried.length > 0 ? { attachments: carried } : {}) }))
			.catch(refuse);
	};
	const archive = () => {
		setRefused(null);
		void wire.command("side.archive", { sideId: side.sideId }).then(onChanged, refuse);
	};
	const resume = () => {
		setRefused(null);
		void wire.command("side.continue", { sideId: side.sideId }).then(onChanged, refuse);
	};

	return (
		<div ref={root} className="dock-page" role="region" aria-label={`Side thread with ${name}: ${title}`}>
			<Band>
				<button type="button" className="control btn-icon -ml-1" title="Back to threads" aria-label="Back to threads" onClick={onBack}>
					<ArrowLeftIcon />
				</button>
				<h2 className="flex min-w-0 flex-1 items-center gap-2 truncate pl-1 text-lg font-semibold">
					{working && !archived && <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
					<span className="truncate">{title}</span>
				</h2>
				{archived ? (
					<button type="button" className="control btn-quiet px-2 text-sm" data-autofocus title="Bring this side thread back, with what it remembers" onClick={resume}>
						Continue
					</button>
				) : (
					<button type="button" className="control btn-quiet px-2 text-sm" title="Archive this side thread" onClick={archive}>
						Archive
					</button>
				)}
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={onClose}>
					<CloseIcon />
				</button>
			</Band>
			<p className="instrument px-4 pb-1 pt-0.5" role="status">
				{archived ? "archived" : working ? "working" : parked ? "parked" : "side thread"}
			</p>
			<FollowWindow following={side.sideId} count={events.length + streaming.length}>
				<SidePieceRows sideId={side.sideId} pieces={pieces} settled={!working || archived} />
				{pieces.length === 0 && !archived && <p className="work-empty">{loaded ? `${name} is getting started.` : ""}</p>}
			</FollowWindow>
			{archived ? (
				<>
					{marker?.result !== undefined && <p className="work-notice selectable px-2" style={{ color: "var(--ink-2)" }}>{marker.result}</p>}
					{refused !== null && <p className="work-notice selectable px-2">{refused}</p>}
				</>
			) : (
				<>
					{parked && (
						<p className="work-notice selectable px-2" style={{ color: "var(--ink-3)" }}>
							Parked: no agent is running. Saying something here picks it back up where it left off.
						</p>
					)}
					{refused !== null && <p className="work-notice selectable px-2">{refused}</p>}
					<Composer
						embedded
						personaId={side.sideId}
						name={name}
						state={working ? "thinking" : "ready"}
						replyQuote={null}
						onSend={send}
						onCancel={() => void wire.command("side.cancel", { sideId: side.sideId }).catch(() => undefined)}
						onClearReply={() => undefined}
					/>
				</>
			)}
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
