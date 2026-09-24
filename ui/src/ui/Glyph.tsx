import { useEffect, useRef } from "react";
import type { ActivityPhase } from "../activity";
import { RECEIVER_SMALL as RECEIVER, receiverLine, receiverPath } from "./receiver";

/**
 * The Hotline mark, moving because of something.
 *
 * Every pose is a pure function of (phase, seconds in that phase, stall).
 * That is the whole constraint: a movement that can also fire at random
 * cannot mean anything, so the only decorative motion left is the blink, and
 * it shares no vocabulary with the rest.
 *
 * The receiver is the largest thing that moves at 30px, so it carries each
 * phase, and every phase differs from every other on at least two of three
 * channels — where the handset is held, what the eyes do, how the body
 * moves. At rest it is on the cradle. Thinking holds it up at the ear and
 * looks up. Reading parks it low and squints down the page. Searching snaps
 * the head from place to place with the handset swinging against it.
 * Editing tucks it at the shoulder, the way you hold a phone to type, and
 * the body takes each keystroke. Running hops. Waiting on you puts it back
 * on the cradle and rings, then stares. A blink marks each change into a
 * new kind of work, so the change is seen.
 *
 * The reply is the call itself: the handset comes off the head and pulls
 * out into a line above it, and the toad stays below watching it — on the
 * line — for as long as the words are on their way. When the turn ends the transcript
 * holds the mark in `landed` for LANDED_MS: the line reels back into a
 * handset, drops onto the cradle with a clunk, and the toad winks — done,
 * over to you. The hang-up is where the call ends, not where the reply
 * starts.
 *
 * The drawing is assets/hotline-mark-small.svg, the same one HotlineMark draws
 * still; here the pupils are painted in the ground's colour rather than
 * masked, because a mask cannot be animated part by part and a blink needs
 * the eye and its pupil to squash together.
 */

type Pose = {
	pupil: number;
	pupilY: number;
	eyeL: number;
	eyeR: number;
	/** Both axes of the eyes: only the stare widens them. */
	wide: number;
	body: number;
	rot: number;
	dy: number;
	/** The handset, off the cradle: up, turned, across. */
	lift: number;
	tilt: number;
	hx: number;
	/** The handset's own jolt, on top of where it is held: the ring. */
	hy: number;
	ht: number;
};

const REST: Pose = { pupil: 0, pupilY: 0, eyeL: 1, eyeR: 1, wide: 1, body: 1, rot: 0, dy: 0, lift: 0, tilt: 0, hx: 0, hy: 0, ht: 0 };

/** Uneven on purpose — a regular sweep would read as reading. */
const DARTS = [-1, 0.6, -0.3, 1, -0.8, 0.2, 0.9, -0.6];

/** Typing has a rhythm, not a metronome: key times within one phrase. */
const KEYS = [0, 0.12, 0.22, 0.4, 0.49, 0.63, 0.71, 0.86];
const PHRASE = 1.15;
const HOP = 0.62;

/**
 * Where a tool stops looking like progress. Past this the motion drags
 * rather than quickens: speeding up would be a claim that something is
 * happening, and the slowdown is the only honest thing to say about a
 * process nobody can see into.
 */
const STALL_AFTER = 6;
const STALL_OVER = 6;

/**
 * Two rings when a permission request lands and two more half a minute
 * later if it is still waiting; then the stare holds on its own. A phone
 * that rang for as long as you ignored it would be the loudest thing in the
 * window, and the stare is meant to be.
 */
const RING_AGAIN = 30;

/** The reply: the handset lifts off into the line over MORPH, and landing reels it back. */
const LINE_POINTS = 80;
const MORPH = 0.5;
const REEL = 0.4;
const DROP = 0.3;
const CLUNK = 0.12;
const WINK = 0.56;
/** When the first blink comes after the mark mounts: as it finishes rising from behind the composer (`wake` in index.css). */
const WAKE_BLINK = 0.7;
/** How long the transcript keeps the mark after the turn so it can land. */
export const LANDED_MS = (REEL + DROP + CLUNK + WINK) * 1000 + 100;

/** The line is the handset's own line, as thick as the handset: one line, lifted into the reply and back. */
const LINE_WIDTH = RECEIVER.thick;

/** Shut quicker than opened, then a moment with both eyes open: a gesture, not a twitch. */
function winkAt(p: number): number {
	if (p < 0.36) return 1 - 0.92 * ease(p / 0.36);
	if (p < 0.45) return 0.08;
	if (p < 0.78) return 0.08 + 0.92 * ease((p - 0.45) / 0.33);
	return 1;
}

const REDUCED_MOTION = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

const ease = (p: number) => (p < 0.5 ? 4 * p ** 3 : 1 - (-2 * p + 2) ** 3 / 2);
const easeIn = (p: number) => p ** 3;
const clamp01 = (p: number) => Math.max(0, Math.min(1, p));

/** Brrring, brrring: the handset jumps on the cradle. */
function ring(t: number): { hy: number; ht: number } {
	const c = t >= RING_AGAIN ? t - RING_AGAIN : t;
	const on = t < RING_AGAIN + 1.3 && (c < 0.5 || (c > 0.8 && c < 1.3));
	if (!on) return { hy: 0, ht: 0 };
	const w = Math.sin(t * Math.PI * 2 * 15);
	return { hy: -1.1 * Math.abs(w), ht: 3.2 * w };
}

/** Crouch, spring, land: one hop is one step of a command. */
function hop(t: number): { dy: number; body: number } {
	const p = (((t % HOP) + HOP) % HOP) / HOP;
	if (p < 0.2) {
		const k = ease(p / 0.2);
		return { dy: 1.4 * k, body: 1 - 0.16 * k };
	}
	if (p < 0.72) {
		const q = (p - 0.2) / 0.52;
		return { dy: 1.4 - 7.4 * Math.sin(q * Math.PI) - 1.4 * q, body: q < 0.3 ? 0.84 + 0.24 * ease(q / 0.3) : 1.08 - 0.08 * ease((q - 0.3) / 0.7) };
	}
	const q = (p - 0.72) / 0.28;
	return { dy: 1.2 * Math.sin(q * Math.PI), body: 1 - 0.12 * Math.sin(q * Math.PI) };
}

function poseOf(phase: ActivityPhase, t: number, stall: number): Pose {
	const s = t * (1 - 0.72 * stall);
	switch (phase) {
		/* Up at the ear and looking up: the one phase held high. */
		case "thinking":
			return {
				...REST,
				rot: -3.6 * Math.sin(t * 0.7),
				dy: 2.2 * Math.sin(t * 0.5),
				pupil: 1.8 * Math.sin(t * 0.42),
				pupilY: -2.2,
				lift: 8,
				tilt: -16 + 3 * Math.sin(t * 0.6),
			};
		/* Parked low and level, squinting down the page: steadily across, then a flick back. */
		case "read": {
			const p = (s / 1.7) % 1;
			const x = p < 0.82 ? -1 + (p / 0.82) * 2 : 1 - ((p - 0.82) / 0.18) * 2;
			return { ...REST, pupil: x * 2.8, pupilY: 1.3, eyeL: 0.62, eyeR: 0.62, lift: 3 };
		}
		/* Looking for a thing rather than at one: the head snaps to each new
		 * place and the handset swings the other way. */
		case "search": {
			const step = t / 0.42;
			const n = Math.floor(step) % DARTS.length;
			const prev = DARTS[(n + DARTS.length - 1) % DARTS.length]!;
			const d = prev + (DARTS[n]! - prev) * ease(clamp01((step - Math.floor(step)) / 0.28));
			return { ...REST, pupil: d * 3, pupilY: DARTS[(n + 3) % DARTS.length]! * 1.4, rot: d * 5, lift: 5, tilt: -d * 9 };
		}
		/* On the phone while typing: the handset tucked at the shoulder, eyes
		 * down on the caret, the body taking each keystroke. */
		case "edit": {
			const c = s % PHRASE;
			let tap = 0;
			let side = 1;
			let keys = 0;
			for (let i = 0; i < KEYS.length && c >= KEYS[i]!; i++) {
				keys = i + 1;
				side = i % 2 ? -1 : 1;
				tap = Math.exp(-(c - KEYS[i]!) / 0.05);
			}
			return {
				...REST,
				pupil: -2.6 + (5.2 * keys) / KEYS.length,
				pupilY: 1.5,
				body: 1 - 0.06 * tap,
				dy: 0.7 * tap,
				rot: 1.3 * side * tap,
				lift: 2.5,
				hx: 3,
				tilt: 20,
				hy: -0.4 * tap,
			};
		}
		/* A toad running is a toad hopping, the handset trailing each hop by a beat. */
		case "execute": {
			const h = hop(s);
			const lag = hop(s - 0.07);
			return { ...REST, body: h.body, dy: h.dy, pupilY: -0.4, lift: 4, tilt: -3, hy: lag.dy - h.dy };
		}
		/* A tool that named no kind. Says work is happening and nothing more. */
		case "doing":
			return { ...REST, body: 1 + 0.07 * Math.sin(s * 3.4), lift: 4 + 1.4 * Math.sin(s * 2.2), tilt: -4 };
		/* On the cradle, sat up a little so the widened eyes keep their cut,
		 * ringing, then the stare: nothing else in the vocabulary is this still.
		 * One slow blink, so it reads as held rather than hung. */
		case "blocked": {
			const shut = t % 5 > 4.75 ? 0.12 : 1;
			return { ...REST, wide: 1.12, eyeL: shut, eyeR: shut, lift: 2.4, ...ring(t) };
		}
		/* The loop draws these: they are a line, not a pose. A copy, never
		 * REST itself — the loop writes into the pose it is handed. */
		case "writing":
		case "landed":
			return { ...REST };
		/* Going back to sleep: on the cradle, still. */
		case "rest":
			return { ...REST };
	}
}

/**
 * On the line: the handset lifted off the head and pulled out into a wave
 * that runs along itself above the toad, tapered at both ends. The toad
 * stays below and watches it — the reply is coming down the line to you.
 */
function voiceLine(t: number): [number, number][] {
	const points: [number, number][] = [];
	for (let i = 0; i < LINE_POINTS; i++) {
		const u = i / (LINE_POINTS - 1);
		const x = 10 + 44 * u;
		points.push([x, 10 + 2.6 * Math.sin(Math.PI * u) ** 0.6 * Math.sin((2 * Math.PI * x) / 9 - 2 * Math.PI * 1.4 * t)]);
	}
	return points;
}

/** The toad watching the line, `m` of the way there: eyes up and wandering along it, sat a little lower. */
function watching(pose: Pose, t: number, m: number): void {
	pose.pupilY = -1.8 * m;
	pose.pupil = 2.2 * Math.sin(t * 2.4) * m;
	pose.dy = 1.2 * m;
}

const HANDSET = receiverLine(RECEIVER, LINE_POINTS);

/** The handset's centreline where the painted handset is. */
function heldLine(lift: number, tilt: number, hx: number): [number, number][] {
	const [cx, cy] = HANDSET.centre;
	const a = (tilt * Math.PI) / 180;
	const c = Math.cos(a);
	const s = Math.sin(a);
	return HANDSET.points.map(([x, y]) => [cx + (x - cx) * c - (y - cy) * s + hx, cy + (x - cx) * s + (y - cy) * c - lift]);
}

const mix = (a: [number, number][], b: [number, number][], m: number): [number, number][] =>
	a.map(([x, y], i) => [x + (b[i]![0] - x) * m, y + (b[i]![1] - y) * m]);

type Line = { points: [number, number][]; width: number };

export function Glyph({ phase }: { phase: ActivityPhase }) {
	const root = useRef<SVGSVGElement>(null);
	// Read inside the loop rather than closed over, so a phase change takes
	// effect on the next frame instead of on the next mount.
	const live = useRef(phase);
	const since = useRef(0);
	if (live.current !== phase) {
		live.current = phase;
		since.current = -1;
	}

	useEffect(() => {
		if (!REDUCED_MOTION) return;
		/* Held still: the pose at rest, or the line still above the toad while a reply is on its way. */
		const node = root.current;
		if (!node) return;
		if (phase === "writing") {
			const pose = { ...REST };
			watching(pose, 0, 1);
			paint(node, pose, { lift: 0, tilt: 0, hx: 0 }, { points: voiceLine(0), width: LINE_WIDTH });
		} else paint(node, { ...REST }, { lift: 0, tilt: 0, hx: 0 }, null);
	}, [phase]);

	useEffect(() => {
		if (REDUCED_MOTION) return;
		let frame = 0;
		let last = performance.now();
		let blinkAt = -1;
		// The first blink is the waking one: just as the mark clears the composer.
		let nextBlink = performance.now() / 1000 + WAKE_BLINK;
		/* The handset eases between holds — picking up a phone takes a
		 * moment — and everything else is painted raw. */
		const held = { lift: 0, tilt: 0, hx: 0 };
		let entry = { ...held };
		/* Where the voice line had got to, so landing reels in from there. */
		let spoken = 0;

		const draw = (now: number) => {
			frame = requestAnimationFrame(draw);
			const node = root.current;
			if (!node) return;
			const dt = Math.min(0.05, (now - last) / 1000);
			last = now;
			const phase = live.current;
			if (since.current < 0) {
				since.current = now;
				entry = { ...held };
				if (phase !== "writing" && phase !== "landed" && phase !== "rest") blinkAt = now / 1000;
			}
			const t = (now - since.current) / 1000;
			const stall =
				phase === "read" || phase === "edit" || phase === "execute" || phase === "doing"
					? Math.min(1, Math.max(0, (t - STALL_AFTER) / STALL_OVER))
					: 0;

			// The blink is the one thing here that is not caused, and it is
			// suppressed while blocked because the stare is the whole message.
			const clock = now / 1000;
			if (clock > nextBlink && phase !== "blocked") {
				blinkAt = clock;
				nextBlink = clock + 2.4 + Math.random() * 4;
			}
			const gap = clock - blinkAt;
			const blink = gap >= 0 && gap < 0.18 ? 0.08 + 0.92 * Math.abs(gap / 0.09 - 1) : 1;

			const pose = poseOf(phase, t, stall);
			pose.eyeL = Math.min(pose.eyeL, blink);
			pose.eyeR = Math.min(pose.eyeR, blink);
			const k = 1 - Math.exp(-dt / 0.08);
			let line: Line | null = null;

			if (phase === "writing") {
				/* Off the head and into the line, the toad looking up after it. */
				const m = ease(clamp01(t / MORPH));
				line = { points: mix(heldLine(entry.lift, entry.tilt, entry.hx), voiceLine(t), m), width: LINE_WIDTH };
				watching(pose, t, m);
				spoken = t;
			} else if (phase === "landed") {
				/* Reel in, drop onto the cradle, clunk, wink. */
				if (t < REEL) {
					const m = ease(t / REEL);
					line = { points: mix(voiceLine(spoken + t), heldLine(3, 0, 0), m), width: LINE_WIDTH };
					watching(pose, spoken + t, 1 - m);
					Object.assign(held, { lift: 3, tilt: 0, hx: 0 });
				} else {
					const c = t - REEL;
					held.lift = c < DROP ? 3 * (1 - easeIn(c / DROP)) : 0;
					const d = c - DROP;
					if (d >= 0 && d < CLUNK) pose.body = 1 - 0.1 * Math.sin((d / CLUNK) * Math.PI);
					else if (d >= CLUNK && d < CLUNK + WINK) pose.eyeL = Math.min(pose.eyeL, winkAt((d - CLUNK) / WINK));
				}
			} else if (phase === "search" && t > 0.25) {
				/* The swing is the search; easing it would blunt every snap. */
				Object.assign(held, { lift: pose.lift, tilt: pose.tilt, hx: pose.hx });
			} else {
				held.lift += (pose.lift - held.lift) * k;
				held.tilt += (pose.tilt - held.tilt) * k;
				held.hx += (pose.hx - held.hx) * k;
			}

			paint(node, pose, held, line);
		};

		frame = requestAnimationFrame(draw);
		return () => cancelAnimationFrame(frame);
	}, []);

	return (
		<svg ref={root} className="glyph" viewBox="4 8 56 40" width="30" height="21.43" aria-hidden="true" focusable="false">
			<g className="g-all">
				<g className="g-body">
					<rect x="4" y="30" width="56" height="18" rx="6" />
				</g>
				<g className="g-eyeL">
					<circle cx="20" cy="30" r="10.5" />
				</g>
				<g className="g-eyeR">
					<circle cx="44" cy="30" r="10.5" />
				</g>
				<g className="g-pupils">
					<rect x="14.5" y="28" width="11" height="4" rx="2" />
					<rect x="38.5" y="28" width="11" height="4" rx="2" />
				</g>
				<g className="g-hand">
					<path className="g-receiver" d={receiverPath(RECEIVER)} />
				</g>
				<path className="g-line" d="" opacity="0" />
			</g>
		</svg>
	);
}

/**
 * The only place that touches the DOM. Attributes rather than React state:
 * this runs every frame, and re-rendering a component sixty times a second
 * to move a handful of shapes would cost more than the animation does.
 */
function paint(root: SVGSVGElement, pose: Pose, held: { lift: number; tilt: number; hx: number }, line: Line | null): void {
	const set = (selector: string, name: string, value: string) => root.querySelector(selector)?.setAttribute(name, value);
	const w = pose.wide;
	const f = (v: number) => Math.max(0.001, v).toFixed(3);
	set(".g-all", "transform", `translate(0 ${pose.dy.toFixed(2)}) rotate(${pose.rot.toFixed(2)} 32 34)`);
	set(".g-body", "transform", `scale(1 ${f(pose.body)})`);
	set(".g-eyeL", "transform", `scale(${f(w)} ${f(pose.eyeL * w)})`);
	set(".g-eyeR", "transform", `scale(${f(w)} ${f(pose.eyeR * w)})`);
	// The pupils are one group, so an uneven wink squashes them by the lesser
	// of the two — the open eye keeps its slit and the shut one has nothing to show.
	set(".g-pupils", "transform", `translate(${pose.pupil.toFixed(2)} ${pose.pupilY.toFixed(2)}) scale(1 ${f(Math.max(pose.eyeL, pose.eyeR) * w)})`);
	set(".g-hand", "transform", `translate(${held.hx.toFixed(2)} ${(pose.hy - held.lift).toFixed(2)})`);
	set(".g-receiver", "transform", `rotate(${(held.tilt + pose.ht).toFixed(2)})`);
	set(".g-hand", "opacity", line ? "0" : "1");
	set(".g-line", "opacity", line ? "1" : "0");
	if (line) {
		set(".g-line", "d", `M${line.points.map(([x, y]) => `${x.toFixed(2)} ${y.toFixed(2)}`).join("L")}`);
		set(".g-line", "stroke-width", line.width.toFixed(2));
	}
}
