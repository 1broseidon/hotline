import { useEffect, useRef, useState } from "react";
import type { ActivityPhase } from "../activity";

/**
 * The Hotline mark, moving because of something.
 *
 * Every pose is a pure function of (phase, seconds in that phase, stall).
 * That is the whole constraint: a movement that can also fire at random
 * cannot mean anything, so the only decorative motion left is the blink, and
 * it shares no vocabulary with the rest. Reading sweeps, searching darts,
 * editing presses, running ratchets — and when a permission request lands
 * the mark stops dead and looks at you, because stillness is the loudest
 * thing available when everything else moves.
 *
 * Once the reply is on its way it collapses into the three dots every
 * messages app uses. The eyes already sit twelve units either side of
 * centre, so the row they fall into is the one they were always standing on.
 *
 * The drawing is assets/hotline-mark.svg, the same one HotlineMark draws still;
 * here the pupils are painted in the ground's colour rather than masked,
 * because a mask cannot be animated part by part and a blink needs the eye
 * and its pupil to squash together.
 */

type Pose = { pupil: number; eyeL: number; eyeR: number; body: number; rot: number; dy: number };

const REST: Pose = { pupil: 0, eyeL: 1, eyeR: 1, body: 1, rot: 0, dy: 0 };

/** Uneven on purpose — a regular sweep would read as reading. */
const DARTS = [-1, 0.6, -0.3, 1, -0.8, 0.2, 0.9, -0.6];

/**
 * Where a tool stops looking like progress. Past this the motion drags
 * rather than quickens: speeding up would be a claim that something is
 * happening, and the slowdown is the only honest thing to say about a
 * process nobody can see into.
 */
const STALL_AFTER = 6;
const STALL_OVER = 6;

/**
 * The one flourish, and why it is allowed. A wink is directed in a way a
 * blink is not, so it cannot sit anywhere it might be read as a verdict —
 * not after a tool call, not on a timer while one runs. But the moment work
 * stops and a reply starts is neither: it is a state change that has
 * already happened and is about to be visible anyway. A beat there is
 * punctuation, and it gives the collapse into dots its anticipation.
 */
const HANDOFF = 560;

/** Shut quicker than opened, then a moment with both eyes open: a gesture, not a twitch. */
function winkAt(p: number): number {
	if (p < 0.36) return 1 - 0.92 * ease(p / 0.36);
	if (p < 0.45) return 0.08;
	if (p < 0.78) return 0.08 + 0.92 * ease((p - 0.45) / 0.33);
	return 1;
}

const REDUCED_MOTION = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

const ease = (p: number) => (p < 0.5 ? 4 * p ** 3 : 1 - (-2 * p + 2) ** 3 / 2);

function poseOf(phase: ActivityPhase, t: number, stall: number): Pose {
	const speed = 1 - 0.72 * stall;
	switch (phase) {
		/* Reading is eye movement: steadily across, then a flick back. */
		case "read": {
			const p = ((t * speed) / 1.7) % 1;
			const x = p < 0.82 ? -1 + (p / 0.82) * 2 : 1 - ((p - 0.82) / 0.18) * 2;
			return { ...REST, pupil: x * 2.6 };
		}
		/* Looking for a thing rather than at one, so the rhythm is irregular. */
		case "search": {
			const step = Math.floor((t * speed) / 0.34);
			const n = ((step % DARTS.length) + DARTS.length) % DARTS.length;
			return { ...REST, pupil: DARTS[n]! * 2.8 };
		}
		/* Work with weight behind it. */
		case "edit": {
			const push = Math.sin((((t * speed) / 0.95) % 1) * Math.PI);
			return { ...REST, body: 1 + 0.2 * push, dy: 1.6 * push };
		}
		/* Geared steps — a command is a discrete thing happening. */
		case "execute": {
			const steps = (t * speed) / 0.46;
			const f = ease(steps - Math.floor(steps));
			return { ...REST, body: 1 + 0.13 * Math.sin(f * Math.PI), rot: -1.5 + 3 * f };
		}
		/* A tool that named no kind. Says work is happening and nothing more. */
		case "doing":
			return { ...REST, body: 1 + 0.07 * Math.sin(t * speed * 3.4) };
		case "thinking":
			return {
				...REST,
				rot: -3.6 * Math.sin(t * speed * 0.7),
				dy: 2.2 * Math.sin(t * speed * 0.5),
				pupil: 0.9 * Math.sin(t * speed * 0.42),
			};
		/* Motionless, but not dead: one slow blink, so it reads as held
		 * rather than hung. Nothing else in the vocabulary is this still. */
		case "blocked": {
			const shut = t % 5 > 4.75 ? 0.12 : 1;
			return { ...REST, eyeL: shut, eyeR: shut };
		}
		/* A copy, never REST itself: the loop writes the blink and the wink
		 * into the pose it is handed, and a shared constant with a shut eye
		 * written into it is a mark with no eyes for the rest of the day. */
		case "writing":
			return { ...REST };
	}
}

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

	/* The dots are held back for one beat so the wink has somewhere to play:
	 * once the class flips, CSS owns the eyes and nothing the loop paints on
	 * them can show. */
	const [showDots, setShowDots] = useState(phase === "writing");
	const dots = useRef(showDots);
	dots.current = showDots;

	useEffect(() => {
		if (phase !== "writing") {
			setShowDots(false);
			return;
		}
		const timer = setTimeout(() => setShowDots(true), HANDOFF);
		return () => clearTimeout(timer);
	}, [phase]);

	useEffect(() => {
		if (REDUCED_MOTION) return;
		let frame = 0;
		let blinkAt = -1;
		let nextBlink = 2 + Math.random() * 3;

		const draw = (now: number) => {
			frame = requestAnimationFrame(draw);
			const node = root.current;
			if (!node) return;
			if (since.current < 0) since.current = now;

			const phase = live.current;
			const held = (now - since.current) / 1000;
			const stall =
				phase === "read" || phase === "edit" || phase === "execute" || phase === "doing"
					? Math.min(1, Math.max(0, (held - STALL_AFTER) / STALL_OVER))
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

			const pose = poseOf(phase, held, stall);
			pose.eyeL = Math.min(pose.eyeL, blink);
			pose.eyeR = Math.min(pose.eyeR, blink);

			/* The handoff: while the dots are held back, the mark is at rest
			 * and winks once — then the class flips and it collapses. */
			if (phase === "writing" && !dots.current) {
				const p = Math.min(1, (held * 1000) / HANDOFF);
				pose.pupil = 0;
				pose.body = 1;
				pose.rot = 0;
				pose.dy = 0;
				pose.eyeL = winkAt(p);
				pose.eyeR = 1;
			}

			paint(node, pose);
		};

		frame = requestAnimationFrame(draw);
		return () => cancelAnimationFrame(frame);
	}, []);

	return (
		<svg
			ref={root}
			className={`glyph ${showDots ? "glyph-dots" : "glyph-mark"}`}
			viewBox="4 15 56 34"
			width="30"
			height="18"
			aria-hidden="true"
			focusable="false"
		>
			<g className="g-all">
				<g className="g-body">
					<rect x="4" y="30" width="56" height="18" rx="6" />
				</g>
				{/* Grows in as the body shrinks out, so the row lands on three
				    of the same thing rather than two dots and a squashed bar. */}
				<circle className="g-dot" cx="32" cy="39" r="4.2" />
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
			</g>
		</svg>
	);
}

/**
 * The only place that touches the DOM. Attributes rather than React state:
 * this runs every frame, and re-rendering a component sixty times a second
 * to move two rectangles would cost more than the animation does.
 */
function paint(root: SVGSVGElement, pose: Pose): void {
	const set = (selector: string, name: string, value: string) =>
		root.querySelector(selector)?.setAttribute(name, value);
	const l = pose.eyeL.toFixed(3);
	const r = pose.eyeR.toFixed(3);
	set(".g-all", "transform", `translate(0 ${pose.dy.toFixed(2)}) rotate(${pose.rot.toFixed(2)} 32 34)`);
	set(".g-body", "transform", `scale(1 ${pose.body.toFixed(3)})`);
	set(".g-eyeL", "transform", `scale(1 ${l})`);
	set(".g-eyeR", "transform", `scale(1 ${r})`);
	// The pupils are one group, so an uneven wink squashes them by the lesser
	// of the two — the open eye keeps its slit and the shut one has nothing to show.
	set(".g-pupils", "transform", `translate(${pose.pupil.toFixed(2)} 0) scale(1 ${Math.max(pose.eyeL, pose.eyeR).toFixed(3)})`);
}
