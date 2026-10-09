import { useEffect, useRef } from "react";
import { shownState } from "../activity";
import type { Call, CallPhase } from "../voice/call";
import type { RosterEntry } from "../wire";

/*
 * What runs round the mark on a call, as the phone draws it: bars that
 * follow the reply's playing audio while it speaks, or an arc while the
 * teammate is working off the call. Both are drawn in the phone's own
 * units, a 168-wide stage, and scaled to the disc by the viewBox.
 */

const STAGE = 168;
const HALF = STAGE / 2;
/** Where the bars start, from the centre: just clear of the phone's 92-wide mark. */
const BARS_INNER = 92 / 2 + 7;
const BARS_REACH = HALF - BARS_INNER - 2;
const BARS = 60;
/** The arc's radius: just round the mark. */
const RING_RADIUS = 92 / 2 + 6;
/** Reduced motion shows the bars short and still. */
const STILL_LEVEL = 0.15;
/** A frame every 32ms is plenty for bars, and half the work at 60Hz. */
const FRAME_MS = 32;

/** The bars while the reply speaks; otherwise the arc while the teammate works; otherwise nothing. */
export function stageRing(phase: CallPhase, working: boolean): "bars" | "working" | null {
	if (phase === "speaking") return "bars";
	return working ? "working" : null;
}

/**
 * Whether the teammate on the call is working off it: its own conversation's
 * turn, or a work thread of its own with a turn going. A call with the desk
 * counts any teammate's work.
 */
export function callWorking(roster: readonly Pick<RosterEntry, "persona" | "session" | "sides">[], personaId: string | undefined): boolean {
	const rows = personaId === undefined ? roster : roster.filter((row) => row.persona.id === personaId);
	return rows.some((row) => {
		const state = shownState(row.session);
		return state === "thinking" || state === "starting" || (row.sides ?? []).some((side) => side.working);
	});
}

/**
 * One bar's length in stage units: at least a dot, and with the level up to
 * nearly the stage's edge. Each bar sways on two waves of its own, so a
 * level reads as a voice and not sixty bars drawn alike.
 */
export function barLength(index: number, level: number, seconds: number): number {
	const sway = 0.5 + 0.5 * Math.sin(index * 1.7 + seconds * 9) * Math.sin(index * 0.43 - seconds * 5.3);
	return 2 + level * (BARS_REACH * 0.3 + BARS_REACH * 0.68 * sway);
}

/** The bars are faint at a whisper and whole at a speaking voice. */
export function barsOpacity(level: number): number {
	return 0.45 + 0.55 * Math.min(1, level * 1.4);
}

function barsPath(level: number, seconds: number): string {
	let d = "";
	for (let index = 0; index < BARS; index++) {
		// From twelve o'clock, round clockwise.
		const angle = (index / BARS) * Math.PI * 2 - Math.PI / 2;
		const outer = BARS_INNER + barLength(index, level, seconds);
		const x = Math.cos(angle);
		const y = Math.sin(angle);
		d += `M${(HALF + x * BARS_INNER).toFixed(1)} ${(HALF + y * BARS_INNER).toFixed(1)}L${(HALF + x * outer).toFixed(1)} ${(HALF + y * outer).toFixed(1)}`;
	}
	return d;
}

/**
 * Speaking: bars round the mark that move with the reply's playing audio,
 * read from the call's level, and lie flat in its pauses. They are drawn
 * outside React's renders; reduced motion shows a still ring of short bars.
 */
export function VoiceBars({ call }: { call: Call }) {
	const path = useRef<SVGPathElement>(null);

	useEffect(() => {
		const element = path.current;
		if (element === null) return;
		const draw = (level: number, seconds: number) => {
			element.setAttribute("d", barsPath(level, seconds));
			element.setAttribute("stroke-opacity", barsOpacity(level).toFixed(3));
		};
		if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true) {
			draw(STILL_LEVEL, 0);
			return;
		}
		let speech = 0;
		const unwatch = call.watchLevel((level) => {
			speech = level;
		});
		draw(speech, performance.now() / 1000);
		// The window stops animation frames while it is hidden, so this costs nothing nobody sees.
		let last = 0;
		let frame = 0;
		const step = (now: number) => {
			frame = window.requestAnimationFrame(step);
			if (now - last < FRAME_MS) return;
			last = now;
			draw(speech, now / 1000);
		};
		frame = window.requestAnimationFrame(step);
		return () => {
			window.cancelAnimationFrame(frame);
			unwatch();
		};
	}, [call]);

	return (
		<svg className="call-bars" viewBox={`0 0 ${STAGE} ${STAGE}`} aria-hidden="true" focusable="false">
			<path ref={path} fill="none" stroke="currentColor" strokeWidth={2.6} strokeLinecap="round" />
		</svg>
	);
}

/** Working: an arc that runs round the mark while the teammate works off the call. Reduced motion holds it still. */
export function WorkingRing() {
	const around = 2 * Math.PI * RING_RADIUS;
	return (
		<svg className="call-working" viewBox={`0 0 ${STAGE} ${STAGE}`} aria-hidden="true" focusable="false">
			<circle cx={HALF} cy={HALF} r={RING_RADIUS} fill="none" stroke="currentColor" strokeOpacity={0.16} strokeWidth={4} />
			<circle
				cx={HALF}
				cy={HALF}
				r={RING_RADIUS}
				fill="none"
				stroke="currentColor"
				strokeWidth={4}
				strokeLinecap="round"
				strokeDasharray={`${around * 0.28} ${around}`}
			/>
		</svg>
	);
}
