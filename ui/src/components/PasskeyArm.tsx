import { useEffect, useState } from "react";
import type { PasskeyAsk, PasskeyRegistration } from "../generated/contract";
import { PlusIcon } from "../icons";
import { Refusal } from "../ui/Refusal";
import { wire } from "../wire";

const NESTED = "group-row pl-7";
/** How often the desk is asked where the making stands. */
const POLL_EVERY_MS = 2000;

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** Who the site said the passkey would be for, as the site named them. */
export function askedFor(ask: Pick<PasskeyAsk, "userName" | "userDisplayName">): string | null {
	const { userName, userDisplayName } = ask;
	if (userName !== undefined && userDisplayName !== undefined && userDisplayName !== userName) return `${userName} (${userDisplayName})`;
	return userName ?? userDisplayName ?? null;
}

/**
 * Under a teammate's computer, in its Browser fold: a passkey is made, not
 * typed. The operator names it and the site, and arms this teammate's
 * computer for ten minutes. Then they open the teammate's screen, sign in
 * to the site as the teammate should be, and add a passkey in the site's
 * security settings — or ask the teammate to. The site's request waits in
 * the browser until the person approves it on the teammate's tape; then
 * the browser makes it, and the room stores it and ticks it for the
 * teammate. Nothing is stored until then; a denial or a cancel ends the
 * arming with nothing; and the room's own watch does all of it whether or
 * not this pane is open — this row only reads where it stands.
 */
export function PasskeyArm({
	personaId,
	teammate,
	disabled,
	onStored,
}: {
	personaId: string;
	teammate: string;
	disabled: boolean;
	onStored(): void;
}) {
	const [open, setOpen] = useState(false);
	const [name, setName] = useState("");
	const [site, setSite] = useState("");
	const [registration, setRegistration] = useState<PasskeyRegistration | null>(null);
	const [note, setNote] = useState<string | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [working, setWorking] = useState(false);

	// An arming may be live from before this pane opened, or the room may
	// have stored the passkey while it was closed: pick either up.
	useEffect(() => {
		let gone = false;
		void wire
			.command("secrets.passkey.registration", { personaId })
			.then((current) => {
				if (gone || current.state === "idle") return;
				setRegistration(current);
				setOpen(true);
				if (current.state === "stored") onStored();
			})
			.catch(() => {});
		return () => {
			gone = true;
		};
	}, [personaId, onStored]);

	// While live, ask every couple of seconds where it stands.
	const live = registration !== null && registration.state !== "idle" && registration.state !== "stored";
	useEffect(() => {
		if (!live) return;
		let gone = false;
		const look = async () => {
			try {
				const next = await wire.command("secrets.passkey.registration", { personaId });
				if (gone) return;
				if (next.state === "stored") {
					setRegistration(next);
					onStored();
				} else if (next.state === "idle") {
					setRegistration(null);
					setNote(
						"The arming ended without a passkey: the request was denied on the tape, its ten minutes ran out, or the computer restarted. Arm it again when you are ready.",
					);
				} else {
					setRegistration(next);
				}
			} catch (error) {
				if (gone) return;
				setRegistration(null);
				setRefusal(reason(error));
			}
		};
		const timer = setInterval(() => void look(), POLL_EVERY_MS);
		return () => {
			gone = true;
			clearInterval(timer);
		};
	}, [live, personaId, onStored]);

	const close = () => {
		setOpen(false);
		setRegistration(null);
		setNote(null);
		setRefusal(null);
		setName("");
		setSite("");
	};

	const ready = name.trim() !== "" && site.trim() !== "";
	const register = async () => {
		if (!ready || working) return;
		setWorking(true);
		setRefusal(null);
		setNote(null);
		try {
			setRegistration(await wire.command("secrets.passkey.register", { name: name.trim(), personaId, rpId: site.trim().toLowerCase() }));
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setWorking(false);
		}
	};
	const cancel = async () => {
		setWorking(true);
		setRefusal(null);
		try {
			await wire.command("secrets.passkey.cancel", { personaId });
			setRegistration(null);
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setWorking(false);
		}
	};

	if (!open) {
		return (
			<button type="button" className={`${NESTED} group-row-add`} disabled={disabled} onClick={() => setOpen(true)}>
				<PlusIcon />
				Add a passkey
			</button>
		);
	}

	if (registration?.state === "stored") {
		return (
			<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					Stored <span className="font-mono">{registration.name}</span> and ticked it for {teammate}. Its browser signs in to{" "}
					{registration.rpId} with it from now on. Take it back any time: untick it under Secrets, remove it under Settings →
					Secrets, or delete the passkey in the site's security settings.
				</span>
				<div className="flex justify-end gap-2">
					<button type="button" className="control btn" onClick={close}>
						Done
					</button>
				</div>
			</div>
		);
	}

	if (registration !== null && registration.state !== "idle") {
		const rpId = registration.rpId ?? site;
		const until = registration.expiresAt !== undefined ? new Date(registration.expiresAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;
		const ask = registration.ask;
		const who = ask !== undefined ? askedFor(ask) : null;
		let title: string;
		let detail: string;
		let waiting: string;
		if (registration.state === "armed" || ask === undefined) {
			title = `Armed for ${rpId}${until !== null ? ` until ${until}` : ""}`;
			detail = `Open ${teammate}'s screen, sign in to ${rpId} the way the teammate should be signed in, and add a passkey in the site's security settings; or ask ${teammate} to. When the site asks, a card on ${teammate}'s tape asks you to approve it, whether or not this pane is open. Nothing is made without that answer, and nothing at all when not armed.`;
			waiting = "Waiting for the site to ask…";
		} else if (registration.state === "asked") {
			title = `${rpId} asks for a passkey`;
			detail = `${ask.rpName !== undefined ? `${ask.rpName} at ` : ""}${ask.origin} asks to make a passkey${who !== null ? ` for ${who}` : ""}. Approve or deny it on ${teammate}'s tape. Approved, it is stored as ${registration.name ?? "the name you gave"} and ticked for ${teammate}; denied, the arming ends.`;
			waiting = "Waiting for your answer on the tape…";
		} else {
			title = `Approved: ${teammate}'s browser is making it`;
			detail = `The moment it is made, this desk stores it as ${registration.name ?? "the name you gave"} and ticks it for ${teammate}; ${teammate}'s tape says so.`;
			waiting = "Waiting for the passkey…";
		}
		return (
			<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
				<span className="group-row-text">
					<span className="group-row-title">{title}</span>
					<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
						{detail}
					</span>
				</span>
				<span className="group-row-detail">{waiting}</span>
				<div className="flex justify-end gap-2">
					<button type="button" className="control btn-quiet" disabled={working} onClick={() => void cancel()}>
						Cancel
					</button>
				</div>
				{refusal !== null && <Refusal message={refusal} />}
			</div>
		);
	}

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<div>
				<label className="label" htmlFor="passkey-name">
					Name
				</label>
				<input
					id="passkey-name"
					className="field font-mono text-sm"
					placeholder="GITHUB_PASSKEY"
					autoComplete="off"
					spellCheck={false}
					value={name}
					onChange={(event) => setName(event.target.value.toUpperCase())}
				/>
			</div>
			<div>
				<label className="label" htmlFor="passkey-site">
					Site
				</label>
				<input
					id="passkey-site"
					className="field font-mono text-sm"
					placeholder="github.com"
					autoComplete="off"
					spellCheck={false}
					value={site}
					onChange={(event) => setSite(event.target.value)}
				/>
			</div>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				The site is its host name, as the passkey will be registered: github.com, not a page. Arming lasts ten minutes and starts{" "}
				{teammate}'s computer if it is stopped. The passkey is {teammate}'s own, made by its browser once you approve the site's
				request on its tape; it is stored here only once made, and ticked for {teammate}.
			</span>
			{note !== null && (
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={working} onClick={close}>
					Cancel
				</button>
				<button type="button" className="control btn" disabled={working || !ready} onClick={() => void register()}>
					{working ? "Arming…" : "Arm the computer"}
				</button>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
		</div>
	);
}
