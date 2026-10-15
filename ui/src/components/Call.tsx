import { type ReactNode, useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, HangUpIcon, PauseIcon, PhoneIcon, PlayIcon } from "../icons";
import { Avatar } from "../ui/Avatar";
import { CallToad } from "./CallToad";
import { callWorking } from "./CallStage";
import { activeDeskId } from "../desks";
import type { RosterEntry } from "../wire";
import { type Call as CallSession, type CallPhase, closeCall, restartCall, useCallSnapshot } from "../voice/call";

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
				{target === undefined ? <CallToad call={call} phase={state.phase} width={24} /> :
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

			{state.trouble !== undefined && (
				<span className="call-trouble min-w-0 truncate" role="alert" title={state.trouble}>
					{state.trouble}
				</span>
			)}

			<span className="min-w-0 flex-1" />

			{live && state.phase !== "connecting" && <CallVoice call={call} still={state.phase === "held"} />}

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
	);
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
