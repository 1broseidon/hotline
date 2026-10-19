import { type ReactNode, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, HangUpIcon, InfoIcon, PauseIcon, PhoneIcon, PlayIcon, TranscriptIcon } from "../icons";
import { Avatar } from "../ui/Avatar";
import { callWorking } from "./CallStage";
import { activeDeskId } from "../desks";
import type { RosterEntry } from "../wire";
import { type Call as CallSession, type CallLine, type CallPhase, closeCall, restartCall, useCallSnapshot } from "../voice/call";
import { setHearOnThisMac } from "../voice/hearing";

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
 * The call is a strip under the conversation's band, the band growing down
 * into one card with it the way the phone shows a call: up and out of the
 * way. It stays while the window moves between conversations (on its own
 * card atop a pane with no conversation). It names who is on the line, the
 * desk's toad or the teammate's face (left out under that teammate's own
 * band, whose face rings instead), then what the call is doing and its
 * clock, any teammate that needs you, and Hold and Hang up. The mark rings
 * green while the line is live and swells with the voice; pressing it while
 * they speak cuts in. A call that ends folds the strip back up and closes
 * it; one that ended on trouble stays to say why, with Call again and
 * Close. Work and the transcript stay in the teammate's own conversation,
 * which the name opens.
 */
export function CallBar({
	call,
	names,
	roster,
	onOpenTeammate,
	bare = false,
	alone = false,
}: {
	call: CallSession;
	names: (personaId: string) => string | undefined;
	/** The desk on screen's roster; a call to another desk reads none of it. */
	roster: readonly RosterEntry[];
	onOpenTeammate(personaId: string): void;
	/** Under the called teammate's own band: its face there is the mark. */
	bare?: boolean;
	/** Atop a pane with no band to hang from: a card of its own. */
	alone?: boolean;
}) {
	const state = useCallSnapshot(call);
	const bar = useRef<HTMLElement>(null);
	const again = useRef<HTMLButtonElement>(null);
	const [transcript, setTranscript] = useState(false);

	useEffect(
		() =>
			call.watchLevel((level) => {
				bar.current?.style.setProperty("--level", level.toFixed(3));
			}),
		[call],
	);

	const live = state.phase !== "ended";
	// When the line goes, the next thing to press is Call again.
	useEffect(() => {
		if (!live) again.current?.focus();
	}, [live]);
	const speaking = state.phase === "speaking" || state.phase === "thinking";
	const target = call.target;
	// A plain hang-up folds the strip away; trouble stays to be read.
	const folding = !live && state.trouble === undefined;
	return (
		<>
		<aside
			ref={bar}
			className={alone ? "call-bar call-bar-alone" : "call-bar"}
			data-phase={state.phase}
			data-folding={folding ? "" : undefined}
			data-bare={bare ? "" : undefined}
			aria-label={`Call with ${target?.name ?? "the desk"}`}
			onAnimationEnd={(event) => {
				if (folding && event.target === event.currentTarget) closeCall();
			}}
		>
			{!bare && (
			<button
				type="button"
				className="call-bar-mark"
				title={speaking ? "Press to cut in" : undefined}
				aria-label={speaking ? "Cut in" : WORDS[state.phase]}
				// aria-disabled rather than disabled, so focus stays put when the desk stops talking.
				aria-disabled={!speaking}
				onClick={() => {
					if (speaking) call.interrupt();
				}}
			>
				{target === undefined ? <PhoneIcon /> :
					<Avatar id={target.personaId} name={target.name} hash={target.avatarHash} size={22} read={call.readAvatar} />}
			</button>
			)}
			{/* Under the teammate's own band the status is beside the name there. */}
			{!bare && (
				<CallStatus call={call} roster={roster}>
					{target !== undefined && (
						<>
							<button type="button" className="call-bar-name" onClick={() => onOpenTeammate(target.personaId)}>
								{target.name}
							</button>
							<span className="call-bar-dot" aria-hidden="true" />
						</>
					)}
				</CallStatus>
			)}

			{state.cards.map((card) => (
				<button
					key={card.requestId}
					type="button"
					className="call-card"
					onClick={() => {
						call.dismissCard(card.requestId);
						onOpenTeammate(card.personaId);
					}}
				>
					<span className="min-w-0 flex-1 truncate">{call.nameOf(card.personaId) ?? names(card.personaId) ?? "A teammate"} needs you</span>
				</button>
			))}

			{state.trouble !== undefined && <CallTrouble text={state.trouble} />}

			<span className="min-w-0 flex-1" />

			{live && state.phase !== "connecting" && <CallVoice call={call} still={state.phase === "held"} />}

			<button
				type="button"
				className="call-button"
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
						className="call-button"
						disabled={state.phase === "connecting"}
						title={state.phase === "held" ? "Resume" : "Hold"}
						aria-label={state.phase === "held" ? "Resume" : "Hold"}
						onClick={() => call.hold(state.phase !== "held")}
					>
						{state.phase === "held" ? <PlayIcon /> : <PauseIcon />}
					</button>
					<button type="button" className="call-button call-button-end" title="Hang up" aria-label="Hang up" onClick={() => call.hangUp()}>
						<HangUpIcon />
					</button>
				</>
			) : (
				<>
					{state.deskModel === true && <DeskModelKey call={call} />}
					<button
						ref={again}
						type="button"
						className="call-button call-button-start"
						title="Call again"
						aria-label="Call again"
						onClick={() => void restartCall(call)}
					>
						<PhoneIcon />
					</button>
					<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={closeCall}>
						<CloseIcon />
					</button>
				</>
			)}
		</aside>
		{transcript && <CallTranscript lines={state.lines} name={target?.name ?? "The desk"} alone={alone} />}
		</>
	);
}

/**
 * What was said on the call, both sides, written down under the card: your
 * words and every line said back, the ones the teammate wrote and the ones
 * the voice answered on its own, which the conversation does not hold. It
 * keeps to the newest as lines arrive.
 */
function CallTranscript({ lines, name, alone }: { lines: CallLine[]; name: string; alone: boolean }) {
	const list = useRef<HTMLOListElement>(null);
	// Its own scroll only: the conversation behind it stays where it is.
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new line is what scrolls the list.
	useEffect(() => {
		if (list.current !== null) list.current.scrollTop = list.current.scrollHeight;
	}, [lines.length]);
	return (
		<ol ref={list} className={alone ? "call-transcript call-transcript-alone" : "call-transcript"} aria-label="What was said on the call">
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

/**
 * What went wrong, in two or three words: a call's card is no place for a
 * paragraph. The whole sentence, with what to do about it, is one press of
 * the info key away; its tooltip stays a few words, as every tooltip does.
 */
function CallTrouble({ text }: { text: string }) {
	const [open, setOpen] = useState(false);
	return (
		<span className="call-trouble" role="alert">
			<span className="truncate">{troubleWords(text)}</span>
			<button type="button" className="call-trouble-info" title="What happened" aria-label={text} aria-expanded={open} onClick={() => setOpen((was) => !was)}>
				<InfoIcon />
			</button>
			{open && <span className="call-trouble-more">{text}</span>}
		</span>
	);
}

/** A call's trouble named in two or three words, from the sentence the call gave. */
export function troubleWords(text: string): string {
	if (/no words/i.test(text)) return "No words heard";
	if (/no microphone/i.test(text)) return "No microphone";
	if (/busy or not working/i.test(text)) return "Microphone busy";
	if (/microphone any more/i.test(text)) return "Microphone lost";
	if (/can't use the microphone/i.test(text)) return "Microphone blocked";
	if (/needs an update/i.test(text)) return "Desk needs update";
	if (/sign in/i.test(text)) return "Sign-in needed";
	if (/budget/i.test(text)) return "Budget spent";
	if (/reach|connect|offline/i.test(text)) return "Desk unreachable";
	return "Call failed";
}

const VOICE_BARS = 24;
/** Each bar's place across the row: the voice is tallest in the middle. */
const VOICE_SHAPE = Array.from({ length: VOICE_BARS }, (_, index) => 0.3 + 0.7 * Math.sin((Math.PI * (index + 0.5)) / VOICE_BARS));

/** One frame of the bars: dots at silence, each swaying on its own wave as the level rises. */
function voicePath(level: number, seconds: number): string {
	let path = "";
	for (let index = 0; index < VOICE_BARS; index++) {
		const sway = 0.45 + 0.55 * Math.abs(Math.sin(seconds * (5 + (index % 5)) + index * 1.7));
		const half = 0.75 + level * 8.5 * VOICE_SHAPE[index]! * sway;
		const x = 2.5 + index * 5;
		path += `M${x} ${(10 - half).toFixed(2)}V${(10 + half).toFixed(2)}`;
	}
	return path;
}

/**
 * The voice on the line, at the bottom centre of the call card: a row of
 * bars, dots at silence, that rise and sway with whoever is speaking, you or
 * them. Still while held, and with reduced motion. The path is drawn
 * straight from the call's level, without re-rendering.
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
 * clock, parted by small dots: on the call's strip, or beside the called
 * teammate's name in its own band. Whatever comes first (a name) leads.
 */
export function CallStatus({ call, roster, children }: { call: CallSession; roster: readonly RosterEntry[]; children?: ReactNode }) {
	const state = useCallSnapshot(call);
	const live = state.phase !== "ended";
	const working = callWorking(call.deskId == null || call.deskId === activeDeskId() ? roster : [], call.target?.personaId);
	return (
		<span className="call-bar-status" data-phase={state.phase}>
			{children}
			{/* Only the state is announced; the clock would be read out every second. */}
			<span aria-live="polite">{WORDS[state.phase]}</span>
			{working && live && state.phase !== "speaking" && (
				<>
					<span className="call-bar-dot" aria-hidden="true" />
					<span>Working</span>
				</>
			)}
			{live && state.phase !== "connecting" && (
				<>
					<span className="call-bar-dot" aria-hidden="true" />
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
		<span className="call-bar-clock">
			{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
		</span>
	);
}

/**
 * A call to the desk, as a card that grows up out of the rail's dock above
 * its search and call key: the handset ringed green and swelling with the
 * voice beside "Desk" and the status, then the voice bars and the
 * transcript, Hold and Hang up keys, the transcript opening above them. In
 * the faces-only rail, the ringed handset over Hang up. A plain hang-up
 * folds it away; trouble stays to be read, with Call again and Close.
 */
export function DeskCallCard({ call, roster, compact }: { call: CallSession; roster: readonly RosterEntry[]; compact: boolean }) {
	const state = useCallSnapshot(call);
	const card = useRef<HTMLElement>(null);
	const [transcript, setTranscript] = useState(false);
	useEffect(() => call.watchLevel((level) => card.current?.style.setProperty("--level", level.toFixed(3))), [call]);
	const live = state.phase !== "ended";
	const folding = !live && state.trouble === undefined;
	const speaking = state.phase === "speaking" || state.phase === "thinking";
	const mark = (
		<button
			type="button"
			className="desk-call-mark"
			title={speaking ? "Press to cut in" : undefined}
			aria-label={speaking ? "Cut in" : WORDS[state.phase]}
			aria-disabled={!speaking}
			onClick={() => {
				if (speaking) call.interrupt();
			}}
		>
			<PhoneIcon />
		</button>
	);
	const hangUp = (
		<button type="button" className="call-button call-button-end" title="Hang up" aria-label="Hang up" onClick={() => call.hangUp()}>
			<HangUpIcon />
		</button>
	);
	return (
		<section
			ref={card}
			className={compact ? "desk-call desk-call-compact" : "desk-call"}
			data-phase={state.phase}
			data-folding={folding ? "" : undefined}
			aria-label="Call with the desk"
			onAnimationEnd={(event) => {
				if (folding && event.target === event.currentTarget) closeCall();
			}}
		>
			{compact ? (
				<>
					{mark}
					{live ? hangUp : <CloseKey />}
				</>
			) : (
				<>
					{transcript && <CallTranscript lines={state.lines} name="The desk" alone={false} />}
					<div className="desk-call-top">
						{mark}
						{/* Trouble takes the status's place: the keys' row is too narrow for both. */}
						{state.trouble !== undefined ? (
							<span className="call-bar-status">
								<span className="desk-call-name">Desk</span>
								<span className="call-bar-dot" aria-hidden="true" />
								<CallTrouble text={state.trouble} />
							</span>
						) : (
							<CallStatus call={call} roster={roster}>
								<span className="desk-call-name">Desk</span>
								<span className="call-bar-dot" aria-hidden="true" />
							</CallStatus>
						)}
					</div>
					<div className="desk-call-keys">
						{live && state.phase !== "connecting" ? (
							<CallVoice call={call} still={state.phase === "held"} />
						) : (
							<span />
						)}
						<span className="min-w-0 flex-1" />
						<button
							type="button"
							className="call-button"
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
									className="call-button"
									disabled={state.phase === "connecting"}
									title={state.phase === "held" ? "Resume" : "Hold"}
									aria-label={state.phase === "held" ? "Resume" : "Hold"}
									onClick={() => call.hold(state.phase !== "held")}
								>
									{state.phase === "held" ? <PlayIcon /> : <PauseIcon />}
								</button>
								{hangUp}
							</>
						) : (
							<>
								{state.deskModel === true && <DeskModelKey call={call} />}
								<button type="button" className="call-button call-button-start" title="Call again" aria-label="Call again" onClick={() => void restartCall(call)}>
									<PhoneIcon />
								</button>
								<CloseKey />
							</>
						)}
					</div>
				</>
			)}
		</section>
	);
}

/**
 * This Mac heard a voice and no words: call again with the desk's own
 * model hearing instead, which is this Mac's choice of hearing from now
 * on, as picking it in Settings would be.
 */
function DeskModelKey({ call }: { call: CallSession }) {
	return (
		<button
			type="button"
			className="control btn"
			onClick={() => {
				setHearOnThisMac(false);
				void restartCall(call);
			}}
		>
			Use desk model
		</button>
	);
}

function CloseKey() {
	return (
		<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={closeCall}>
			<CloseIcon />
		</button>
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
