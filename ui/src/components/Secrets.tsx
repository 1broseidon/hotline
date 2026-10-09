import { Fragment, useCallback, useEffect, useState } from "react";
import type { SharedSecret, SharedSecretKind } from "../generated/contract";
import { CloseIcon, PlusIcon, WarningIcon } from "../icons";
import { BackKey, Band } from "../ui/Band";
import { Chips } from "../ui/Chips";
import { Fold } from "../ui/Fold";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";

const NESTED = "group-row pl-7";
/** What the desk asks of a value, said here before the desk has to. */
const MIN_VALUE_CHARS = 8;

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
 * browser under an arming the operator starts in that teammate's pane (see
 * `PasskeyArm`), listed here once made, and revoked by unticking it on the
 * teammate, removing it here, or deleting it at the site. Every value is
 * written once and never shown again, here or to a teammate. Which teammate
 * gets which is decided on that teammate, under its computer, one tick per
 * name. Every command here is desk-seat only.
 */
export function SecretsSection({ onBack }: { onBack?: (() => void) | undefined }) {
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


	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Secrets</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{adding !== null && (
						<section>
							<h3 className="group-title">Add a secret</h3>
							<div className="grouped">
								<div className="group-row">
									<span className="w-24 shrink-0 text-sm text-ink-2">Kind</span>
									<Chips
										value={adding === "login" ? "login" : "variable"}
										choices={[
											{ id: "variable", name: "Variable", title: "A key or token a teammate's computer finds as an environment variable" },
											{ id: "login", name: "Login", title: "A username and password its computer types on the sites you name" },
										]}
										label="Kind of secret"
										disabled={busy}
										onChange={(kind) => setAdding(kind as SharedSecretKind)}
									/>
								</div>
								{adding === "login" ? (
									<LoginForm key="login" busy={busy} onStore={storeLogin} onCancel={close} />
								) : (
									<VariableForm key="variable" busy={busy} onStore={storeVariable} onCancel={close} />
								)}
							</div>
						</section>
					)}
					<section>
						<h3 className="group-title">Stored</h3>
						<div className="grouped">
							{adding === null && (
								<button
									type="button"
									className="group-row group-row-add"
									disabled={busy}
									onClick={() => {
										close();
										setAdding("variable");
									}}
								>
									<PlusIcon />
									Add a secret
								</button>
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
											<div className="group-row">
												<span className="group-row-text">
													<span className="group-row-title">Remove {secret.name}?</span>
													<span className="group-row-detail">
														{secret.kind === "passkey" ? "Its sign-in stops at once. Delete it on the site too." : "It is gone for good, from every teammate."}
													</span>
												</span>
												<button type="button" className="control btn-quiet" disabled={busy} onClick={close}>
													Cancel
												</button>
												<button type="button" className="control btn text-danger" disabled={busy} onClick={() => void remove(secret.name)}>
													Remove
												</button>
											</div>
										)}
									</Fragment>
								))
							)}
						</div>
						<p className="group-hint">
							Kept in your keychain. Give one to a teammate's computer in its pane.
						</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/** A label and its field, as one row of a form card. */
function FieldRow({ label, htmlFor, top = false, children }: { label: string; htmlFor: string; top?: boolean; children: React.ReactNode }) {
	return (
		<div className={top ? "group-row items-start" : "group-row"}>
			<label className={`w-24 shrink-0 text-sm text-ink-2${top ? " pt-1.5" : ""}`} htmlFor={htmlFor}>
				{label}
			</label>
			<div className="min-w-0 flex-1">{children}</div>
		</div>
	);
}

/** The form's foot: a few words about where the value goes, then its buttons. */
function FormFoot({ note, busy, ready, label, onCancel, onSubmit }: { note: string; busy: boolean; ready: boolean; label: string; onCancel(): void; onSubmit(): void }) {
	return (
		<div className="group-row">
			<span className="min-w-0 flex-1 text-sm text-ink-3">{note}</span>
			<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
				Cancel
			</button>
			<button type="button" className="control btn-primary" disabled={busy || !ready} onClick={onSubmit}>
				{busy ? "Storing…" : label}
			</button>
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
		<>
			{fixed === undefined && (
				<FieldRow label="Name" htmlFor="secret-name">
					<input
						id="secret-name"
						className="field font-mono text-sm"
						placeholder="GITHUB_TOKEN"
						autoComplete="off"
						autoFocus
						spellCheck={false}
						value={name}
						onChange={(event) => setName(event.target.value.toUpperCase())}
					/>
				</FieldRow>
			)}
			<FieldRow label={fixed === undefined ? "Value" : `New value`} htmlFor="secret-value">
				<input
					id="secret-value"
					type="password"
					className="field text-sm"
					autoComplete="new-password"
					autoFocus={fixed !== undefined}
					spellCheck={false}
					placeholder="At least eight characters"
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
			</FieldRow>
			<FormFoot note="Never shown again." busy={busy} ready={ready} label={fixed === undefined ? "Store" : "Replace"} onCancel={onCancel} onSubmit={submit} />
		</>
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
	const [codes, setCodes] = useState(false);
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
		<>
			{fixed === undefined && (
				<FieldRow label="Name" htmlFor="login-name">
					<input
						id="login-name"
						className="field font-mono text-sm"
						placeholder="GITHUB_LOGIN"
						autoComplete="off"
						autoFocus
						spellCheck={false}
						value={name}
						onChange={(event) => setName(event.target.value.toUpperCase())}
					/>
				</FieldRow>
			)}
			<FieldRow label="Sites" htmlFor="login-sites" top>
				<textarea
					id="login-sites"
					className="field min-h-16 w-full font-mono text-sm"
					rows={2}
					placeholder={"https://github.com\nhttps://gist.github.com"}
					spellCheck={false}
					value={sites}
					onChange={(event) => setSites(event.target.value)}
				/>
			</FieldRow>
			<FieldRow label="Username" htmlFor="login-username">
				<input
					id="login-username"
					className="field text-sm"
					autoComplete="off"
					spellCheck={false}
					value={username}
					onChange={(event) => setUsername(event.target.value)}
				/>
			</FieldRow>
			<FieldRow label="Password" htmlFor="login-password">
				<input
					id="login-password"
					type="password"
					className="field text-sm"
					autoComplete="new-password"
					spellCheck={false}
					value={password}
					onChange={(event) => setPassword(event.target.value)}
				/>
			</FieldRow>
			<Fold label title="2-step code" value={totp.trim() === "" ? "None" : "Set"} action="Add" open={codes} onToggle={() => setCodes((was) => !was)}>
				<input
					id="login-totp"
					type="password"
					aria-label="Code seed"
					className="field font-mono text-sm"
					autoComplete="off"
					spellCheck={false}
					placeholder="The secret behind the site's QR code"
					value={totp}
					onChange={(event) => setTotp(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
				<p className="hint">Only if the site asks for six-digit codes.</p>
			</Fold>
			<FormFoot note="Typed only on these sites, never shown." busy={busy} ready={ready} label={fixed === undefined ? "Store" : "Replace"} onCancel={onCancel} onSubmit={submit} />
		</>
	);
}

/**
 * On a teammate, under its computer's Secrets fold: the stored names, one tick each. A
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
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				Stored under Settings → Secrets. Tick what this computer may use; a passkey made for this teammate is ticked by itself.
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
