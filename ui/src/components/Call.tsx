import { useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon, HangUpIcon, PauseIcon, PhoneIcon, PlayIcon } from "../icons";
import { HotlineMark } from "../ui/HotlineMark";
import { Avatar } from "../ui/Avatar";
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
 * The chosen desk or teammate stays on the line while the window moves
 * between conversations. Pressing the face while they speak cuts in;
 * work and the transcript stay in the teammate's own conversation.
 */
export function CallFloat({
	call,
	names,
	onOpenTeammate,
}: {
	call: CallSession;
	names: (personaId: string) => string | undefined;
	onOpenTeammate(personaId: string): void;
}) {
	const state = useCallSnapshot(call);
	const stage = useRef<HTMLButtonElement>(null);
	const again = useRef<HTMLButtonElement>(null);

	useEffect(
		() =>
			call.watchLevel((level) => {
				stage.current?.style.setProperty("--level", level.toFixed(3));
			}),
		[call],
	);

	const live = state.phase !== "ended";
	// When the line goes, the next thing to press is Call again.
	useEffect(() => {
		if (!live) again.current?.focus();
	}, [live]);
	const speaking = state.phase === "speaking" || state.phase === "thinking";
	return (
		<aside className="call-float" aria-label={`Call with ${call.target?.name ?? "the desk"}`}>
			<div className="call-top">
				<span className="instrument min-w-0 flex-1 truncate">
					{call.target !== undefined && <>{call.target.name} · </>}
					{/* Only the state is announced; the clock would be read out every second. */}
					<span aria-live="polite">{WORDS[state.phase]}</span>
					{live && state.phase !== "connecting" && <> · <Clock clock={state.clock} /></>}
				</span>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={closeCall}>
					<CloseIcon />
				</button>
			</div>

			<button
				ref={stage}
				type="button"
				className="call-mark"
				data-phase={state.phase}
				title={speaking ? "Press to cut in" : undefined}
				aria-label={speaking ? "Cut in" : WORDS[state.phase]}
				// aria-disabled rather than disabled, so focus stays put when the desk stops talking.
				aria-disabled={!speaking}
				onClick={() => {
					if (speaking) call.interrupt();
				}}
			>
				{call.target === undefined ? <HotlineMark width={44} plain /> :
					<Avatar id={call.target.personaId} name={call.target.name} hash={call.target.avatarHash} size={44} read={call.readAvatar} />}
			</button>

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
				<p className="call-trouble" role="alert">
					{state.trouble}
				</p>
			)}

			{live ? (
				<div className="call-controls">
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
				</div>
			) : (
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
			)}
		</aside>
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
		<span className="tabular-nums">
			{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
		</span>
	);
}
