import { useEffect, useLayoutEffect, useRef } from "react";
import type { Call, CallPhase } from "../voice/call";

/** Where the toad's parts sit in one frame, in the drawing's units. */
export type ToadPose = {
	/** The eyes' height, 1 when open, scaled about their middle line. */
	eyes: number;
	/** How far the pupils move down; up is negative. */
	pupils: number;
	/** The mouth's size against its full 12 by 3, scaled about its centre. */
	mouthWidth: number;
	mouthHeight: number;
	/** How far the whole toad moves down; up is negative. */
	nod: number;
};

/** The phone moves its 92px-wide toad in pixels; this turns them into the drawing's 56 units. */
const PHONE_PX = 56 / 92;

/**
 * The phone's call toad, frame by frame: the reply's level opens the mouth
 * and lifts the toad, thinking looks up and bobs, hold droops the eyes, and
 * every 4.1 seconds it blinks. Still, nothing moves: the mouth stands half
 * open while the desk speaks, so speaking still reads.
 */
export function toadPose(phase: CallPhase, speech: number, seconds: number, still: boolean): ToadPose {
	const beat = seconds % 4.1;
	const blink = !still && beat > 3.85 ? Math.max(0.12, Math.abs(beat - 3.975) / 0.125) : 1;
	let mouth = 0;
	if (phase === "speaking") mouth = still ? 0.3 : speech;
	let nod = 0;
	if (!still && phase === "speaking") nod = -speech * 1.8 * PHONE_PX;
	if (!still && phase === "thinking") nod = Math.sin(seconds * 1.5) * 1.5 * PHONE_PX;
	return {
		eyes: blink * (phase === "held" ? 0.75 : 1),
		pupils: phase === "thinking" ? -1.5 : 0,
		mouthWidth: 0.7 + mouth * 0.3,
		mouthHeight: 0.15 + mouth * 1.55,
		nod,
	};
}

/** A frame every 32ms is plenty for a toad, and half the work at 60Hz. */
const FRAME_MS = 32;

function draw(svg: SVGSVGElement, pose: ToadPose): void {
	svg.querySelector("[data-part=toad]")?.setAttribute("transform", `translate(0 ${pose.nod.toFixed(2)})`);
	// The eyes are drawn twice, once in ink and once in the cut that holds the pupils; they blink together.
	for (const eyes of svg.querySelectorAll("[data-part=eyes]")) {
		eyes.setAttribute("transform", `matrix(1 0 0 ${pose.eyes.toFixed(3)} 0 ${(30 * (1 - pose.eyes)).toFixed(2)})`);
	}
	svg.querySelector("[data-part=pupils]")?.setAttribute("transform", `translate(0 ${pose.pupils})`);
	const { mouthWidth: sx, mouthHeight: sy } = pose;
	svg.querySelector("[data-part=mouth]")?.setAttribute(
		"transform",
		`matrix(${sx.toFixed(3)} 0 0 ${sy.toFixed(3)} ${(32 * (1 - sx)).toFixed(2)} ${(41 * (1 - sy)).toFixed(2)})`,
	);
}

/**
 * The plain toad as the desk on a call, with a mouth: the reply's own
 * playing audio opens it, read from the call's level, not a looping pulse.
 * Like the mark, it is one colour, and the pupils and mouth are cut through
 * to whatever it sits on. It moves outside React's renders, by attributes,
 * and with reduced motion it holds still.
 */
export function CallToad({ call, phase, width = 56 }: { call: Call; phase: CallPhase; width?: number }) {
	const svg = useRef<SVGSVGElement>(null);
	const speech = useRef(0);

	useEffect(
		() =>
			call.watchLevel((level) => {
				speech.current = level;
			}),
		[call],
	);

	// Before paint, so a new phase never shows a frame of the old pose.
	useLayoutEffect(() => {
		const element = svg.current;
		if (element === null) return;
		const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
		// Outside "speaking" the level is your voice, which the disc shows, not the toad.
		const pose = (seconds: number) => toadPose(phase, phase === "speaking" ? speech.current : 0, seconds, still);
		draw(element, pose(performance.now() / 1000));
		if (still) return;
		// The window stops animation frames while it is hidden, so this costs nothing nobody sees.
		let last = 0;
		let frame = 0;
		const step = (now: number) => {
			frame = window.requestAnimationFrame(step);
			if (now - last < FRAME_MS) return;
			last = now;
			draw(element, pose(now / 1000));
		};
		frame = window.requestAnimationFrame(step);
		return () => window.cancelAnimationFrame(frame);
	}, [phase]);

	return (
		<svg ref={svg} className="call-toad" viewBox="4 17 56 33" width={width} height={(width * 33) / 56} aria-hidden="true" focusable="false">
			<mask id="call-toad-cuts" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">
				<rect width="64" height="64" fill="#fff" />
				<g data-part="eyes">
					<g data-part="pupils" fill="#000">
						<rect x="14.5" y="28" width="11" height="4" rx="2" />
						<rect x="38.5" y="28" width="11" height="4" rx="2" />
					</g>
				</g>
				<rect data-part="mouth" x="26" y="39.5" width="12" height="3" rx="1.5" fill="#000" />
			</mask>
			<g data-part="toad">
				<g mask="url(#call-toad-cuts)" fill="currentColor">
					<rect x="4" y="30" width="56" height="18" rx="6" />
					<g data-part="eyes">
						<circle cx="20" cy="30" r="10.5" />
						<circle cx="44" cy="30" r="10.5" />
					</g>
				</g>
			</g>
		</svg>
	);
}
