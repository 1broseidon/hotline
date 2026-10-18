import { useEffect, useReducer, useRef, useState, type RefObject } from "react";
import type { TranscriptEvent } from "../../generated/contract";
import { REST, step, type Bubble, type Cadence } from "../../cadence";
import type { Block } from "../../scheduledRuns";
import { type Activity, type ActivityPhase, LANDED, RESTING } from "../../activity";
import { LANDED_MS } from "../../ui/Glyph";

/**
 * Each kind of work stays on the mark for at least DWELL_MS before the next
 * replaces it, so an agent alternating reads and searches a few times a
 * second shows each as a pose and not as a twitch. Waiting on you and
 * writing take over at once: they are the two you need to see the moment
 * they happen.
 */
const DWELL_MS = 900;

export function useSteady(activity: Activity | null): Activity | null {
	const shown = useRef<{ activity: Activity | null; at: number }>({ activity, at: Date.now() });
	const [, wake] = useReducer((n: number) => n + 1, 0);
	const now = Date.now();
	const was = shown.current;
	const work = (one: Activity | null) => one !== null && one.phase !== "blocked" && one.phase !== "writing";
	const changing = activity?.phase !== was.activity?.phase;
	const hold = changing && work(activity) && work(was.activity) && now - was.at < DWELL_MS;
	if (changing && !hold) shown.current = { activity, at: now };
	const dueIn = hold ? DWELL_MS - (now - was.at) : null;
	useEffect(() => {
		if (dueIn === null) return;
		const timer = window.setTimeout(wake, dueIn);
		return () => window.clearTimeout(timer);
	}, [dueIn]);
	return hold ? was.activity : activity;
}

/**
 * The mark wakes and goes back to sleep behind the composer: it rises from
 * behind it when a turn starts (the CSS does that on mount) and, once the
 * turn and any landing are over, it holds RESTING for SLEEP_MS — the handset
 * settles on the cradle, then the toad sinks back down out of sight. A turn
 * that starts again while it is sinking takes the mark back up. Worked out
 * during render, like the landing, so there is no frame without a mark.
 * SLEEP_MS is the settle and the sink in index.css's `.ambient-mark[data-sleeping]`.
 */
const SLEEP_MS = 800;

export function useSleep(personaId: string, activity: Activity | null): { activity: Activity | null; sleeping: boolean } {
	const awake = useRef(false);
	const [sleeping, setSleeping] = useState(false);
	const off = activity === null;
	const going = off && (sleeping || awake.current);
	useEffect(() => {
		awake.current = false;
		setSleeping(false);
	}, [personaId]);
	useEffect(() => {
		if (!off) {
			awake.current = true;
			setSleeping(false);
			return;
		}
		if (!awake.current) return;
		awake.current = false;
		setSleeping(true);
		const timer = window.setTimeout(() => setSleeping(false), SLEEP_MS);
		return () => window.clearTimeout(timer);
	}, [off]);
	return activity !== null ? { activity, sleeping: false } : going ? { activity: RESTING, sleeping: true } : { activity: null, sleeping: false };
}

/**
 * The mark hangs up after the reply lands. A turn that ended while it was
 * writing keeps the mark for LANDED_MS in the `landed` phase, so the voice
 * line can reel back into the handset and drop onto the cradle; any other
 * ending — a cancel mid-tool, a refusal — lets it go at once, because
 * nothing was said to hang up on. Worked out during render rather than after
 * it, so the mark is never gone for a frame between writing and landing.
 */
export function useLanding(personaId: string, activity: Activity | null): Activity | null {
	const last = useRef<ActivityPhase | null>(null);
	const [landing, setLanding] = useState(false);
	const phase = activity?.phase ?? null;
	const ended = phase === null && (landing || last.current === "writing");
	useEffect(() => {
		last.current = null;
		setLanding(false);
	}, [personaId]);
	useEffect(() => {
		if (phase !== null) {
			last.current = phase;
			setLanding(false);
			return;
		}
		if (last.current !== "writing") return;
		last.current = null;
		setLanding(true);
		const timer = window.setTimeout(() => setLanding(false), LANDED_MS);
		return () => window.clearTimeout(timer);
	}, [phase]);
	return activity ?? (ended ? LANDED : null);
}

/**
 * The agent's bubbles land to a beat. The ids still waiting their turn, and a
 * re-render when the next is due. Everything else on the tape shows at once.
 */
export function useCadence(personaId: string, blocks: Block[]): ReadonlySet<string> {
	const cadence = useRef<Cadence>(REST);
	const [, wake] = useReducer((n: number) => n + 1, 0);
	useEffect(() => {
		cadence.current = REST;
	}, [personaId]);
	const bubbles: Bubble[] = [];
	for (const block of blocks) {
		if (block.kind === "event" && block.event.kind === "agent") {
			bubbles.push({ id: block.event.id, text: block.event.text, ts: block.event.ts });
		}
	}
	const next = step(cadence.current, bubbles, Date.now());
	cadence.current = next.cadence;
	const dueIn = next.dueIn;
	useEffect(() => {
		if (dueIn === null) return;
		const timer = window.setTimeout(wake, dueIn);
		return () => window.clearTimeout(timer);
	}, [dueIn, next.hidden.length]);
	return new Set(next.hidden);
}

/**
 * A search hit or a reply's quote: unpin, bring the row to the middle, and
 * light it briefly. The tape arrives after the jump is asked for when
 * Everywhere opens another teammate, so this waits until the fold contains
 * the id. `found` going true is the retry; a later append does not change
 * `found` and so does not jump.
 */
export function useScrollToEvent(
	scroller: RefObject<HTMLDivElement | null>,
	pinned: RefObject<boolean>,
	focus: { eventId: string; at: number } | null,
	events: TranscriptEvent[],
): void {
	const found = focus !== null && events.some((one) => one.id === focus.eventId);
	useEffect(() => {
		if (!focus || !found) return;
		const root = scroller.current;
		// A step lives inside its block, so the block is what is found.
		const row =
			root?.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(focus.eventId)}"]`) ??
			root?.querySelector<HTMLElement>(`[data-step-id="${CSS.escape(focus.eventId)}"]`)?.closest<HTMLElement>("[data-event-id]");
		if (!root || !row) return;
		pinned.current = false;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
		row.classList.add("row-lit");
		const timer = window.setTimeout(() => row.classList.remove("row-lit"), 1_800);
		return () => window.clearTimeout(timer);
	}, [focus, found, scroller, pinned]);
}
