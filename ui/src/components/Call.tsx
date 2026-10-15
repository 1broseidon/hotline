import { useEffect, useRef, useState } from "react";
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
 * The call is a strip under the conversation's band, the band growing a
 * line the way the phone shows a call: up and out of the way. It stays
 * while the window moves between conversations (and sits at the top of a
 * pane with no conversation). It names who is on the line, the desk's toad
 * or the teammate's face, then what the call is doing and its clock, any
 * teammate that needs you, and Hold and Hang up; once the line is gone,
 * Call again and Close. The mark rings green while the line is live and
 * swells with the voice; pressing it while they speak cuts in. Work and the
 * transcript stay in the teammate's own conversation, which the name opens.
 */
export function CallBar({
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
	const working = callWorking(call.deskId == null || call.deskId === activeDeskId() ? roster : [], call.target?.personaId);
	const target = call.target;
	return (
		<aside ref={bar} className="call-bar" data-phase={state.phase} aria-label={`Call with ${target?.name ?? "the desk"}`}>
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
			<span className="instrument min-w-0 truncate">
				{target !== undefined && (
					<button type="button" className="call-bar-name" onClick={() => onOpenTeammate(target.personaId)}>
						{target.name}
					</button>
				)}
				{target !== undefined && " · "}
				{/* Only the state is announced; the clock would be read out every second. */}
				<span aria-live="polite">{WORDS[state.phase]}</span>
				{working && live && state.phase !== "speaking" && <span className="text-ink-3"> · working</span>}
				{live && state.phase !== "connecting" && <> · <Clock clock={state.clock} /></>}
			</span>

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
