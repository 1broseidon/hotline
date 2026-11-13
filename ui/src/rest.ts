/**
 * The window at rest.
 *
 * Some work holds the window up for longer than a frame and cannot be cut
 * smaller: evaluating mermaid's script, drawing one diagram. Under a scroll
 * or a keystroke that is a stutter the person feels; a moment after they
 * stop it is not. So such work waits here until nothing has stirred for
 * REST_MS.
 *
 * What stirs: a wheel, a touch that moves, a key, a press, and a scroll
 * that is not the code's own (ui/Scroll.tsx says which: a conversation
 * following its last line scrolls on every word that streams in, and that
 * is nobody scrolling).
 */
const REST_MS = 300;

const STIRRED_BY = ["wheel", "touchmove", "keydown", "pointerdown"] as const;

let stirred = Number.NEGATIVE_INFINITY;
let watched: Document | null = null;

/** The person did something just now, or work that held the window up has just ended. */
export function stir(): void {
	stirred = performance.now();
}

/**
 * Runs `work` once nothing has stirred for REST_MS, on a task of its own.
 *
 * A window that was held up looks the same from in here as one at rest:
 * the clock moved and nothing stirred, because what the person did
 * meanwhile is still on its way in. So when the wait is over the window is
 * given one frame, and a task after it, to take in what is queued; a stir
 * in that time starts the wait again. (A window nobody can see draws no
 * frame, and nothing waiting here runs in it.)
 */
export function atRest(work: () => void): void {
	// Watched from the first time anything waits; until then nobody was
	// counting, so the wait starts from now.
	if (watched !== document) {
		watched = document;
		for (const kind of STIRRED_BY) document.addEventListener(kind, stir, { capture: true, passive: true });
		stir();
	}
	const check = () => {
		const left = REST_MS - (performance.now() - stirred);
		if (left > 0) {
			window.setTimeout(check, left);
			return;
		}
		const last = stirred;
		requestAnimationFrame(() => window.setTimeout(() => (stirred === last ? work() : check()), 0));
	};
	window.setTimeout(check, 0);
}
