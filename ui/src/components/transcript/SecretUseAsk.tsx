import { useEffect, useState } from "react";
import type { SecretUseAnswer, SecretUseStatus, TranscriptEvent } from "../../generated/contract";
import { askKey, useAnswer } from "../../answering";
import { wire } from "../../wire";
import { SettledAsk } from "./Asks";

const outcomes: Record<SecretUseStatus, string> = {
	pending: "Needs you", approved: "Approved", allowed: "Allowed for 10 min", denied: "Denied", expired: "Expired",
};

export function SecretUseAsk({ event, teammate }: { event: Extract<TranscriptEvent, { kind: "secret_use_ask" }>; teammate: string }) {
	const { answering, failure: refusal, answer: send } = useAnswer(askKey(event), event.status !== "pending");
	const [expired, setExpired] = useState(Date.now() >= event.expiresAt);
	useEffect(() => {
		setExpired(Date.now() >= event.expiresAt);
		const timer = setTimeout(() => setExpired(true), Math.max(0, event.expiresAt - Date.now()));
		return () => clearTimeout(timer);
	}, [event.expiresAt]);
	const status = event.status === "pending" && expired ? "expired" : event.status;
	const answer = (answer: SecretUseAnswer) => {
		if (status !== "pending" || Date.now() >= event.expiresAt) return;
		send(() => wire.command("secrets.use.answer", { personaId: event.teammate, useId: event.useId, answer }));
	};
	const question = asked(event);
	// A split computer says what each use is; ten minutes there cover only a
	// sign-in its own browser is doing. An older computer's card is as it was.
	const oneUse = event.operation !== undefined && event.attested !== true;
	const covers = event.operation === undefined ? "Allow 10 min covers this key, this site and this teammate." : "Allow 10 min covers this sign-in only.";
	// Settled, it folds to one line, as every ask does.
	if (status !== "pending") return <SettledAsk state={outcomes[status]} what={question.line} who={teammate} />;
	return <div className="card card-live mt-3">
		<p className="eyebrow mb-1">{outcomes[status]} · {event.secretKind === "passkey" ? "Passkey" : "Login"}</p>
		<p className="selectable">{question.before}<span className="font-mono">{event.secret}</span>{question.after}</p>
		<p className="mt-1 text-sm text-ink-3">For {teammate} · {event.site}{event.path}</p>
		<p className="mt-1.5 text-sm text-ink-3">{oneUse ? "Not asked by Hotline's own browser, so this is for one use." : covers} Voice cannot answer this card.</p>
		<div className="mt-2.5 flex flex-wrap gap-1.5">
			<button type="button" className="control btn-primary" disabled={answering} onClick={() => answer("approve")}>Approve</button>
			{!oneUse && <button type="button" className="control btn" disabled={answering} onClick={() => answer("allow")}>Allow 10 min</button>}
			<button type="button" className="control btn" disabled={answering} onClick={() => answer("deny")}>Deny</button>
		</div>
		{refusal !== null && <p className="mt-2 text-sm text-danger">{refusal}</p>}
	</div>;
}

/**
 * What the card asks, bound to the one operation it is for: a sign-in as
 * an account on a site, or a passkey sign-in. The desk browser's own card
 * asks the same words (`use_question` in the core). A card from a computer
 * that cannot say what the use is names the key and the site.
 */
function asked(event: Extract<TranscriptEvent, { kind: "secret_use_ask" }>): { before: string; after: string; line: string } {
	const operation = event.operation;
	if (operation === undefined) return { before: "Use ", after: ` on ${event.site}?`, line: `Use ${event.secret} on ${event.site}${event.path}` };
	const site = operation.kind === "passkey" ? (operation.rpId ?? hostOf(operation.origin)) : hostOf(operation.origin);
	const as = operation.account !== "" ? ` as ${operation.account}` : "";
	const before = `Sign in to ${site}${as} with ${operation.kind === "passkey" ? "the passkey " : ""}`;
	return { before, after: "?", line: `${before}${event.secret}` };
}

function hostOf(origin: string): string {
	try {
		return new URL(origin).host;
	} catch {
		return origin;
	}
}
