import { useEffect, useState } from "react";
import type { SecretUseAnswer, SecretUseStatus, TranscriptEvent } from "../../generated/contract";
import { wire } from "../../wire";

const outcomes: Record<SecretUseStatus, string> = {
	pending: "Needs you", approved: "Approved", allowed: "Allowed for 10 min", denied: "Denied", expired: "Expired",
};

export function SecretUseAsk({ event, teammate }: { event: Extract<TranscriptEvent, { kind: "secret_use_ask" }>; teammate: string }) {
	const [answering, setAnswering] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [expired, setExpired] = useState(Date.now() >= event.expiresAt);
	useEffect(() => {
		setExpired(Date.now() >= event.expiresAt);
		const timer = setTimeout(() => setExpired(true), Math.max(0, event.expiresAt - Date.now()));
		return () => clearTimeout(timer);
	}, [event.expiresAt]);
	const status = event.status === "pending" && expired ? "expired" : event.status;
	const answer = (answer: SecretUseAnswer) => {
		if (answering || status !== "pending" || Date.now() >= event.expiresAt) return;
		setAnswering(true);
		setRefusal(null);
		void wire.command("secrets.use.answer", { personaId: event.teammate, useId: event.useId, answer }).catch((error: unknown) => {
			setRefusal(error instanceof Error ? error.message : String(error));
			setAnswering(false);
		});
	};
	return <div className={`card mt-3${status === "pending" ? " card-live" : ""}`}>
		<p className="eyebrow mb-1">{outcomes[status]} · {event.secretKind === "passkey" ? "Passkey" : "Login"}</p>
		<p className="selectable">Use <span className="font-mono">{event.secret}</span> on {event.site} for {teammate}?</p>
		<p className="mt-1 text-sm text-ink-3">{event.site}{event.path}</p>
		{status === "pending" && <>
			<p className="mt-1.5 text-sm text-ink-3">Allow 10 min covers this key, this site and this teammate. Voice cannot answer this card.</p>
			<div className="mt-2.5 flex flex-wrap gap-1.5">
				<button type="button" className="control btn-primary" disabled={answering} onClick={() => answer("approve")}>Approve</button>
				<button type="button" className="control btn" disabled={answering} onClick={() => answer("allow")}>Allow 10 min</button>
				<button type="button" className="control btn" disabled={answering} onClick={() => answer("deny")}>Deny</button>
			</div>
		</>}
		{refusal !== null && status === "pending" && <p className="mt-2 text-sm text-danger">{refusal}</p>}
	</div>;
}
