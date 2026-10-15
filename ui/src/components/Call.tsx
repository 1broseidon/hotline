import { useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, HangUpIcon, PauseIcon, PhoneIcon, PlayIcon, TranscriptIcon } from "../icons";
import { Avatar } from "../ui/Avatar";
import { callWorking } from "./CallStage";
import { activeDeskId } from "../desks";
import type { RosterEntry } from "../wire";
import { type Call as CallSession, type CallLine, type CallPhase, closeCall, restartCall, useCallSnapshot } from "../voice/call";

const WORDS: Record<CallPhase, string> = {
	connecting: "Calling",
	listening: "Listening",
	hearing: "Hearing you",
	thinking: "Thinking",
	speaking: "Speaking",
	held: "On hold",
	ended: "Ended",
};

/**
 * Every call, to a teammate or to the desk, shows in one place: the island,
 * a black pill on the title bar's centre line that stands where the mark
 * rests, the way the phone shows a call. It stays while the window moves
 * between conversations and panes. It leads with who is on the line, the
 * teammate's face or the desk's handset, ringed green while the line is live
 * and swelling with the voice (pressing it while they speak cuts in), then
 * the name (a teammate's opens its conversation), what the call is doing and
 * its clock, any teammate that needs you, the voice on the line, and the
 * transcript, Hold and Hang up keys. The transcript drops down under it. A
 * call that ends folds the island back into the mark; one that ended on
 * trouble stays to say why, with Call again and Close.
 */
export function CallIsland({
	call,
	names,
	roster,
	onOpenTeammate,
}: {
	call: CallSession;
	names: (personaId: string) => string | undefined;
	/** The desk on screen's roster; a call to another desk reads none of it. */
	roster: readonly RosterEntry[];
	onOpenTeammate(personaId: string): void;
}) {
	const state = useCallSnapshot(call);
	const island = useRef<HTMLDivElement>(null);
	const again = useRef<HTMLButtonElement>(null);
	const [transcript, setTranscript] = useState(false);
	useEffect(() => call.watchLevel((level) => island.current?.style.setProperty("--level", level.toFixed(3))), [call]);
	const live = state.phase !== "ended";
	// When the line goes, the next thing to press is Call again.
	useEffect(() => {
		if (!live) again.current?.focus();
	}, [live]);
	// A plain hang-up folds the island away; trouble stays to be read.
	const folding = !live && state.trouble === undefined;
	const speaking = state.phase === "speaking" || state.phase === "thinking";
	const target = call.target;
	return (
		<div className="island-wrap">
			<div
				ref={island}
				className="island"
				role="region"
				data-phase={state.phase}
				data-folding={folding ? "" : undefined}
				aria-label={`Call with ${target?.name ?? "the desk"}`}
				onAnimationEnd={(event) => {
					if (folding && event.target === event.currentTarget) closeCall();
				}}
			>
				<button
					type="button"
					className="island-mark"
					title={speaking ? "Press to cut in" : undefined}
					aria-label={speaking ? "Cut in" : WORDS[state.phase]}
					// aria-disabled rather than disabled, so focus stays put when the desk stops talking.
					aria-disabled={!speaking}
					onClick={() => {
						if (speaking) call.interrupt();
					}}
				>
					{target === undefined ? <PhoneIcon /> : <Avatar id={target.personaId} name={target.name} hash={target.avatarHash} size={20} read={call.readAvatar} />}
				</button>
				{target === undefined ? (
					<span className="island-name">Desk</span>
				) : (
					<button type="button" className="island-name" onClick={() => onOpenTeammate(target.personaId)}>
						{target.name}
					</button>
				)}
				<CallStatus call={call} roster={roster} />
				{state.cards.map((card) => (
					<button
						key={card.requestId}
						type="button"
						className="island-card"
						onClick={() => {
							call.dismissCard(card.requestId);
							onOpenTeammate(card.personaId);
						}}
					>
						{call.nameOf(card.personaId) ?? names(card.personaId) ?? "A teammate"} needs you
					</button>
				))}
				{state.trouble !== undefined && (
					<span className="island-trouble" role="alert" title={state.trouble}>
						{state.trouble}
					</span>
				)}
				<span className="min-w-0 flex-1" />
				{live && state.phase !== "connecting" && <CallVoice call={call} still={state.phase === "held"} />}
				<button
					type="button"
					className="island-key"
					title={transcript ? "Hide what was said" : "What was said"}
					aria-label="What was said"
					aria-pressed={transcript}
					onClick={() => setTranscript((was) => !was)}
				>
					<TranscriptIcon />
				</button>
				{live ? (
					<>
						<button
							type="button"
							className="island-key"
							disabled={state.phase === "connecting"}
							title={state.phase === "held" ? "Resume" : "Hold"}
							aria-label={state.phase === "held" ? "Resume" : "Hold"}
							onClick={() => call.hold(state.phase !== "held")}
						>
							{state.phase === "held" ? <PlayIcon /> : <PauseIcon />}
						</button>
						<button type="button" className="island-key island-hang-up" title="Hang up" aria-label="Hang up" onClick={() => call.hangUp()}>
							<HangUpIcon />
						</button>
					</>
				) : (
					<>
						<button ref={again} type="button" className="island-key island-again" title="Call again" aria-label="Call again" onClick={() => void restartCall(call)}>
							<PhoneIcon />
						</button>
						<button type="button" className="island-key" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={closeCall}>
							<CloseIcon />
						</button>
					</>
				)}
			</div>
			{transcript && !folding && <CallTranscript lines={state.lines} name={target?.name ?? "The desk"} />}
		</div>
	);
}

/**
 * What was said on the call, both sides, written down under the island:
 * your words and every line said back, the ones the teammate wrote and the
 * ones the voice answered on its own, which the conversation does not hold.
 * It keeps to the newest as lines arrive, in its own scroll.
 */
function CallTranscript({ lines, name }: { lines: CallLine[]; name: string }) {
	const list = useRef<HTMLOListElement>(null);
	useEffect(() => {
		if (list.current !== null) list.current.scrollTop = list.current.scrollHeight;
	}, [lines.length]);
	return (
		<ol ref={list} className="call-transcript" aria-label="What was said on the call">
			{lines.length === 0 && <li className="call-transcript-empty">Nothing said yet.</li>}
			{lines.map((line) => (
				<li key={line.id} className={line.kind === "you" ? "call-line-you" : "call-line-them"}>
					<span className="sr-only">{line.kind === "you" ? "You: " : `${line.from ?? name}: `}</span>
					{line.text}
				</li>
			))}
		</ol>
	);
}

const VOICE_BARS = 16;
/** Each bar's place across the row: the voice is tallest in the middle. */
const VOICE_SHAPE = Array.from({ length: VOICE_BARS }, (_, index) => 0.3 + 0.7 * Math.sin((Math.PI * (index + 0.5)) / VOICE_BARS));

/** One frame of the bars: dots at silence, each swaying on its own wave as the level rises. */
function voicePath(level: number, seconds: number): string {
	let path = "";
	for (let index = 0; index < VOICE_BARS; index++) {
		const sway = 0.45 + 0.55 * Math.abs(Math.sin(seconds * (5 + (index % 5)) + index * 1.7));
		const half = 0.75 + level * 8 * VOICE_SHAPE[index]! * sway;
		const x = 2.5 + index * 5;
		path += `M${x} ${(10 - half).toFixed(2)}V${(10 + half).toFixed(2)}`;
	}
	return path;
}

/**
 * The voice on the line: a row of bars, dots at silence, that rise and sway
 * with whoever is speaking, you or them. Still while held, and with reduced
 * motion. The path is drawn straight from the call's level, without
 * re-rendering.
 */
function CallVoice({ call, still }: { call: CallSession; still: boolean }) {
	const path = useRef<SVGPathElement>(null);
	useEffect(() => {
		const element = path.current;
		if (element === null) return;
		element.setAttribute("d", voicePath(0, 0));
		if (still || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true) return;
		return call.watchLevel((level) => element.setAttribute("d", voicePath(level, performance.now() / 1000)));
	}, [call, still]);
	return (
		<svg className="call-voice" data-still={still ? "" : undefined} viewBox={`0 0 ${VOICE_BARS * 5} 20`} aria-hidden="true" focusable="false">
			<path ref={path} />
		</svg>
	);
}

/**
 * What a call is doing, whether the teammate is working off it, and its
 * clock, parted by small dots.
 */
function CallStatus({ call, roster }: { call: CallSession; roster: readonly RosterEntry[] }) {
	const state = useCallSnapshot(call);
	const live = state.phase !== "ended";
	const working = callWorking(call.deskId == null || call.deskId === activeDeskId() ? roster : [], call.target?.personaId);
	return (
		<span className="call-status" data-phase={state.phase}>
			{/* Only the state is announced; the clock would be read out every second. */}
			<span aria-live="polite">{WORDS[state.phase]}</span>
			{working && live && state.phase !== "speaking" && (
				<>
					<span className="call-dot" aria-hidden="true" />
					<span>Working</span>
				</>
			)}
			{live && state.phase !== "connecting" && (
				<>
					<span className="call-dot" aria-hidden="true" />
					<Clock clock={state.clock} />
				</>
			)}
		</span>
	);
}

function Clock({ clock }: { clock: { base: number; since: number | null } }) {
	const [now, setNow] = useState(Date.now());
	useEffect(() => {
		if (clock.since === null) return;
		const timer = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(timer);
	}, [clock.since]);
	// Talk time: it stops while the call is on hold, and never counts the dialling.
	const ms = clock.base + (clock.since === null ? 0 : Math.max(0, now - clock.since));
	const seconds = Math.floor(ms / 1000);
	return (
		<span className="call-clock">
			{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
		</span>
	);
}

/**
 * The desk's call key, a handset (teammates' call keys are a soundwave): in
 * the rail's dock, or beside the rail's key while the rail is closed. On a
 * call to the desk it is green, and pressing it hangs up; while a teammate
 * is on the line it waits, disabled. The call itself shows in the island.
 */
export function DeskCallKey({
	call,
	available,
	onCall,
	className = "desk-call-key",
}: {
	call: CallSession | null;
	/** The desk can take a call. */
	available: boolean;
	onCall(): void;
	className?: string;
}) {
	const state = useCallSnapshot(call);
	const live = call !== null && state.phase !== "ended";
	const desk = live && call.target === undefined;
	if (!available && !desk) return null;
	const busy = live && !desk;
	const keys = chordKeys("desk");
	return (
		<button
			type="button"
			className={className}
			data-on={desk ? "" : undefined}
			disabled={busy}
			title={desk ? "Hang up on the desk" : busy ? `On a call with ${call.target?.name ?? "a teammate"}` : `Call the desk${keys === "" ? "" : ` (${keys})`}`}
			aria-label={desk ? "Hang up on the desk" : "Call the desk"}
			onClick={desk ? () => call.hangUp() : onCall}
		>
			<PhoneIcon />
		</button>
	);
}
