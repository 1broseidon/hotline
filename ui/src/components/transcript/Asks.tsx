import { useState } from "react";
import { askKey, useAnswer } from "../../answering";
import type {
	ExchangePauseStatus,
	HumanActionStatus,
	HumanAnswer,
	PasskeyAskStatus,
	ThreadId,
	TranscriptEvent,
} from "../../generated/contract";
import { Avatar } from "../../ui/Avatar";
import { answerCard } from "../../tape";
import { wire } from "../../wire";
import { askedFor } from "../PasskeyArm";

/** The teammate a card is from, by face and name. */
export type Asker = { id: string; name: string; hash?: string | undefined };

/**
 * Any ask once it is settled, a permission, a sign-in, a question: one quiet
 * line with how it ended in a chip, what was asked, and who asked. What is
 * done is history, and history should not take a card's room.
 */
export function SettledAsk({ state, what, command = false, who }: { state: string; what: string; command?: boolean; who: string }) {
	return (
		<p className="ask-settled">
			<span className="link-row-dot" data-tone={/allow|yes|done|signed|stored|used/i.test(state) ? "done" : /den|refus|fail/i.test(state) ? "failed" : undefined} />
			{command ? <code className="ask-settled-what selectable">{what}</code> : <span className="ask-settled-what selectable">{what}</span>}
			<span className="ask-settled-who">{who}</span>
			<span className="ask-chip">{state}</span>
		</p>
	);
}

/** A command as the well shows it: without the backticks some harnesses fence it in. */
function commandOf(title: string): string {
	return title.trim().replace(/^`+|`+$/g, "");
}

/**
 * A permission the agent asked: who wants to run what, the command in a
 * well, and the agent's own answers side by side at one weight, none of them
 * picked for you. They stay dead while an answer is in flight so a second
 * click cannot race the first. Answered, it folds to one line.
 */
export function Permission({
	thread,
	event,
	who,
}: {
	thread: ThreadId;
	event: Extract<TranscriptEvent, { kind: "permission" }>;
	who: Asker;
}) {
	const chosen = chosenOption(event);
	const { answering, failure, answer: send } = useAnswer(askKey(event), event.decision !== undefined);

	const answer = (optionId: string) => {
		send(() => answerCard(thread, { kind: "permission", requestId: event.requestId, optionId }));
	};

	if (chosen !== undefined) return <SettledAsk state={chosen} what={commandOf(event.title)} command who={who.name} />;
	return (
		<div className="ask-card">
			<p className="ask-head">
				<Avatar id={who.id} name={who.name} size={18} hash={who.hash} />
				<span className="line-label">
					<b>{who.name}</b> wants to run
				</span>
			</p>
			<p className="ask-command">
				<span className="ask-prompt" aria-hidden="true">
					$
				</span>
				<span className="selectable">{commandOf(event.title)}</span>
			</p>
			<div className="card-actions">
				{event.options.map((option) => (
					<button key={option.optionId} type="button" disabled={answering} className="control btn" onClick={() => answer(option.optionId)}>
						{option.name}
					</button>
				))}
			</div>
			{failure !== null && <p role="alert" className="mt-2 text-sm text-danger">{failure}</p>}
		</div>
	);
}

function chosenOption(event: Extract<TranscriptEvent, { kind: "permission" }>): string | undefined {
	if (event.decision === "expired") return "Expired";
	if (event.decidedOptionName !== undefined && event.decidedOptionName !== "") {
		return event.decidedOptionName;
	}
	if (event.decision === undefined) return undefined;
	const option = event.options.find((one) => one.optionId === event.decision);
	return option?.name ?? event.decision;
}

const AFTERLIFE: Record<Exclude<HumanActionStatus, "pending">, string> = {
	done: "Done",
	dismissed: "Declined",
	expired: "Expired",
	withdrawn: "Withdrawn",
};

/**
 * The agent asked for hands it does not have: the amber Needs you card, with
 * the face and name of who is asking. A pending card is answered here; a
 * decided one folds to its outcome on one line. The note goes with either
 * answer and reaches the agent word for word, so a card that asked a
 * question is answered in the same place it was asked. Enter is Done.
 */
export function HumanAction({
	thread,
	event,
	who,
	onOpenScreen,
}: {
	thread: ThreadId;
	event: Extract<TranscriptEvent, { kind: "human_action" }>;
	who: Asker;
	onOpenScreen?(): void;
}) {
	const [noting, setNoting] = useState(false);
	const [note, setNote] = useState("");
	const { answering, failure, answer: send } = useAnswer(askKey(event), event.status !== "pending");

	const answer = (status: HumanAnswer) => {
		const trimmed = note.trim();
		send(() => answerCard(thread, { kind: "human", actionId: event.actionId, status, ...(trimmed ? { note: trimmed } : {}) }));
	};

	if (event.status !== "pending") {
		const state = event.question && event.status === "done" ? "Yes" : event.question && event.status === "dismissed" ? "No" : AFTERLIFE[event.status];
		const note = event.note?.trim();
		return <SettledAsk state={state} what={note ? `${event.reason} · ${note}` : event.reason} who={who.name} />;
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1.5 needs-you-head">
				<Avatar id={who.id} name={who.name} size={18} hash={who.hash} />
				<span>
					Needs you · <b>{who.name}</b>
				</span>
			</p>
			<p className="selectable text-lg font-medium">{event.reason}</p>
			{onOpenScreen !== undefined && !event.question && (
				<p className="mt-1.5 text-sm text-ink-3">
					The screen is yours until you press Done. What you type there goes to the desktop, not to the teammate.
				</p>
			)}
			{noting && (
				<input
					className="field mt-2.5 w-full"
					aria-label="A note for the teammate"
					placeholder="A note for the teammate"
					autoComplete="off"
					// biome-ignore lint/a11y/noAutofocus: the field opens on a press of Add a note, which wants a note typed next.
					autoFocus
					value={note}
					onChange={(change) => setNote(change.target.value)}
					onKeyDown={(key) => {
						if (key.key !== "Enter") return;
						key.preventDefault();
						answer("done");
					}}
				/>
			)}
			<div className="mt-2.5 flex flex-wrap items-center gap-1.5">
				{onOpenScreen !== undefined && !event.question && (
					<button type="button" className="control btn-primary" onClick={onOpenScreen}>
						Open the screen
					</button>
				)}
				<button
					type="button"
					disabled={answering}
					className={onOpenScreen !== undefined && !event.question ? "control btn" : "control btn-primary"}
					onClick={() => answer("done")}
				>
					{event.question ? "Yes" : "Done"}
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => answer("declined")}>
					{event.question ? "No" : "Decline"}
				</button>
				{!noting && (
					<button type="button" className="control btn-quiet ml-auto" onClick={() => setNoting(true)}>
						Add a note
					</button>
				)}
			</div>
			{failure !== null && <p role="alert" className="mt-2 text-sm text-danger">{failure}</p>}
		</div>
	);
}

const PASSKEY_AFTERLIFE: Record<Exclude<PasskeyAskStatus, "pending">, string> = {
	approved: "Approved",
	denied: "Denied",
	expired: "Expired",
};

/**
 * A site asked the teammate's browser to make a passkey, under an arming
 * the person started in the teammate's pane; the request waits in the
 * browser until it is answered here, from whichever seat. Approve and the
 * browser makes it, the room stores it under the arming's name and ticks
 * it for this teammate; deny and the site hears no and the arming ends. A
 * decided card folds to its outcome on one line.
 */
export function PasskeyAskCard({ personaId, event, teammate }: { personaId: string; event: Extract<TranscriptEvent, { kind: "passkey_ask" }>; teammate: string }) {
	const { answering, failure: refusal, answer: send } = useAnswer(askKey(event), event.status !== "pending");
	const who = askedFor(event);
	const site = event.rpName !== undefined ? `${event.rpName} (${event.rpId})` : event.rpId;
	const asks = `${site} asks to make a passkey${who !== null ? ` for ${who}` : ""}.`;

	const answer = (approved: boolean) => {
		send(() => wire.command("secrets.passkey.answer", { personaId, askId: event.askId, approved }));
	};

	if (event.status !== "pending") {
		return <SettledAsk state={PASSKEY_AFTERLIFE[event.status]} what={event.status === "approved" ? `Passkey for ${site}, stored as ${event.name}` : `Passkey for ${site}`} who={teammate} />;
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Needs you · Passkey</p>
			<p className="selectable">{asks}</p>
			<p className="mt-1.5 text-sm text-ink-3">
				The request came from {event.origin}. Approved, the passkey is stored as <span className="font-mono">{event.name}</span> and
				ticked for this teammate, whose browser signs in there with it from now on. Denied, the site hears no and the arming ends.
			</p>
			<div className="mt-2.5 flex flex-wrap items-center gap-1.5">
				<button type="button" disabled={answering} className="control btn-primary" onClick={() => answer(true)}>
					Approve
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => answer(false)}>
					Deny
				</button>
			</div>
			{refusal !== null && <p className="mt-2 text-sm text-danger">{refusal}</p>}
		</div>
	);
}

const EXCHANGE_SETTLED: Record<Exclude<ExchangePauseStatus, "pending">, string> = {
	resumed: "Exchange resumed",
	stopped: "Exchange stopped",
};

/**
 * Both asks and handoffs count toward the pair's message cap. Keep going
 * releases queued work and starts counting afresh; Stop exchange ends the
 * exchange, not the collaboration grant. Either outcome becomes a quiet line.
 */
export function ExchangePaused({
	personaId,
	ownerName,
	event,
}: {
	personaId: string;
	event: Extract<TranscriptEvent, { kind: "exchange_paused" }>;
	ownerName: string;
}) {
	const { answering, failure: refusal, answer: send } = useAnswer(
		askKey(event),
		event.status !== "pending",
		() => "Could not update this exchange. Try again.",
	);

	if (event.status !== "pending") {
		return <p className="rule-line rule-line-plain">{EXCHANGE_SETTLED[event.status]}</p>;
	}

	const act = (cmd: "teammates.exchange_resume" | "teammates.exchange_stop") => {
		send(() => wire.command(cmd, { a: personaId, b: event.withPersonaId }));
	};

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1">Paused</p>
			<p className="selectable">
				{ownerName} and {event.withName} paused after {event.exchanges} {event.exchanges === 1 ? "message" : "messages"} without you.
			</p>
			<div className="card-actions">
				<button type="button" disabled={answering} className="control btn-primary" onClick={() => act("teammates.exchange_resume")}>
					Keep going
				</button>
				<button type="button" disabled={answering} className="control btn" onClick={() => act("teammates.exchange_stop")}>
					Stop exchange
				</button>
			</div>
			{refusal !== null && <p role="alert" className="mt-2 text-sm text-danger">{refusal}</p>}
		</div>
	);
}
