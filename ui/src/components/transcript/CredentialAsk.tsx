import { useEffect, useRef, useState } from "react";
import type { CredentialAnswer, TranscriptEvent } from "../../generated/contract";
import { CheckIcon } from "../../icons";
import { wire } from "../../wire";

type Ask = Extract<TranscriptEvent, { kind: "credential_ask" }>;

/** Values stay in this form's DOM until the one capture command takes them. */
export function CredentialAsk({ event, teammate }: { event: Ask; teammate: string }) {
	const [expired, setExpired] = useState(false);
	useEffect(() => {
		setExpired(false);
		if (event.status !== "pending") return;
		const timer = setTimeout(() => setExpired(true), Math.max(0, event.expiresAt - Date.now()));
		return () => clearTimeout(timer);
	}, [event.expiresAt, event.status]);
	const status = event.status === "pending" && expired ? "expired" : event.status;
	// Settled, it reads as a permission does: what was asked, of what, and how it ended.
	if (status !== "pending") return (
		<div className="card mt-3">
			<p className="eyebrow mb-1">{event.credentialKind === "passkey" ? "Asked for a passkey" : "Asked to sign in"}</p>
			<p className="selectable font-mono text-md">{event.site.replace(/^https?:\/\//, "").replace(/\/$/, "")}</p>
			<p className="mt-1.5 flex items-center gap-1.5 text-sm text-ink-3">
				{status === "stored" && <CheckIcon className="text-ink-4" />}
				{status === "stored" ? `Stored as ${event.storedAs ?? event.name}` : status === "expired" ? "Expired unanswered" : status.charAt(0).toUpperCase() + status.slice(1)}
				{` · ${teammate}`}
			</p>
		</div>
	);
	if (event.armed) return (
		<div className="card mt-3">
			<p className="eyebrow mb-1">Ready to make a passkey · {event.name}</p>
			<p className="selectable">{event.site} · {teammate}</p>
			<p className="mt-1.5 text-sm text-ink-3">Add a passkey on the site, then approve its separate request here.</p>
		</div>
	);
	return <CaptureForm key={event.actionId} event={event} teammate={teammate} />;
}

function CaptureForm({ event, teammate }: { event: Ask; teammate: string }) {
	const form = useRef<HTMLFormElement>(null);
	const [answering, setAnswering] = useState(false);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		const fields = form.current;
		return () => fields?.reset();
	}, []);

	const send = (choice: "save" | "decline" | string) => {
		if (answering || !form.current) return;
		if (Date.now() >= event.expiresAt) {
			form.current.reset();
			return;
		}
		const fields = new window.FormData(form.current);
		const answer: CredentialAnswer = { actionId: event.actionId, ask: fields.get("ask") === "on" };
		if (choice !== "decline") {
			answer.name = choice === "save" ? String(fields.get("name") ?? "").trim() : choice;
			if (choice === "save" && event.credentialKind === "login") {
				const totp = String(fields.get("totp") ?? "");
				answer.values = { username: String(fields.get("username") ?? ""), password: String(fields.get("password") ?? ""), ...(totp ? { totp } : {}) };
			}
		}
		form.current.reset();
		setAnswering(true);
		setError(null);
		void wire.command("secrets.capture.answer", answer).catch(() => {
			setAnswering(false);
			setError("Could not save this answer. Check whether the card is still waiting, then enter the fields again.");
		});
	};

	return (
		<form ref={form} className="card card-live mt-3" autoComplete="off" onSubmit={(event) => { event.preventDefault(); send("save"); }}>
			<p className="eyebrow mb-1">Needs you · {event.credentialKind === "login" ? "Login" : "Passkey"} for {teammate}</p>
			<p className="selectable font-mono">{event.site}</p>
			<p className="selectable mt-1">{event.reason}</p>
			<label className="block mt-2">Name<input aria-label="Key name" className="field w-full" name="name" defaultValue={event.name} maxLength={64} required pattern="[A-Z][A-Z0-9_]*" disabled={answering} /></label>
			<label className="flex items-center gap-2 mt-2"><input type="checkbox" name="ask" disabled={answering} />Ask me</label>
			{event.credentialKind === "login" && <>
				<label className="block mt-2">Username<input aria-label="Username" className="field w-full" name="username" autoComplete="off" spellCheck={false} required disabled={answering} /></label>
				<label className="block mt-2">Password<input aria-label="Password" className="field w-full" name="password" type="password" autoComplete="new-password" required minLength={8} disabled={answering} /></label>
				<label className="block mt-2">One-time-code seed (optional)<input aria-label="One-time-code seed" className="field w-full" name="totp" type="password" autoComplete="new-password" disabled={answering} /></label>
				<p className="mt-1.5 text-sm text-ink-3">Goes to the vault and {teammate}'s computer. The chat and teammate receive only the key's name.</p>
			</>}
			{event.existing.length > 0 && <p className="mt-1.5 text-sm text-ink-3">Existing keys keep their current Ask me setting.</p>}
			<div className="card-actions">
				<button type="submit" className="control btn-primary" disabled={answering}>{event.credentialKind === "login" ? "Save" : "Make a passkey"}</button>
				<button type="button" className="control btn" disabled={answering} onClick={() => send("decline")}>Decline</button>
				{event.existing.map(name => <button key={name} type="button" className="control btn" disabled={answering} onClick={() => send(name)}>Grant {name} instead</button>)}
			</div>
			{error && <p role="alert" className="mt-2 text-sm text-danger">{error}</p>}
		</form>
	);
}
