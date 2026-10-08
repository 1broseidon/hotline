import { useEffect, useRef } from "react";
import { followLevel } from "../voice/dictation";

/** Any stream of levels, 0 to 1: a dictation's `watchLevel`, or a call's. */
export type LevelSource = (listener: (level: number) => void) => () => void;

/** Waiting for the engine, hearing, or waiting for the final words. */
export type MeterState = "waiting" | "listening" | "finishing";

/** How tall each bar may stand at full voice: a hump, the way a voice looks. */
const REACH = [0.55, 0.8, 1, 0.8, 0.55];
/** Each bar's own sway, so a level reads as a voice and not one bar drawn five times. */
const PHASES = [0, 1.7, 0.6, 2.4, 1.1];
/** The bars at rest, and how far they breathe while nobody speaks: the beat's 1800ms. */
const FLOOR = 0.2;
const BREATH = 0.08;
const BREATH_MS = 1800;

/**
 * A voice as a few rounded bars that rise with the level. They breathe
 * while it is quiet, so a listening meter never looks dead, and shimmer
 * one after another while the final words are on their way. The level is
 * smoothed every frame towards the latest reading (`followLevel`), and the
 * bars are moved by a CSS variable, outside React's renders. With reduced
 * motion the bars only follow the level: no breath, no sway, no shimmer.
 */
export function VoiceMeter({ source, state, className = "" }: { source: LevelSource; state: MeterState; className?: string }) {
	const box = useRef<HTMLSpanElement>(null);

	useEffect(() => {
		const element = box.current;
		if (element === null || state === "finishing") return;
		const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
		const bars = Array.from(element.children) as HTMLElement[];
		let target = 0;
		let shown = 0;
		let last = performance.now();
		let frame = 0;
		const unwatch = source((level) => {
			target = level;
		});
		const draw = (now: number) => {
			shown = followLevel(shown, target, now - last);
			last = now;
			const floor = still ? FLOOR : FLOOR + BREATH * (0.5 + 0.5 * Math.sin((2 * Math.PI * now) / BREATH_MS));
			bars.forEach((bar, index) => {
				const sway = still ? 1 : 0.8 + 0.2 * Math.sin(now / 140 + PHASES[index]!);
				const height = Math.min(1, floor + shown * REACH[index]! * sway * (1 - floor));
				bar.style.setProperty("--bar", height.toFixed(3));
			});
			frame = requestAnimationFrame(draw);
		};
		frame = requestAnimationFrame(draw);
		return () => {
			cancelAnimationFrame(frame);
			unwatch();
		};
	}, [source, state]);

	return (
		<span ref={box} className={`voice-meter ${className}`} data-state={state} aria-hidden="true">
			{REACH.map((_, index) => (
				<span key={index} className="voice-meter-bar" style={{ animationDelay: `${index * 90}ms` }} />
			))}
		</span>
	);
}
