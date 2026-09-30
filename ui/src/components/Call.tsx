import { useEffect, useRef, useState } from "react";
import { chordKeys } from "../chords";
import { CloseIcon } from "../icons";
import { Band } from "../ui/Band";
import { HotlineMark } from "../ui/HotlineMark";
import { type Call as CallSession, type CallPhase, closeCall, startCall, useCallSnapshot } from "../voice/call";

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
 * A call with the desk, beside whatever conversation is open: you talk to
 * the room, not to one teammate, so the call stays up while you move
 * between them. The mark is the call — it swells with whoever is talking,
 * and pressing it while the desk speaks cuts in. What was said is kept
 * here as speech; what the teammates did lands in their own conversations.
 */
export function CallPane({
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
	const lines = useRef<HTMLDivElement>(null);

	useEffect(
		() =>
			call.watchLevel((level) => {
				stage.current?.style.setProperty("--level", level.toFixed(3));
			}),
		[call],
	);
	useEffect(() => {
		const box = lines.current;
		if (box) box.scrollTop = box.scrollHeight;
	}, [state.lines.length]);

	const live = state.phase !== "ended";
	const speaking = state.phase === "speaking" || state.phase === "thinking";
	return (
		<aside className="inspector call-pane" aria-label="Call with the desk">
			<Band>
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Desk</h2>
				<span className="instrument" aria-live="polite">
					{WORDS[state.phase]}
					{live && state.phase !== "connecting" && <> · <Clock since={state.startedAt} /></>}
				</span>
				<button type="button" className="control btn-icon" title={`Close (${chordKeys("close")})`} aria-label="Close" onClick={closeCall}>
					<CloseIcon />
				</button>
			</Band>

			<div className="call-stage">
				<button
					ref={stage}
					type="button"
					className="call-mark"
					data-phase={state.phase}
					aria-label={speaking ? "Cut in" : WORDS[state.phase]}
					disabled={!speaking}
					onClick={() => call.interrupt()}
				>
					<HotlineMark width={56} />
				</button>
				<p className="instrument call-hint">{speaking ? "Press to cut in" : state.phase === "held" ? "The desk is waiting" : " "}</p>
			</div>

			<div ref={lines} className="call-lines">
				{state.lines.map((line) =>
					line.kind === "you" ? (
						<div key={line.id} className="flex justify-end">
							<p className="speech said-me selectable">{line.text}</p>
						</div>
					) : (
						<div key={line.id}>
							{line.from !== undefined && <p className="instrument mb-1">From {line.from}</p>}
							<p className="speech said-them selectable">{line.text}</p>
						</div>
					),
				)}
			</div>

			{state.cards.length > 0 && (
				<ul className="call-cards">
					{state.cards.map((card) => (
						<li key={card.requestId} className="call-card">
							<span className="min-w-0 flex-1 truncate">{names(card.personaId) ?? "A teammate"} needs you</span>
							<button
								type="button"
								className="control btn"
								onClick={() => {
									call.dismissCard(card.requestId);
									onOpenTeammate(card.personaId);
								}}
							>
								Open
							</button>
						</li>
					))}
				</ul>
			)}

			<footer className="call-foot">
				{state.trouble !== undefined && <p className="call-trouble">{state.trouble}</p>}
				{live ? (
					<div className="flex gap-2">
						<button
							type="button"
							className="control btn"
							disabled={state.phase === "connecting"}
							onClick={() => call.hold(state.phase !== "held")}
						>
							{state.phase === "held" ? "Resume" : "Hold"}
						</button>
						<button type="button" className="control btn btn-danger" onClick={() => call.hangUp()}>
							Hang up
						</button>
					</div>
				) : (
					<button type="button" className="control btn btn-primary" onClick={() => void startCall(names)}>
						Call again
					</button>
				)}
			</footer>
		</aside>
	);
}

function Clock({ since }: { since: number }) {
	const [now, setNow] = useState(Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(timer);
	}, []);
	const seconds = Math.max(0, Math.floor((now - since) / 1000));
	return (
		<span className="tabular-nums">
			{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
		</span>
	);
}
