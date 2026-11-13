import { type RefObject, type SyntheticEvent, useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { onDrawings } from "../Mermaid";
import { FIRST, LIMIT, REACH, SLACK, type Heights, forget, heightOf, heightsOf, rowsIn, settleGuess } from "./rowWindow";

/**
 * The rows in the page: from one row through another, each named by its id
 * so the same rows stay drawn when lines land above or below them. `to` is
 * null while the window reaches the last row, so a line that arrives there
 * is drawn without being asked for. `kept` are rows drawn wherever the view
 * is, and `chosen` the rows a selection runs across.
 */
type Drawn = { from: string; to: string | null; kept: readonly string[]; chosen: { from: string; to: string } | null };

/** Passes a render may take to bring its window to rest before the next frame takes over. */
const PASSES = 6;
/** Rows measured off to the side per pass. */
const PROBED = 12;
/** Rows the person pressed or typed in that stay in the page, the newest. */
const TOUCHED = 32;

/** A row on screen: where its top stood in the view, and where the scroll was then. */
type Seen = { row: HTMLElement; top: number; scroll: number };

/** The row of the column that holds a node, by its id. */
function rowHolding(column: Element, node: Node | null): string | undefined {
	let at = node;
	while (at !== null && at.parentNode !== column) at = at.parentNode;
	const row = at as HTMLElement | null;
	return row?.classList?.contains("tape-row") ? row.dataset["eventId"] : undefined;
}

/** The first row in view: what the eye is on while rows change above it. */
function onScreen(el: HTMLElement): HTMLElement | null {
	const view = el.getBoundingClientRect();
	for (const row of el.querySelectorAll<HTMLElement>(".tape-column > .tape-row")) {
		const box = row.getBoundingClientRect();
		if (box.bottom > view.top) return box.top < view.bottom ? row : null;
	}
	return null;
}

/** The rows a selection runs across, first and last, or null when nothing in the column is selected. */
function chosenRows(column: Element, ids: readonly string[]): { from: string; to: string } | null {
	const selection = document.getSelection();
	if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return null;
	const range = selection.getRangeAt(0);
	if (!range.intersectsNode(column)) return null;
	// An end outside every row (the whole window selected) takes all the rows that side.
	return { from: rowHolding(column, range.startContainer) ?? ids[0]!, to: rowHolding(column, range.endContainer) ?? ids[ids.length - 1]! };
}

/**
 * Keeps only the rows near the view in the page.
 *
 * A gap as tall as the rest stands in for each run of the others. A row
 * drawn says how tall it is and keeps that height when it leaves, so a row
 * coming back moves nothing. One never drawn stands at a guess until it is
 * measured, which happens off to the side, a few rows at a time, nearest the
 * view first, so its height is known before anyone scrolls to it. A row
 * holding something that has not found its size yet (`data-unsized`: a
 * diagram waiting to be drawn) is not measured until it has, since it is
 * about to be another height: off to the side it waits there. Whenever a
 * height learned changes a gap above the view, the row on screen is put
 * back where it was unless the engine's own anchoring already kept it there.
 *
 * Rows also stay in the page while a selection runs across them, while one
 * holds the keyboard's focus, once the person has pressed or typed in one
 * (what they opened stays open), and when `jump` or `anchor` names one.
 */
export function useRowWindow({
	scroller,
	pinned,
	pinning,
	flying,
	anchor,
	thread,
	ids,
	arrived,
	jump,
}: {
	scroller: RefObject<HTMLDivElement | null>;
	/** Following the conversation: the view is its foot. */
	pinned: RefObject<boolean>;
	/** Set when this writes the scroll, so the scrollbar does not show for it. */
	pinning: RefObject<boolean>;
	/** A flight is on its way (transcript/hooks.ts): nothing on screen is being read, so nothing is held in place. */
	flying: RefObject<(() => void) | null>;
	/** The row a page of older lines is measured against while it is on its way. */
	anchor: RefObject<{ row: HTMLElement | null } | null>;
	/** Which thread this is: its rows' heights are remembered under it. */
	thread: string;
	ids: readonly string[];
	/** Changes when rows may have arrived, which starts the measuring again. */
	arrived: unknown;
	/** The row a jump is going to. */
	jump: string | undefined;
}): {
	heights: Heights;
	/** Whether the row at `index` is in the page. */
	isDrawn(index: number, id: string): boolean;
	/** The rows being measured off to the side. */
	probed: readonly string[];
	/** Puts the window where the view is, drawn before the frame paints. */
	move(): void;
	/** A press or a keystroke in a row: it stays in the page. */
	touch(event: SyntheticEvent): void;
} {
	const heights = useMemo(() => heightsOf(thread), [thread]);
	const [drawn, setDrawn] = useState<Drawn | null>(null);
	const [probed, setProbed] = useState<readonly string[]>([]);
	const [, redraw] = useReducer((count: number) => count + 1, 0);
	/* Counts the times there came to be rows to measure with no line arriving: every height forgotten, a diagram drawn. Each starts the measuring again. */
	const [forgotten, again] = useReducer((count: number) => count + 1, 0);
	/* A row in the page was left unmeasured the last time the window was placed: it waits for its size, and the measuring goes on until it has one. */
	const unread = useRef(false);
	const touched = useRef(new Set<string>());

	// A thread opens on its last lines. After that the window is from one row
	// through another by id; a row that has gone takes the window back to the
	// foot, and the pass after the render puts it where the view is.
	const at = useMemo(() => new Map(ids.map((id, index) => [id, index])), [ids]);
	const fromAt = drawn === null ? undefined : at.get(drawn.from);
	const toEnd = drawn === null || drawn.to === null || fromAt === undefined;
	const from = fromAt ?? Math.max(0, ids.length - FIRST);
	const to = toEnd ? ids.length - 1 : Math.max(from, at.get(drawn.to!) ?? ids.length - 1);
	const kept = useMemo(() => new Set(drawn?.kept), [drawn]);
	const chosenFrom = drawn?.chosen ? at.get(drawn.chosen.from) : undefined;
	const chosenTo = drawn?.chosen ? at.get(drawn.chosen.to) : undefined;
	const isDrawn = (index: number, id: string) =>
		(index >= from && index <= to) || id === jump || kept.has(id) || (chosenFrom !== undefined && chosenTo !== undefined && index >= chosenFrom && index <= chosenTo);

	/* The last render's rows and window, for the scroll to read. */
	const laid = useRef({ ids, at, from, to, toEnd, drawn, probed });
	laid.current = { ids, at, from, to, toEnd, drawn, probed };
	/* A row on screen, where it stood and where the scroll was, before the rows around it changed. */
	const held = useRef<Seen | null>(null);
	/* The same, as of the last time the window was placed: what a row that grows by itself is held against. */
	const eye = useRef<Seen | null>(null);
	/** The first row on screen and where it stands, unless a flight is passing through. */
	const look = useCallback((): Seen | null => {
		const el = scroller.current;
		const seen = !el || flying.current !== null ? null : onScreen(el);
		return !el || seen === null ? null : { row: seen, top: seen.getBoundingClientRect().top, scroll: el.scrollTop };
	}, [scroller, flying]);
	/* What the last put-back could not give back, owed while the scroll is where that one left it (`putBack`). */
	const owed = useRef({ scroll: Number.NaN, by: 0 });
	/**
	 * Puts a row seen on screen back where it stood, and says whether it had
	 * to. Only when nothing else has moved the scroll since it was seen: an
	 * engine that anchors has then done this already, and a pin to the bottom
	 * or a scroll of the person's own is where the view is meant to be.
	 *
	 * A row stands at fractions of a pixel, and WebKit keeps the scroll in
	 * whole ones, so there a put-back leaves the row a fraction from where it
	 * stood. Left at that, a run of them adds up: each height learned above
	 * the view, each diagram drawn there, moved what was being read most of
	 * a pixel, always the same way. So the scroll goes to the nearest pixel
	 * and what that leaves is carried to the next put-back.
	 */
	const putBack = useCallback(
		(was: Seen | null): boolean => {
			const el = scroller.current;
			if (!el || !was?.row.isConnected || el.scrollTop !== was.scroll) return false;
			const moved = was.row.getBoundingClientRect().top - was.top;
			if (moved === 0) return false;
			const want = moved + (owed.current.scroll === was.scroll ? owed.current.by : 0);
			el.scrollTop = was.scroll + want;
			if (el.scrollTop - was.scroll !== want) el.scrollTop = was.scroll + Math.round(want);
			const went = el.scrollTop - was.scroll;
			// At either end of the column the scroll goes less far than asked, and nothing is owed for that.
			owed.current = { scroll: el.scrollTop, by: Math.abs(want - went) < 1 ? want - went : 0 };
			if (went !== 0) pinning.current = true;
			return true;
		},
		[scroller, pinning],
	);
	/* Heights learned since the last render, which its gaps do not have yet. */
	const learned = useRef(false);
	const passes = useRef(0);

	/** Reads the heights of what is drawn, and says what the view asks to have drawn: null when the page has it. */
	const place = (): Drawn | null => {
		const el = scroller.current;
		const column = el?.firstElementChild;
		const { ids, at, from, to, toEnd, drawn } = laid.current;
		if (!el || !column || ids.length === 0) return null;
		// At another width every row wraps elsewhere: what was measured is
		// forgotten, the rows in the page are read again here, and the rest
		// are measured again off to the side.
		if (column.clientWidth !== heights.width) {
			forget(heights, column.clientWidth);
			learned.current = true;
			again();
		}
		// No height is taken while a font is on its way: the words would be
		// measured in the font standing in for it, and wrap where they will
		// not. A row in the page is read once it has arrived, and the rows
		// off to the side wait there for it.
		const settled = document.fonts?.status !== "loading";
		// Nor of a row holding something that has not found its size yet: a
		// diagram waiting to be drawn. In the page it is read once it has; off
		// to the side it waits there, and its drawing wakes this (below).
		const unsized = column.querySelectorAll("[data-unsized]");
		const waits = (row: Element) => {
			for (const part of unsized) if (row.contains(part)) return true;
			return false;
		};
		unread.current = false;
		let first: Element | null = null;
		for (const child of column.children) {
			if (child.classList.contains("tape-row")) {
				first ??= child;
				if (!settled) continue;
				if (waits(child)) {
					unread.current = true;
					continue;
				}
				const id = (child as HTMLElement).dataset["eventId"]!;
				// A row measured for the first time is a height the room the
				// column holds for its rows (Transcript's `.tape-room`) lacks.
				if (!heights.known.has(id)) learned.current = true;
				heights.known.set(id, child.getBoundingClientRect().height);
			} else if (child.classList.contains("tape-gap")) first ??= child;
			else if (child.classList.contains("tape-probe") && settled) {
				for (const row of child.children) {
					if (waits(row)) continue;
					const id = (row as HTMLElement).dataset["probe"]!;
					// Its height as laid out, not as a box on the page: these rows
					// stand far off, where a box's edges are too coarse to subtract.
					const height = Number.parseFloat(window.getComputedStyle(row).height);
					if (heights.known.get(id) === height) continue;
					heights.known.set(id, height);
					learned.current = true;
				}
			}
		}
		if (first === null) return null;
		if (settleGuess(heights)) learned.current = true;
		// Where the window's first row starts and ends, and its last.
		let y = 0;
		const head = { start: 0, end: 0 };
		const foot = { start: 0, end: 0 };
		for (let index = 0; index < ids.length; index++) {
			const start = y;
			y += heightOf(heights, ids[index]!);
			if (index === from) Object.assign(head, { start, end: y });
			if (index === to) Object.assign(foot, { start, end: y });
		}
		// Following the conversation, the view is its foot, wherever the scroll
		// has got to; on a flight down to it, the view is where the flight is.
		const top = pinned.current && flying.current === null ? y - el.clientHeight : el.getBoundingClientRect().top - first.getBoundingClientRect().top;
		const bottom = top + el.clientHeight;
		const last = ids.length - 1;
		// Enough is drawn while the window runs well past the view both ways
		// and its end rows are out of view: a row on screen always has its
		// neighbours in the page, however tall it is.
		const enough = (from === 0 || (head.start <= top - SLACK && head.end <= top)) && (to === last || (foot.end >= bottom + SLACK && foot.start >= bottom));
		const tooMuch = head.start < top - LIMIT || foot.end > bottom + LIMIT;
		const next = enough && !tooMuch ? { from, to } : rowsIn(ids, heights, top - REACH, bottom + REACH);

		// What stays whatever the view, where the window does not already hold it.
		const outside = (id: string | undefined): id is string => {
			const index = id === undefined ? undefined : at.get(id);
			return index !== undefined && (index < next.from || index > next.to);
		};
		for (const id of touched.current) if (!at.has(id)) touched.current.delete(id);
		const keep = [...touched.current, rowHolding(column, document.activeElement), anchor.current?.row?.dataset["eventId"]].filter(outside).sort();
		const span = chosenRows(column, ids);
		const chosen = span !== null && (outside(span.from) || outside(span.to)) ? span : null;

		const same =
			next.from === from &&
			next.to === to &&
			(next.to === last) === toEnd &&
			keep.join("\n") === (drawn?.kept ?? []).join("\n") &&
			chosen?.from === drawn?.chosen?.from &&
			chosen?.to === drawn?.chosen?.to;
		return same ? null : { from: ids[next.from]!, to: next.to === last ? null : ids[next.to]!, kept: keep, chosen };
	};

	/** Puts the window where the view is. `inFrame` on a frame of its own, where the rows must be drawn before it paints. */
	const move = (inFrame: boolean) => {
		const el = scroller.current;
		const next = place();
		eye.current = look();
		if (!el || (next === null && !learned.current)) {
			passes.current = 0;
			return;
		}
		if (!inFrame && ++passes.current > PASSES) {
			requestAnimationFrame(() => moveRef.current(true));
			return;
		}
		learned.current = false;
		held.current = eye.current;
		// While rows are swapped for a gap and back the engine's own scroll
		// anchoring is off (index.css): WebKit lays the column out part-way
		// through the swap and moves the view by what it saw of it, the gap
		// that grew without the rows that left. The row on screen is put back
		// by hand after the render, where it is known what moved. A flight
		// has it off from start to end (transcript/hooks.ts): turning it off
		// and on under a scroll the engine is gliding ends the glide.
		if (flying.current === null) el.dataset["swapping"] = "";
		const draw = () => (next === null ? redraw() : setDrawn(next));
		if (inFrame) flushSync(draw);
		else draw();
	};
	const moveRef = useRef(move);
	moveRef.current = move;

	// After every render: the row that was on screen is where it was, and the
	// window is where the view is. A page landed, a line arrived, a row grew:
	// whatever the render was, the rows near the view are drawn before it paints.
	useLayoutEffect(() => {
		const column = scroller.current?.firstElementChild;
		putBack(held.current);
		held.current = null;
		// The swap is done: the engine anchors the view again.
		if (scroller.current && flying.current === null) delete scroller.current.dataset["swapping"];
		move(false);
		// The rows now in the page are the ones watched for growing.
		const watch = watched.current;
		if (watch === null || !column) return;
		for (const row of watch.rows) {
			if (row.isConnected) continue;
			watch.observer.unobserve(row);
			watch.rows.delete(row);
		}
		for (const row of column.children) {
			if (!row.classList.contains("tape-row") || watch.rows.has(row)) continue;
			watch.observer.observe(row);
			watch.rows.add(row);
		}
	});

	// A row can grow by itself after it is drawn: a picture arrives, a font
	// loads, the window is made narrower. Above the view that pushes down what
	// is being read. An engine that anchors the scroll holds the view through
	// it; WebKitGTK does not, so there the row that was on screen when the
	// window was last placed is put back where it stood. Each row's height is
	// kept as it changes, so the gap that stands in for it later is as tall as
	// it left.
	const watched = useRef<{ observer: ResizeObserver; rows: Set<Element> } | null>(null);
	useEffect(() => {
		if (typeof ResizeObserver === "undefined") return;
		const anchors = typeof CSS !== "undefined" && CSS.supports?.("overflow-anchor", "auto") === true;
		const observer = new ResizeObserver((entries) => {
			const el = scroller.current;
			for (const { target } of entries) {
				const id = (target as HTMLElement).dataset["eventId"];
				if (id !== undefined && target.isConnected && heights.known.has(id)) heights.known.set(id, target.getBoundingClientRect().height);
			}
			if (anchors || !el || flying.current !== null) return;
			if (putBack(eye.current)) eye.current = look();
		});
		watched.current = { observer, rows: new Set() };
		return () => {
			observer.disconnect();
			watched.current = null;
		};
	}, [scroller, flying, heights, look, putBack]);

	// Rows never drawn are measured off to the side, a few every other frame,
	// those nearest above the window first: the way up a conversation is
	// where a height learned late would move what is being read. It runs while
	// the person scrolls too; waiting for them to stop would leave a long
	// fling running into rows still at the guess. It runs on frames, so a
	// window nobody can see measures nothing.
	// biome-ignore lint/correctness/useExhaustiveDependencies: `arrived` and `forgotten` say there may be rows to measure; the pass reads them from the last render.
	useEffect(() => {
		let frame = 0;
		const pass = () => {
			const { ids, from, to, probed } = laid.current;
			const next: string[] = [];
			for (let index = from - 1; index >= 0 && next.length < PROBED; index--) if (!heights.known.has(ids[index]!)) next.push(ids[index]!);
			for (let index = to + 1; index < ids.length && next.length < PROBED; index++) if (!heights.known.has(ids[index]!)) next.push(ids[index]!);
			// Nothing being measured and nothing to measure: no state is set.
			// This runs after every line that lands, and a state set here,
			// even to what it already is, asks for a second render from
			// inside the first; a burst of lines doing that trips React's
			// guard against an update that never ends.
			if (next.length > 0 || probed.length > 0) {
				// The rows there are to measure are the ones already off to the
				// side: they wait there for a font or for a diagram to be drawn.
				// They are read again, in case what they wait for has come, and
				// no state is set: one set on every pass for as long as a
				// diagram waits would draw the conversation again each time.
				if (next.length === probed.length && next.every((id, at) => id === probed[at])) moveRef.current(true);
				else setProbed(next);
			}
			// Over once nothing is left to measure: no row off to the side, and
			// none in the page still waiting for its size, which may leave the
			// page before it has one and is then measured here.
			if (next.length === 0 && !unread.current) return;
			frame = requestAnimationFrame(() => {
				frame = requestAnimationFrame(pass);
			});
		};
		pass();
		return () => {
			if (frame !== 0) cancelAnimationFrame(frame);
		};
	}, [arrived, heights, forgotten]);

	// A font arrived: the rows held off to the side for it are read now.
	useEffect(() => {
		const fonts = document.fonts as FontFaceSet | undefined;
		if (fonts === undefined) return;
		const onFonts = () => moveRef.current(true);
		fonts.addEventListener("loadingdone", onFonts);
		return () => fonts.removeEventListener("loadingdone", onFonts);
	}, []);

	// A diagram was drawn, or found not to draw: the rows that waited for it
	// are read in the render this asks for, and the measuring goes on. A
	// drawing changes its row's height in a render of the block's own, which
	// lands before this one. So the row on screen is noted here, before
	// either, and put back after both, with the engine's anchoring off
	// between, as when the window moves (`move`, above). Following the
	// conversation, the view is then taken to its foot, where a diagram
	// drawn in view has made the column taller: the transcript's own pin
	// comes a frame later, and for that frame the last line would be out of
	// view. Nobody is scrolling: a diagram is drawn with the window at
	// rest. All of it is over before the window can paint.
	useEffect(
		() =>
			onDrawings(() => {
				const el = scroller.current;
				held.current = look();
				if (el && flying.current === null) el.dataset["swapping"] = "";
				queueMicrotask(() => {
					flushSync(again);
					if (!el || !pinned.current || flying.current !== null) return;
					const was = el.scrollTop;
					el.scrollTop = el.scrollHeight;
					if (el.scrollTop !== was) pinning.current = true;
				});
			}),
		[scroller, pinned, pinning, flying, look],
	);

	// A selection made or stretched without a scroll (shift and a press, the
	// whole window selected) still has to find every row it covers in the page.
	useEffect(() => {
		let frame = 0;
		const onSelection = () => {
			if (frame !== 0) return;
			frame = requestAnimationFrame(() => {
				frame = 0;
				moveRef.current(true);
			});
		};
		document.addEventListener("selectionchange", onSelection);
		return () => {
			document.removeEventListener("selectionchange", onSelection);
			if (frame !== 0) cancelAnimationFrame(frame);
		};
	}, []);

	const touch = useCallback((event: SyntheticEvent) => {
		const id = rowHolding(event.currentTarget as Element, event.target as Node);
		if (id === undefined) return;
		// Pressed again, it is the newest.
		touched.current.delete(id);
		touched.current.add(id);
		if (touched.current.size > TOUCHED) touched.current.delete(touched.current.values().next().value as string);
	}, []);
	const moveInFrame = useCallback(() => moveRef.current(true), []);

	return { heights, isDrawn, probed, move: moveInFrame, touch };
}
