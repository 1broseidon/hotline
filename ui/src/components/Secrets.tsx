import { Fragment, useCallback, useEffect, useState } from "react";
import type { PasskeyRegistration, SharedSecret, SharedSecretKind } from "../generated/contract";
import { CloseIcon, InfoIcon, PlusIcon, WarningIcon } from "../icons";
import { BackKey, Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { type RosterEntry, wire } from "../wire";

const NESTED = "group-row pl-7";
/** What the desk asks of a value, said here before the desk has to. */
const MIN_VALUE_CHARS = 8;
/** How often the desk is asked whether the passkey has been made. */
const POLL_EVERY_MS = 2000;

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** What a secret is for, as its row says it: never a value. */
function describe(secret: SharedSecret): string {
	switch (secret.kind) {
		case "login":
			return `Login for ${(secret.sites ?? []).join(", ")} as ${secret.username ?? "?"}${secret.totp === true ? ", with a code" : ""}`;
		case "passkey":
			return `Passkey for ${secret.rpId ?? "?"}${secret.userName !== undefined ? ` as ${secret.userName}` : ""}`;
		default:
			return "Variable";
	}
}

/** The same, short, beside a tick. */
function describeShort(secret: SharedSecret): string {
	switch (secret.kind) {
		case "login":
			return `login for ${(secret.sites ?? []).map((site) => site.replace(/^https?:\/\//, "")).join(", ")}`;
		case "passkey":
			return `passkey for ${secret.rpId ?? "?"}`;
		default:
			return "variable";
	}
}

/**
 * Settings → Secrets: what the operator keeps for teammates' computers to
 * use without ever seeing. A variable is a key or token under the name that
 * becomes an environment variable. A login is a username and password, with
 * a code seed when the site asks for one, typed by the computer only on the
 * login's own sites. A passkey is the teammate's own, made by its computer's
 * browser under an arming the operator starts here, and revoked by unticking
 * it on the teammate, removing it here, or deleting it at the site. Every
 * value is written once and never shown again, here or to a teammate. Which
 * teammate gets which is decided on that teammate, under its computer, one
 * tick per name. Every command here is desk-seat only.
 */
export function SecretsSection({ roster, onBack }: { roster: RosterEntry[]; onBack?: (() => void) | undefined }) {
	const [stored, setStored] = useState<SharedSecret[] | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	// Which kind is being added, if one is.
	const [adding, setAdding] = useState<SharedSecretKind | null>(null);
	// The name being replaced, if one is.
	const [editing, setEditing] = useState<string | null>(null);
	const [removing, setRemoving] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);

	const refresh = useCallback(() => {
		void wire
			.command("secrets.list", {})
			.then(setStored)
			.catch((error: unknown) => setRefusal(reason(error)));
	}, []);

	useEffect(() => {
		refresh();
	}, [refresh]);

	const close = () => {
		setAdding(null);
		setEditing(null);
		setRemoving(null);
	};

	const storeVariable = async (name: string, value: string) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.set", { name, value });
			close();
			refresh();
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setBusy(false);
		}
	};

	const storeLogin = async (name: string, sites: string[], username: string, password: string, totp: string | null) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.login.set", { name, sites, username, password, ...(totp !== null ? { totp } : {}) });
			close();
			refresh();
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setBusy(false);
		}
	};

	const remove = async (name: string) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.delete", { name });
			close();
			refresh();
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setBusy(false);
		}
	};

	const stored_ = useCallback(() => refresh(), [refresh]);

	const addRow = (kind: SharedSecretKind, label: string) => (
		<button
			type="button"
			className="group-row group-row-add"
			disabled={busy}
			onClick={() => {
				close();
				setAdding(kind);
			}}
		>
			<PlusIcon />
			{label}
		</button>
	);

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Secrets</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section>
						<h3 className="group-title">Stored</h3>
						<div className="grouped">
							{adding === "variable" ? (
								<VariableForm busy={busy} onStore={storeVariable} onCancel={close} />
							) : (
								addRow("variable", "Store a variable")
							)}
							{adding === "login" ? <LoginForm busy={busy} onStore={storeLogin} onCancel={close} /> : addRow("login", "Store a login")}
							{adding === "passkey" ? (
								<PasskeyPanel roster={roster} onStored={stored_} onClose={close} />
							) : (
								addRow("passkey", "Add a passkey for a teammate")
							)}
							{stored === null ? (
								<p className="group-row text-sm text-ink-3">Reading the keychain…</p>
							) : stored.length === 0 ? (
								<p className="group-row text-sm text-ink-3">Nothing stored yet.</p>
							) : (
								stored.map((secret) => (
									<Fragment key={secret.name}>
										<div className="group-row">
											<span className="group-row-text">
												<span className="group-row-title font-mono text-sm">{secret.name}</span>
												<span className="group-row-detail">
													{describe(secret)} · stored {new Date(secret.updatedAt).toLocaleDateString()}
												</span>
											</span>
											{secret.kind !== "passkey" && (
												<button
													type="button"
													className="control btn-quiet btn-sm"
													disabled={busy}
													onClick={() => {
														close();
														setEditing(secret.name);
													}}
												>
													Replace
												</button>
											)}
											<button
												type="button"
												className="control btn-icon -mr-1.5"
												aria-label={`Remove ${secret.name}`}
												title="Remove"
												disabled={busy}
												onClick={() => {
													close();
													setRemoving(secret.name);
												}}
											>
												<CloseIcon />
											</button>
										</div>
										{editing === secret.name && secret.kind === "variable" && (
											<VariableForm name={secret.name} busy={busy} onStore={storeVariable} onCancel={close} />
										)}
										{editing === secret.name && secret.kind === "login" && (
											<LoginForm name={secret.name} sites={secret.sites ?? []} username={secret.username ?? ""} busy={busy} onStore={storeLogin} onCancel={close} />
										)}
										{removing === secret.name && (
											<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
												<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
													{secret.kind === "passkey"
														? `Remove ${secret.name}? The teammate's browser loses it at once and cannot sign in with it again. The site still lists the passkey until you delete it there too.`
														: `Remove ${secret.name}? A teammate given it loses it from its computer, and the value is kept nowhere else.`}
												</span>
												<div className="flex justify-end gap-2">
													<button type="button" className="control btn-quiet" disabled={busy} onClick={close}>
														Cancel
													</button>
													<button type="button" className="control btn" disabled={busy} onClick={() => void remove(secret.name)}>
														Remove
													</button>
												</div>
											</div>
										)}
									</Fragment>
								))
							)}
						</div>
						<p className="group-hint">
							A variable is an environment variable in a teammate's computer. A login is typed by the computer, only on the
							login's own sites. A passkey is the teammate's own, made by its computer's browser while you watch, and it signs
							in by itself. Every value is kept in this machine's keychain, written once and never shown again, here or to a
							teammate; the computer redacts it from what its tools answer. Give one to a teammate in its pane, under its
							computer. Take a passkey back from there, from here, or from the site's own security settings: any one of the
							three ends it.
						</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/** A variable: a name, unless replacing, and a value that is typed once and never read back. */
function VariableForm({
	name: fixed,
	busy,
	onStore,
	onCancel,
}: {
	name?: string;
	busy: boolean;
	onStore(name: string, value: string): Promise<void>;
	onCancel(): void;
}) {
	const [name, setName] = useState(fixed ?? "");
	const [value, setValue] = useState("");
	const ready = name.trim() !== "" && value.length >= MIN_VALUE_CHARS;
	const submit = () => {
		if (!ready || busy) return;
		void onStore(name.trim(), value);
	};

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<div>
				<label className="label" htmlFor="secret-name">
					Name
				</label>
				<input
					id="secret-name"
					className="field font-mono text-sm"
					placeholder="GITHUB_TOKEN"
					autoComplete="off"
					spellCheck={false}
					readOnly={fixed !== undefined}
					value={name}
					onChange={(event) => setName(event.target.value.toUpperCase())}
				/>
			</div>
			<div>
				<label className="label" htmlFor="secret-value">
					Value
				</label>
				<input
					id="secret-value"
					type="password"
					className="field text-sm"
					autoComplete="new-password"
					spellCheck={false}
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
			</div>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				{fixed === undefined ? "The name is the environment variable a teammate's computer finds it under. " : ""}
				At least eight characters, one line. Kept in this machine's keychain and never shown again.
			</span>
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
					Cancel
				</button>
				<button type="button" className="control btn" disabled={busy || !ready} onClick={submit}>
					{busy ? "Storing…" : fixed === undefined ? "Store" : "Replace"}
				</button>
			</div>
		</div>
	);
}

/**
 * A login: the sites it is for, one per line, a username, a password, and
 * the code seed when the site asks for six digits. Replacing keeps the name
 * and asks for everything else again, since the password is never read back.
 */
function LoginForm({
	name: fixed,
	sites: knownSites,
	username: knownUsername,
	busy,
	onStore,
	onCancel,
}: {
	name?: string;
	sites?: string[];
	username?: string;
	busy: boolean;
	onStore(name: string, sites: string[], username: string, password: string, totp: string | null): Promise<void>;
	onCancel(): void;
}) {
	const [name, setName] = useState(fixed ?? "");
	const [sites, setSites] = useState((knownSites ?? []).join("\n"));
	const [username, setUsername] = useState(knownUsername ?? "");
	const [password, setPassword] = useState("");
	const [totp, setTotp] = useState("");
	const siteList = sites
		.split(/\s+/)
		.map((site) => site.trim())
		.filter((site) => site !== "");
	const ready = name.trim() !== "" && siteList.length > 0 && username.trim() !== "" && password.length >= MIN_VALUE_CHARS;
	const submit = () => {
		if (!ready || busy) return;
		void onStore(name.trim(), siteList, username.trim(), password, totp.trim() === "" ? null : totp.replace(/\s+/g, ""));
	};

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<div>
				<label className="label" htmlFor="login-name">
					Name
				</label>
				<input
					id="login-name"
					className="field font-mono text-sm"
					placeholder="GITHUB_LOGIN"
					autoComplete="off"
					spellCheck={false}
					readOnly={fixed !== undefined}
					value={name}
					onChange={(event) => setName(event.target.value.toUpperCase())}
				/>
			</div>
			<div>
				<label className="label" htmlFor="login-sites">
					Sites
				</label>
				<textarea
					id="login-sites"
					className="field min-h-16 w-full font-mono text-sm"
					rows={2}
					placeholder={"https://github.com\nhttps://gist.github.com"}
					spellCheck={false}
					value={sites}
					onChange={(event) => setSites(event.target.value)}
				/>
			</div>
			<div>
				<label className="label" htmlFor="login-username">
					Username
				</label>
				<input
					id="login-username"
					className="field text-sm"
					autoComplete="off"
					spellCheck={false}
					value={username}
					onChange={(event) => setUsername(event.target.value)}
				/>
			</div>
			<div>
				<label className="label" htmlFor="login-password">
					Password
				</label>
				<input
					id="login-password"
					type="password"
					className="field text-sm"
					autoComplete="new-password"
					spellCheck={false}
					value={password}
					onChange={(event) => setPassword(event.target.value)}
				/>
			</div>
			<div>
				<label className="label" htmlFor="login-totp">
					Code seed, if the site asks for six digits
				</label>
				<input
					id="login-totp"
					type="password"
					className="field font-mono text-sm"
					autoComplete="off"
					spellCheck={false}
					placeholder="The base32 secret behind the QR code, optional"
					value={totp}
					onChange={(event) => setTotp(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
			</div>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				One site per line, with its scheme. The computer types this login only on a page of these sites and refuses any
				other; the teammate is told the name and the sites and never sees the password or a code. Kept in this machine's
				keychain and never shown again.
			</span>
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
					Cancel
				</button>
				<button type="button" className="control btn" disabled={busy || !ready} onClick={submit}>
					{busy ? "Storing…" : fixed === undefined ? "Store" : "Replace"}
				</button>
			</div>
		</div>
	);
}

/**
 * A passkey is made, not typed: the operator names it, names the site and
 * the teammate, and arms that teammate's computer for ten minutes. Then they
 * open the teammate's screen, sign in to the site as the teammate should,
 * and add a passkey in the site's security settings — or ask the teammate
 * to — and the computer's browser makes it. The desk polls until it is
 * made, stores it, and ticks it for the teammate; nothing is stored until
 * then, and a cancel ends the arming with nothing.
 */
function PasskeyPanel({ roster, onStored, onClose }: { roster: RosterEntry[]; onStored(): void; onClose(): void }) {
	const teammates = roster
		.filter((entry) => entry.persona.computer?.enabled === true)
		.map((entry) => ({ id: entry.persona.id, name: entry.persona.name }));
	const [name, setName] = useState("");
	const [site, setSite] = useState("");
	const [personaId, setPersonaId] = useState(teammates[0]?.id ?? "");
	const [registration, setRegistration] = useState<PasskeyRegistration | null>(null);
	const [note, setNote] = useState<string | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [working, setWorking] = useState(false);
	const teammate = teammates.find((one) => one.id === personaId)?.name ?? "the teammate";

	// An arming may be live from before this panel opened, or the room may
	// have stored the passkey while it was closed: pick either up.
	useEffect(() => {
		if (personaId === "") return;
		let gone = false;
		void wire
			.command("secrets.passkey.registration", { personaId })
			.then((current) => {
				if (gone) return;
				if (current.state === "armed") {
					setRegistration(current);
				} else if (current.state === "stored") {
					setRegistration(current);
					onStored();
				}
			})
			.catch(() => {});
		return () => {
			gone = true;
		};
	}, [personaId, onStored]);

	// While armed, ask every couple of seconds whether it has been made.
	const armed = registration?.state === "armed";
	useEffect(() => {
		if (!armed || personaId === "") return;
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
					setNote("The arming ended without a passkey: its ten minutes ran out, or the computer restarted. Arm it again when you are ready.");
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
	}, [armed, personaId, onStored]);

	const ready = name.trim() !== "" && site.trim() !== "" && personaId !== "";
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

	if (registration?.state === "stored") {
		return (
			<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					Stored <span className="font-mono">{registration.name}</span> and ticked it for {teammate}. Its browser signs in to{" "}
					{registration.rpId} with it from now on. Take it back any time: untick it on {teammate}, remove it here, or delete the
					passkey in the site's security settings.
				</span>
				<div className="flex justify-end gap-2">
					<button type="button" className="control btn" onClick={onClose}>
						Done
					</button>
				</div>
			</div>
		);
	}

	if (registration?.state === "armed") {
		const until = registration.expiresAt !== undefined ? new Date(registration.expiresAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : null;
		return (
			<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
				<span className="group-row-text">
					<span className="group-row-title">
						Armed for {registration.rpId}
						{until !== null ? ` until ${until}` : ""}
					</span>
					<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
						Open {teammate}'s screen, sign in to {registration.rpId} the way the teammate should be signed in, and add a passkey in
						the site's security settings; or ask {teammate} to. Its browser makes the passkey, and the moment it does this desk
						stores it as <span className="font-mono">{registration.name}</span> and ticks it for {teammate}, whether or not this
						page is open; {teammate}'s tape says so. Nothing else can be made while armed, and nothing at all when not.
					</span>
				</span>
				<span className="group-row-detail">Waiting for the passkey…</span>
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
			<div>
				<span className="label">Teammate</span>
				{teammates.length === 0 ? (
					<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
						No teammate has a computer yet. Turn one on in a teammate's pane first.
					</span>
				) : (
					<Picker value={personaId} choices={teammates} placeholder="Choose a teammate" label="Teammate" field onChange={setPersonaId} />
				)}
			</div>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				The site is its host name, as the passkey will be registered: github.com, not a page. Arming lasts ten minutes and
				starts the teammate's computer if it is stopped. The passkey is the teammate's own, made by its browser; it is stored here
				only once made, and ticked for that teammate.
			</span>
			{note !== null && (
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={working} onClick={onClose}>
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

/**
 * On a teammate, under its computer: the stored names, one tick each. A
 * tick is the grant — nothing is ticked until the person ticks it — and a
 * name the keychain no longer has stays in the list with a warning until
 * it is unticked, so a stale grant is seen rather than silently dropped.
 * Unticking a passkey is one of the three ways to take it back.
 */
export function ComputerSecrets({
	granted,
	disabled,
	onChange,
}: {
	granted: string[];
	disabled: boolean;
	onChange(granted: string[]): void;
}) {
	const [stored, setStored] = useState<SharedSecret[] | null>(null);
	const [note, setNote] = useState<string | null>(null);
	const [about, setAbout] = useState(false);

	// Re-read whenever the ticks change from elsewhere — a passkey the room
	// just stored and ticked shows as what it is, not as "not stored".
	const grantedKey = granted.join("\n");
	useEffect(() => {
		let gone = false;
		wire
			.command("secrets.list", {})
			.then((list) => !gone && setStored(list))
			.catch((error: unknown) => !gone && setNote(reason(error)));
		return () => {
			gone = true;
		};
	}, [grantedKey]);

	const storedNames = stored === null ? null : stored.map((one) => one.name);
	const names = storedNames === null ? granted : [...storedNames, ...granted.filter((name) => !storedNames.includes(name))];
	const toggle = (name: string) =>
		onChange(granted.includes(name) ? granted.filter((one) => one !== name) : [...granted, name].sort());

	return (
		<div className={`${NESTED} flex-col items-stretch gap-1.5 py-3`}>
			<span className="group-row-text">
				<span className="group-row-title flex items-center gap-1">
					Secrets
					<button
						type="button"
						className="control btn-icon btn-quiet h-6 w-6 text-ink-3"
						title="Secrets can be added under Settings → Secrets."
						aria-label="About secrets"
						aria-expanded={about}
						onClick={() => setAbout((open) => !open)}
					>
						<InfoIcon />
					</button>
				</span>
				{about && (
					<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
						Secrets can be added under Settings → Secrets.
					</span>
				)}
			</span>
			{stored === null && note === null && <span className="group-row-detail">Reading the keychain…</span>}
			{note !== null && (
				<span className="group-row-detail text-danger" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}
			{stored !== null && names.length === 0 && <span className="group-row-detail">None stored yet.</span>}
			{names.map((name) => {
				const record = stored?.find((one) => one.name === name);
				const missing = stored !== null && record === undefined;
				return (
					<label key={name} className="flex cursor-pointer items-center gap-2 py-1 text-sm">
						<input type="checkbox" className="check" checked={granted.includes(name)} disabled={disabled} onChange={() => toggle(name)} />
						<span className="min-w-0 flex-1 truncate">
							<span className="font-mono">{name}</span>
							{record !== undefined && <span className="group-row-detail ml-2">{describeShort(record)}</span>}
						</span>
						{missing && (
							<span
								className="flex items-center gap-1 text-danger"
								title="Not stored any more. Untick it, or store it again under Settings → Secrets."
							>
								<WarningIcon />
								<span className="group-row-detail text-danger">not stored</span>
							</span>
						)}
					</label>
				);
			})}
		</div>
	);
}
