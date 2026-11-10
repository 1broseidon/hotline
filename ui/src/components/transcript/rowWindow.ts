/**
 * Which rows of a conversation are in the page.
 *
 * A long thread is thousands of rows, and every one in the page costs
 * elements, memory and layout whether or not anyone can see it. So the
 * transcript draws the rows near the view and stands a gap of the same
 * height in for each run of the others. This file is the arithmetic: how
 * tall each row is, and which rows a stretch of the column holds. The
 * transcript does the measuring and the scrolling.
 */

/** How far past each edge of the view rows are drawn, so a scroll arrives at rows, not at a gap. */
export const REACH = 800;
/** How near the view may come to the last drawn row before the window moves on. */
export const SLACK = 400;
/** How far past the view a drawn row may stand before the window lets it go. */
export const LIMIT = 1600;
/** The rows drawn when a thread opens, before anything is measured: its last lines. */
export const FIRST = 30;

/** A row's height before any row of its thread has been drawn. */
const GUESS = 64;

/**
 * How tall a thread's rows are: each one's own height once it has been
 * drawn, and one guess for the rest, the mean of the first rows measured.
 * The guess is taken once: a guess that moved would move every gap with it.
 * `width` is how wide the column was when they were measured: at another
 * width the words wrap elsewhere and none of the heights hold.
 */
export type Heights = { known: Map<string, number>; guess: number; guessed: boolean; width: number };

/** Threads whose heights are remembered, so coming back to one finds its rows where they were. */
const KEPT = 8;
const threads = new Map<string, Heights>();

export function heightsOf(thread: string): Heights {
	const held = threads.get(thread) ?? { known: new Map<string, number>(), guess: GUESS, guessed: false, width: 0 };
	// Shown again, it is the newest.
	threads.delete(thread);
	threads.set(thread, held);
	if (threads.size > KEPT) threads.delete(threads.keys().next().value as string);
	return held;
}

export function heightOf(heights: Heights, id: string): number {
	return heights.known.get(id) ?? heights.guess;
}

/**
 * Forgets every height: the column is another width, and each row is to be
 * measured again. The guess stays, as the nearest thing to a height the
 * rows have until then.
 */
export function forget(heights: Heights, width: number): void {
	heights.known.clear();
	heights.width = width;
}

/** Takes the guess from what has been measured, the first time anything has. Says whether it did: every gap is then another height. */
export function settleGuess(heights: Heights): boolean {
	if (heights.guessed || heights.known.size === 0) return false;
	let sum = 0;
	for (const height of heights.known.values()) sum += height;
	heights.guess = sum / heights.known.size;
	heights.guessed = true;
	return true;
}

/**
 * The rows to draw for the stretch from `top` to `bottom`, measured down
 * from the first row's top: every row that stands in it, and one more at
 * each end. The one more is what follows a row taller than the stretch, so
 * whatever row is on screen has its neighbours in the page, for the
 * keyboard and a screen reader to move to.
 */
export function rowsIn(ids: readonly string[], heights: Heights, top: number, bottom: number): { from: number; to: number } {
	let from = -1;
	let to = -1;
	let y = 0;
	for (let index = 0; index < ids.length; index++) {
		const end = y + heightOf(heights, ids[index]!);
		if (from === -1 && end >= top) from = index;
		if (y <= bottom) to = index;
		y = end;
	}
	// A stretch past the last row holds the last row.
	if (from === -1) from = ids.length - 1;
	return { from: Math.max(0, from - 1), to: Math.min(ids.length - 1, Math.max(from, to) + 1) };
}
