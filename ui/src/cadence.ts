// Shared with Toad Mobile: the phone copies this file with `npm run sync:contract`.
/**
 * The rhythm bubbles land in: a reading beat between one and the next, the
 * way a person sends a few messages rather than one burst. A display rule,
 * not a wire rule — the tape is written the moment the words exist.
 */

/** A bubble as the beat sees it: which one, what it says, when it was written. */
export type Bubble = { id: string; text: string; ts: number };

/** What has landed so far, and when the last one did. */
export type Cadence = { shown: ReadonlySet<string>; lastAt: number; lastText: string };

export const REST: Cadence = { shown: new Set(), lastAt: 0, lastText: "" };

/** A bubble older than this was there before you looked; it is not paced. */
export const PACE_WITHIN_MS = 10_000;

/** How long a bubble holds the floor before the next may land: long enough to read it. */
export function beatMs(text: string): number {
	const chars = Array.from(text).length;
	return Math.min(1500, Math.max(600, 500 + chars * 10));
}

/**
 * One step of the beat: the bubbles that are on screen after it, the ones
 * still waiting, and when to step again. At most one new bubble lands per
 * step, and only once the last one has had its beat; a bubble written
 * before you were looking lands at once.
 */
export function step(
	cadence: Cadence,
	bubbles: readonly Bubble[],
	now: number,
): { cadence: Cadence; hidden: string[]; dueIn: number | null } {
	const shown = new Set(cadence.shown);
	let lastAt = cadence.lastAt;
	let lastText = cadence.lastText;
	const hidden: string[] = [];
	let dueIn: number | null = null;
	let landed = false;
	for (const bubble of bubbles) {
		if (shown.has(bubble.id)) continue;
		if (now - bubble.ts > PACE_WITHIN_MS) {
			shown.add(bubble.id);
			continue;
		}
		const readyAt = lastAt === 0 ? now : lastAt + beatMs(lastText);
		if (!landed && now >= readyAt) {
			shown.add(bubble.id);
			lastAt = now;
			lastText = bubble.text;
			landed = true;
			continue;
		}
		hidden.push(bubble.id);
		if (dueIn === null) dueIn = Math.max(1, (landed ? now + beatMs(lastText) : readyAt) - now);
	}
	return { cadence: { shown, lastAt, lastText }, hidden, dueIn };
}
