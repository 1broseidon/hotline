import { useEffect, useRef, useState } from "react";
import type { CredentialAnswer, TranscriptEvent } from "../../generated/contract";
import { LockIcon } from "../../icons";
import { askKey, useAnswer } from "../../answering";
import { wire } from "../../wire";
import { SettledAsk } from "./Asks";

type Ask = Extract<TranscriptEvent, { kind: "credential_ask" }>;

/** "github.com" from "https://github.com/": the site as a person names it. */
const siteName = (site: string): string => site.replace(/^https?:\/\//, "").replace(/\/$/, "");

const SETTLED: Record<Exclude<Ask["status"], "pending">, string> = { stored: "Stored", declined: "Declined", expired: "Expired", withdrawn: "Withdrawn" };

/**
 * A teammate asks to sign in somewhere: a lock, the site, why, and one
 * secure key that opens the fields. Values stay in this form's DOM until the
 * one capture command takes them. Settled, it folds to one line.
 */
export function CredentialAsk({ event, teammate }: { event: Ask; teammate: string }) {
	const [expired, setExpired] = useState(false);
	useEffect(() => {
		setExpired(false);
		if (event.status !== "pending") return;
		const timer = setTimeout(() => setExpired(true), Math.max(0, event.expiresAt - Date.now()));
		return () => clearTimeout(timer);
	}, [event.expiresAt, event.status]);
	const status = event.status === "pending" && expired ? "expired" : event.status;
	if (status !== "pending") {
		const asked = event.credentialKind === "passkey" ? `Passkey for ${siteName(event.site)}` : `Sign in to ${siteName(event.site)}`;
		return <SettledAsk state={SETTLED[status]} what={status === "stored" ? `${asked} · ${event.storedAs ?? event.name}` : asked} who={teammate} />;
	}
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
	const [open, setOpen] = useState(false);
	const login = event.credentialKind === "login";
	const { answering, failure: error, answer: sendAnswer } = useAnswer(
		askKey(event),
		event.status !== "pending",
		() => "Could not save this answer. Check whether the card is still waiting, then enter the fields again.",
	);
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
		sendAnswer(() => wire.command("secrets.capture.answer", answer));
	};

	return (
		<form ref={form} className="ask-card" autoComplete="off" onSubmit={(event) => { event.preventDefault(); send("save"); }}>
			<div className="sign-in-head">
				<span className="sign-in-lock"><LockIcon /></span>
				<span className="min-w-0">
					<span className="line-label block"><b>{teammate}</b> {login ? "asks to sign in" : "asks for a passkey"}</span>
					<span className="sign-in-site selectable">{siteName(event.site)}</span>
				</span>
			</div>
			<p className="sign-in-why selectable">{event.reason}</p>
			{open && <>
				<label className="block mt-2">Name<input aria-label="Key name" className="field w-full" name="name" defaultValue={event.name} maxLength={64} required pattern="[A-Z][A-Z0-9_]*" disabled={answering} /></label>
				<label className="flex items-center gap-2 mt-2"><input type="checkbox" name="ask" disabled={answering} />Ask me</label>
				{login && <>
					<label className="block mt-2">Username<input aria-label="Username" className="field w-full" name="username" autoComplete="off" spellCheck={false} required disabled={answering} /></label>
					<label className="block mt-2">Password<input aria-label="Password" className="field w-full" name="password" type="password" autoComplete="new-password" required minLength={8} disabled={answering} /></label>
					<label className="block mt-2">One-time-code seed (optional)<input aria-label="One-time-code seed" className="field w-full" name="totp" type="password" autoComplete="new-password" disabled={answering} /></label>
				</>}
				{event.existing.length > 0 && <p className="mt-1.5 text-sm text-ink-3">Existing keys keep their current Ask me setting.</p>}
			</>}
			{login && <p className="sign-in-note">Goes to the vault and {teammate}'s computer. The chat and teammate receive only the key's name.</p>}
			<div className="card-actions">
				{open ? (
					<button type="submit" className="control btn-primary" disabled={answering}>{login ? "Save" : "Make a passkey"}</button>
				) : (
					<button type="button" className="control btn-primary" disabled={answering} onClick={() => setOpen(true)}>{login ? "Sign in securely" : "Make a passkey"}</button>
				)}
				{event.existing.map(name => <button key={name} type="button" className="control btn" disabled={answering} onClick={() => send(name)}>Grant {name} instead</button>)}
				<button type="button" className="control btn-quiet ml-auto" disabled={answering} onClick={() => send("decline")}>Not now</button>
			</div>
			{error && <p role="alert" className="mt-2 text-sm text-danger">{error}</p>}
		</form>
	);
}
