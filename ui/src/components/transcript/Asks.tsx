import { useState } from "react";
import type {
	ExchangePauseStatus,
	HumanActionStatus,
	HumanAnswer,
	PasskeyAskStatus,
	PermissionOption,
	ThreadId,
	TranscriptEvent,
} from "../../generated/contract";
import { CheckIcon } from "../../icons";
import { answerCard } from "../../tape";
import { wire } from "../../wire";
import { askedFor } from "../PasskeyArm";

/**
 * A permission the agent asked. The choice is history when the tape already
 * names it; otherwise the options are the answer, and they stay dead while
 * that answer is in flight so a second click cannot race the first.
 */
export function Permission({
	thread,
	event,
}: {
	thread: ThreadId;
	event: Extract<TranscriptEvent, { kind: "permission" }>;
}) {
	const [answering, setAnswering] = useState(false);
	const chosen = chosenOption(event);

	const answer = (optionId: string) => {
		if (answering || event.decision !== undefined) return;
		setAnswering(true);
		void answerCard(thread, { kind: "permission", requestId: event.requestId, optionId }).catch(() => setAnswering(false));
	};

	return (
		<div className={`card mt-3 ${chosen === undefined ? "card-live" : ""}`}>
			<p className="eyebrow mb-1">{chosen === undefined ? "Asking permission" : "Asked permission"}</p>
			<p className="selectable font-mono text-md" style={{ whiteSpace: "pre-wrap" }}>{event.title}</p>
			{chosen !== undefined ? (
				<p className="instrument mt-1.5 flex items-center gap-1.5">
					{event.decision !== "expired" && <CheckIcon className="text-ink-4" />}
					{chosen}
				</p>
			) : (
				<div className="card-actions">
					{event.options.map((option) => (
						<button
							key={option.optionId}
							type="button"
							disabled={answering}
							className={`control ${optionKindClass(option)}`}
							onClick={() => answer(option.optionId)}
						>
							{option.name}
						</button>
					))}
				</div>
			)}
		</div>
	);
}

function chosenOption(event: Extract<TranscriptEvent, { kind: "permission" }>): string | undefined {
	if (event.decision === "expired") return "Expired unanswered";
	if (event.decidedOptionName !== undefined && event.decidedOptionName !== "") {
		return event.decidedOptionName;
	}
	if (event.decision === undefined) return undefined;
	const option = event.options.find((one) => one.optionId === event.decision);
	return option?.name ?? event.decision;
}

function optionKindClass(option: PermissionOption): string {
	return option.kind?.startsWith("allow") ? "btn-primary" : "btn";
}

const AFTERLIFE: Record<HumanActionStatus, string> = {
	pending: "Needs you",
	done: "Done",
	dismissed: "Declined",
	expired: "Expired",
	withdrawn: "Withdrawn",
};

/**
 * The agent asked for hands it does not have. A pending card is answered
 * here; a decided one is the outcome on the tape. The note goes with
 * either answer and reaches the agent word for word, so a card that asked
 * a question is answered in the same place it was asked. Enter is Done.
 */
export function HumanAction({
	thread,
	event,
	onOpenScreen,
}: {
	thread: ThreadId;
	event: Extract<TranscriptEvent, { kind: "human_action" }>;
	onOpenScreen?(): void;
}) {
	const [answering, setAnswering] = useState(false);
	const [noting, setNoting] = useState(false);
	const [note, setNote] = useState("");

	const answer = (status: HumanAnswer) => {
		if (answering || event.status !== "pending") return;
		setAnswering(true);
		const trimmed = note.trim();
		void answerCard(thread, { kind: "human", actionId: event.actionId, status, ...(trimmed ? { note: trimmed } : {}) }).catch(() => setAnswering(false));
	};

	if (event.status !== "pending") {
		return (
			<div className="card mt-3">
				<p className="eyebrow mb-1">
					Needed you · {event.question && event.status === "done" ? "Yes" : event.question && event.status === "dismissed" ? "No" : AFTERLIFE[event.status]}
					{event.note?.trim() ? <span className="normal-case"> · {event.note.trim()}</span> : null}
				</p>
				<p className="selectable text-ink-2">{event.reason}</p>
			</div>
		);
	}

	return (
		<div className="card card-live mt-3">
			<p className="eyebrow mb-1.5">Needs you</p>
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
		</div>
	);
}

const PASSKEY_AFTERLIFE: Record<PasskeyAskStatus, string> = {
	pending: "Needs you",
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
 * decided card is the outcome on the tape.
 */
export function PasskeyAskCard({ personaId, event }: { personaId: string; event: Extract<TranscriptEvent, { kind: "passkey_ask" }> }) {
	const [answering, setAnswering] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const who = askedFor(event);
	const site = event.rpName !== undefined ? `${event.rpName} (${event.rpId})` : event.rpId;
	const asks = `${site} asks to make a passkey${who !== null ? ` for ${who}` : ""}.`;

	const answer = (approved: boolean) => {
		if (answering || event.status !== "pending") return;
		setAnswering(true);
		setRefusal(null);
		void wire.command("secrets.passkey.answer", { personaId, askId: event.askId, approved }).catch((error: unknown) => {
			setRefusal(error instanceof Error ? error.message : String(error));
			setAnswering(false);
		});
	};

	if (event.status !== "pending") {
		return (
			<div className="card mt-3">
				<p className="eyebrow mb-1">Passkey · {PASSKEY_AFTERLIFE[event.status]}</p>
				<p className="selectable text-ink-2">{asks}</p>
				{event.status === "approved" && (
					<p className="selectable mt-1 text-ink-2">
						Stored as <span className="font-mono">{event.name}</span> and ticked for this teammate once made.
					</p>
				)}
			</div>
		);
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
	const [answering, setAnswering] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	if (event.status !== "pending") {
		return <p className="rule-line rule-line-plain">{EXCHANGE_SETTLED[event.status]}</p>;
	}

	const act = (cmd: "teammates.exchange_resume" | "teammates.exchange_stop") => {
		if (answering) return;
		setAnswering(true);
		setRefusal(null);
		void wire.command(cmd, { a: personaId, b: event.withPersonaId }).catch(() => {
			setAnswering(false);
			setRefusal("Could not update this exchange. Try again.");
		});
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
