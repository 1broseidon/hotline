import { useLayoutEffect, useRef, useState } from "react";
import type { SessionState } from "../generated/contract";

/** The field stops growing here, and scrolls from then on. */
const MAX_HEIGHT = 200;

/** A session that is between turns and can be spoken to right now. */
export function isWorking(state: SessionState): boolean {
	return state === "starting" || state === "thinking";
}

/** A session that has to be started before anything can be said to it. */
function isDown(state: SessionState): boolean {
	return state === "idle" || state === "stopped" || state === "error";
}

/**
 * Where you type.
 *
 * A stopped teammate is started by talking to it: a message typed at a session
 * that is not running starts one and then says the message, because the person
 * meant to send it either way. The Start button is for the other case — waking
 * a teammate with nothing to say yet. A reply being composed sits as a
 * one-line quote above the field; Escape puts that quote down before it
 * interrupts a turn.
 */
export function Composer({
	personaId,
	state,
	replyQuote,
	onSend,
	onStart,
	onCancel,
	onClearReply,
}: {
	personaId: string;
	state: SessionState;
	replyQuote: string | null;
	onSend(text: string): void;
	onStart(): void;
	onCancel(): void;
	onClearReply(): void;
}) {
	const [text, setText] = useState("");
	const area = useRef<HTMLTextAreaElement>(null);
	const working = isWorking(state);
	const down = isDown(state);

	// Grow with content, up to a ceiling. Before paint, because measuring after
	// it draws a wrapped line at the old height for one frame first.
	useLayoutEffect(() => {
		const el = area.current;
		if (!el) return;
		el.style.height = "auto";
		el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
	}, [text, personaId]);

	// The quote is a decision just made: the field is where the answer goes.
	useLayoutEffect(() => {
		if (replyQuote !== null) area.current?.focus();
	}, [replyQuote]);

	const submit = () => {
		const trimmed = text.trim();
		if (!trimmed) return;
		if (down) onStart();
		setText("");
		onSend(trimmed);
	};

	return (
		<div className="border-t border-rule bg-paper px-6 py-3">
			<div className="mx-auto w-full max-w-[46rem]">
				{replyQuote !== null && (
					<div className="reply-chip">
						<p className="reply-chip-quote">{replyQuote}</p>
						<button
							type="button"
							className="reply-chip-clear"
							aria-label="Stop replying"
							onClick={onClearReply}
						>
							×
						</button>
					</div>
				)}
				<div className="flex items-end gap-2">
					<textarea
						ref={area}
						rows={1}
						value={text}
						aria-label="Message your teammate"
						placeholder={down ? "Message — sending starts the session" : "Message"}
						className="field resize-none"
						onChange={(event) => setText(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && !event.shiftKey) {
								event.preventDefault();
								submit();
								return;
							}
							// The quote is something you put down, so Escape puts it
							// down before it interrupts a turn.
							if (event.key === "Escape" && replyQuote !== null) {
								event.preventDefault();
								onClearReply();
								return;
							}
							// Interrupting with nothing to say is still just Escape,
							// whatever is sitting half-written in the field.
							if (event.key === "Escape" && working) {
								event.preventDefault();
								onCancel();
							}
						}}
					/>

					{working ? (
						<button type="button" className="btn-quiet" title="Interrupt (Esc)" onClick={onCancel}>
							Stop
						</button>
					) : down && text.trim().length === 0 ? (
						<button type="button" className="btn-quiet" onClick={onStart}>
							Start
						</button>
					) : (
						<button
							type="button"
							className="btn-primary"
							title="Send (Enter)"
							disabled={text.trim().length === 0}
							onClick={submit}
						>
							Send
						</button>
					)}
				</div>
			</div>
		</div>
	);
}
