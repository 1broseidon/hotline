import { useEffect, useMemo, useReducer, useRef, useState, type RefObject } from "react";
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
 * they happen. Resting while the turn waits on its subagents is not a kind
 * of work either, so the turn going quiet, or back to work, shows at once.
 */
const DWELL_MS = 900;

export function useSteady(activity: Activity | null): Activity | null {
	const shown = useRef<{ activity: Activity | null; at: number }>({ activity, at: Date.now() });
	const [, wake] = useReducer((n: number) => n + 1, 0);
	const now = Date.now();
	const was = shown.current;
	const work = (one: Activity | null) => one !== null && one.phase !== "blocked" && one.phase !== "writing" && one.phase !== "rest";
	const changing = activity?.phase !== was.activity?.phase;
	const hold = changing && work(activity) && work(was.activity) && now - was.at < DWELL_MS;
	if (changing && !hold) shown.current = { activity, at: now };
	// The same kind in new words is the words now, without starting its dwell again.
	else if (!changing) shown.current = { activity, at: was.at };
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
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new teammate is what starts it again.
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
 * nothing was said to hang up on. A turn that stops to wait on its subagents
 * after a reply hangs up the same way, keeping the words it rests with, and
 * then rests. Worked out during render rather than after it, so the mark is
 * never gone for a frame between writing and landing.
 */
export function useLanding(personaId: string, activity: Activity | null): Activity | null {
	const last = useRef<ActivityPhase | null>(null);
	const [landing, setLanding] = useState(false);
	// Resting while it waits is the far side of a hang-up, as the turn's end is.
	const phase = activity === null || activity.phase === "rest" ? null : activity.phase;
	const ended = phase === null && (landing || last.current === "writing");
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new teammate is what starts it again.
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
	return ended ? { ...LANDED, word: activity?.word ?? "" } : activity;
}

/**
 * The agent's bubbles land to a beat. The ids still waiting their turn, and a
 * re-render when the next is due. Everything else on the tape shows at once.
 */
export function useCadence(personaId: string, blocks: Block[]): ReadonlySet<string> {
	const cadence = useRef<Cadence>(REST);
	const [, wake] = useReducer((n: number) => n + 1, 0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new teammate is what starts it again.
	useEffect(() => {
		cadence.current = REST;
	}, [personaId]);
	const bubbles = useMemo(() => {
		const bubbles: Bubble[] = [];
		for (const block of blocks) {
			if (block.kind === "event" && block.event.kind === "agent") {
				bubbles.push({ id: block.event.id, text: block.event.text, ts: block.event.ts, live: block.live === true });
			}
		}
		return bubbles;
	}, [blocks]);
	const next = step(cadence.current, bubbles, Date.now());
	cadence.current = next.cadence;
	const dueIn = next.dueIn;
	// biome-ignore lint/correctness/useExhaustiveDependencies: a bubble shown is what sets the next wake.
	useEffect(() => {
		if (dueIn === null) return;
		const timer = window.setTimeout(wake, dueIn);
		return () => window.clearTimeout(timer);
	}, [dueIn, next.hidden.length]);
	const hidden = useRef<ReadonlySet<string>>(new Set());
	if (hidden.current.size !== next.hidden.length || next.hidden.some((id) => !hidden.current.has(id))) hidden.current = new Set(next.hidden);
	return hidden.current;
}

/** How long a flight may take to arrive before it is taken to have. */
const FLIGHT_MS = 1_000;
/** What the person does that takes a scroll back from a flight. */
const TAKEN_BY = ["wheel", "touchmove", "keydown"] as const;

/**
 * A scroll the transcript makes to somewhere that may be far: a jump to a
 * row, or back down to the latest line. `go` starts it and `arrive` puts
 * the view exactly there when the scroll ends.
 *
 * Rows on the way may never have been drawn, and arrive at heights nobody
 * knew, so the place the scroll set out for is not always where it should
 * end; and while it is on its way nothing on screen is being read. So for
 * as long as `flying` holds a flight nothing holds the view (useRowWindow)
 * and nothing pins it to the bottom (Transcript), either of which would
 * write the scroll and end the flight where it stood; and the engine's own
 * scroll anchoring is off (`data-swapping`, index.css), which would move the
 * view by the rows swapped under it on the way. One flight at a time:
 * a new one ends the last where it is, and so does a scroll of the person's
 * own. Returns what ends it early.
 */
export function fly(root: HTMLElement, flying: RefObject<(() => void) | null>, go: () => void, arrive: () => void): () => void {
	flying.current?.();
	const land = (arrived: boolean) => {
		if (flying.current !== cancel) return;
		flying.current = null;
		delete root.dataset["swapping"];
		window.clearTimeout(late);
		root.removeEventListener("scrollend", onEnd);
		for (const kind of TAKEN_BY) root.removeEventListener(kind, cancel);
		if (arrived) arrive();
	};
	const onEnd = () => land(true);
	const cancel = () => land(false);
	flying.current = cancel;
	root.dataset["swapping"] = "";
	root.addEventListener("scrollend", onEnd);
	for (const kind of TAKEN_BY) root.addEventListener(kind, cancel, { passive: true });
	// A scroll that is already where it is going moves nothing, and nothing ends.
	const late = window.setTimeout(onEnd, FLIGHT_MS);
	go();
	return cancel;
}

/**
 * A search hit or a reply's quote: unpin, bring the row to the middle, and
 * light it briefly. The tape arrives after the jump is asked for when
 * Everywhere opens another teammate, so this waits until the fold contains
 * the id. `found` going true is the retry; a later append does not change
 * `found` and so does not jump. The row is in the page wherever the view is:
 * the transcript keeps a jump's row drawn.
 */
export function useScrollToEvent(
	scroller: RefObject<HTMLDivElement | null>,
	pinned: RefObject<boolean>,
	flying: RefObject<(() => void) | null>,
	/** Has the transcript take in where the scroll is, once the jump has arrived. */
	read: RefObject<() => void>,
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
			// A file message inside a run of them is found by the run's row.
			root?.querySelector<HTMLElement>(`[data-members~="${CSS.escape(focus.eventId)}"]`) ??
			root?.querySelector<HTMLElement>(`[data-step-id="${CSS.escape(focus.eventId)}"]`)?.closest<HTMLElement>("[data-event-id]");
		if (!root || !row) return;
		pinned.current = false;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
		const cancel = fly(
			root,
			flying,
			() => row.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" }),
			() => {
				// In the middle already, this moves nothing.
				if (row.isConnected) row.scrollIntoView({ block: "center" });
				read.current();
			},
		);
		row.classList.add("row-lit");
		const timer = window.setTimeout(() => row.classList.remove("row-lit"), 1_800);
		return () => {
			window.clearTimeout(timer);
			cancel();
		};
	}, [focus, found, scroller, pinned, flying, read]);
}
