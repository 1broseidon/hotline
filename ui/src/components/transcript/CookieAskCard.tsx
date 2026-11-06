import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { CookieOfferAnswer, CookieOfferSource, CredentialAnswer, TranscriptEvent } from "../../generated/contract";
import { CloseIcon, LockIcon } from "../../icons";
import { askKey, useAnswer } from "../../answering";
import { isDesktop } from "../../native";
import { wire } from "../../wire";
import { SettledAsk } from "./Asks";

type Ask = Extract<TranscriptEvent, { kind: "cookie_ask" }>;

/** "Brave", or "Brave · Work" for a profile other than the browser's first. */
function sourceName(source: CookieOfferSource): string {
	return source.profileName === "Default" ? source.browserName : `${source.browserName} · ${source.profileName}`;
}

/** How a settled ask ended, as its one line says it. */
function settledState(event: Ask): string {
	switch (event.status) {
		case "cookies":
			return `Cookies from ${event.browserName ?? "your browser"}`;
		case "signed_in":
			return "Signed in";
		case "opened":
			return "Browser opened";
		case "not_now":
			return "Not now";
		case "withdrawn":
			return "Withdrawn";
		default:
			return "Expired";
	}
}

/**
 * A teammate asks to be signed in to one site: a lock, the site, and why in
 * its words. Use Cookies copies the site's cookies from the browser named
 * under it, Sign In opens secure fields in this card whose values go only
 * to the vault, and Open Browser hands the person the teammate's own
 * browser. The corner's close is Not now. Settled, it folds to one line.
 */
export function CookieAskCard({
	event,
	personaId,
	teammate,
	onOpenScreen,
}: {
	event: Ask;
	personaId: string;
	teammate: string;
	onOpenScreen?: (() => void) | undefined;
}) {
	const [expired, setExpired] = useState(false);
	useEffect(() => {
		setExpired(false);
		if (event.status !== "pending") return;
		const timer = setTimeout(() => setExpired(true), Math.max(0, event.expiresAt - Date.now()));
		return () => clearTimeout(timer);
	}, [event.expiresAt, event.status]);
	if (event.status !== "pending" || expired) return <SettledAsk what={event.site} state={expired ? "Expired" : settledState(event)} />;
	return <Pending key={event.id} event={event} personaId={personaId} teammate={teammate} onOpenScreen={onOpenScreen} />;
}

function Pending({ event, personaId, teammate, onOpenScreen }: { event: Ask; personaId: string; teammate: string; onOpenScreen?: (() => void) | undefined }) {
	const form = useRef<HTMLFormElement>(null);
	const [signingIn, setSigningIn] = useState(false);
	const [save, setSave] = useState(true);
	const [source, setSource] = useState(0);
	const { answering, failure, answer: send } = useAnswer(askKey(event), false, (error) => (error instanceof Error ? error.message : String(error)));
	useEffect(() => {
		const fields = form.current;
		return () => fields?.reset();
	}, []);

	const choose = (answer: CookieOfferAnswer, then?: () => void) => {
		const chosen = answer === "use" ? event.sources[source] : undefined;
		send(async () => {
			await wire.command("computer.cookies.answer", { personaId, id: event.id, answer, ...(chosen !== undefined ? { source: chosen } : {}) });
			then?.();
		});
	};
	// The teammate's own browser, where the person is: its pane on the desk,
	// or its computer's screen in a window of its own.
	const openBrowser = () => {
		if (event.deskBrowser === true) {
			if (isDesktop()) void invoke("browser_pane_open", { personaId, name: teammate }).catch(() => {});
		} else onOpenScreen?.();
	};
	const signIn = () => {
		if (answering || !form.current) return;
		const fields = new window.FormData(form.current);
		const answer: CredentialAnswer = {
			actionId: event.id,
			name: save ? String(fields.get("name") ?? "").trim() : event.name,
			values: { username: String(fields.get("username") ?? ""), password: String(fields.get("password") ?? "") },
			ask: false,
			save,
		};
		form.current.reset();
		send(() => wire.command("secrets.capture.answer", answer));
	};

	return (
		<form ref={form} className="ask-card" autoComplete="off" onSubmit={(submitted) => { submitted.preventDefault(); signIn(); }}>
			<div className="sign-in-head">
				<span className="sign-in-lock"><LockIcon /></span>
				<span className="sign-in-site selectable min-w-0 flex-1">{event.site}</span>
				<button type="button" className="control btn-icon" title="Not now" aria-label="Not now" disabled={answering} onClick={() => choose("not_now")}>
					<CloseIcon />
				</button>
			</div>
			<p className="sign-in-why selectable">{event.reason}</p>
			{signingIn ? (
				<>
					<label className="block mt-2">Username<input aria-label="Username" className="field w-full" name="username" autoComplete="off" spellCheck={false} required disabled={answering} /></label>
					<label className="block mt-2">Password<input aria-label="Password" className="field w-full" name="password" type="password" autoComplete="new-password" required disabled={answering} /></label>
					<label className="flex items-center gap-2 mt-2"><input type="checkbox" name="save" checked={save} disabled={answering} onChange={(changed) => setSave(changed.target.checked)} />Save to Secrets</label>
					{save && <label className="block mt-2">Name<input aria-label="Key name" className="field w-full" name="name" defaultValue={event.name} maxLength={64} required pattern="[A-Z][A-Z0-9_]*" disabled={answering} /></label>}
					<div className="card-actions">
						<button type="submit" className="control btn-primary" disabled={answering}>Sign In</button>
						<button type="button" className="control btn-quiet" disabled={answering} onClick={() => setSigningIn(false)}>Back</button>
					</div>
				</>
			) : (
				<>
					<div className="card-actions">
						{event.sources.length > 0 && <button type="button" className="control btn-primary" disabled={answering} onClick={() => choose("use")}>Use Cookies</button>}
						<button type="button" className={`control ${event.sources.length > 0 ? "btn" : "btn-primary"}`} disabled={answering} onClick={() => setSigningIn(true)}>Sign In</button>
						<button type="button" className="control btn" disabled={answering} onClick={() => choose("open_browser", openBrowser)}>Open Browser</button>
					</div>
					{event.sources.length === 1 && <p className="sign-in-note">From {sourceName(event.sources[0]!)}</p>}
					{event.sources.length > 1 && (
						<fieldset className="sign-in-note chips items-center" aria-label="Cookies from">
							<span>From</span>
							{event.sources.map((one, index) => (
								<button key={sourceName(one)} type="button" aria-pressed={index === source} className="control btn-quiet btn-sm" disabled={answering} onClick={() => setSource(index)}>
									{sourceName(one)}
								</button>
							))}
						</fieldset>
					)}
				</>
			)}
			{failure !== null && <p role="alert" className="mt-2 text-sm text-danger">{failure}</p>}
		</form>
	);
}
