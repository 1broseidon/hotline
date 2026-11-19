import { Fragment, useCallback, useEffect, useState } from "react";
import type { GrantPlace, Persona, SecretGrant, SecretService, SecretUse, SharedSecret, SharedSecretKind, UsedWith, VariableUse } from "../generated/contract";
import { CloseIcon, PlusIcon, WarningIcon } from "../icons";
import { BackKey, Band } from "../ui/Band";
import { Chips } from "../ui/Chips";
import { Fold } from "../ui/Fold";
import { Refusal, refused, type Refused } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { CookieSourcesGroup, CookiesInUse } from "./settings/CookieSettings";

const NESTED = "group-row pl-7";
/** What the desk asks of a value, said here before the desk has to. */
const MIN_VALUE_CHARS = 8;

const UNREAD = "Couldn't read the keychain";
const UNSAVED = "Couldn't store secret";

/** The label a readable grant carries, and the line that confirms one. */
const READABLE = "Readable by this teammate's programs";

/** The services a key can be used with, from the desk's `services.json`. */
function useServices(): SecretService[] {
	const [services, setServices] = useState<SecretService[]>([]);
	useEffect(() => {
		let gone = false;
		wire
			.command("secrets.services", {})
			.then((list) => !gone && setServices(list))
			.catch(() => {});
		return () => {
			gone = true;
		};
	}, []);
	return services;
}

/** The service a key's "Used with" names, or the host the person gave. */
function usedWithName(use: VariableUse, services: readonly SecretService[]): string {
	const known = services.find((service) => service.id !== "other" && service.origins.length === use.origins.length && service.origins.every((origin) => use.origins.includes(origin)));
	return known?.name ?? use.origins.map((origin) => origin.replace(/^https:\/\//, "")).join(", ");
}

/** The service a key's start suggests: the longest prefix any service lists. */
function suggestedService(value: string, services: readonly SecretService[]): string | null {
	let best: { id: string; length: number } | null = null;
	for (const service of services)
		for (const prefix of service.prefixes)
			if (value.startsWith(prefix) && (best === null || prefix.length > best.length)) best = { id: service.id, length: prefix.length };
	return best?.id ?? null;
}

/** What a secret is for, as its row says it: never a value. */
function describe(secret: SharedSecret, services: readonly SecretService[]): string {
	switch (secret.kind) {
		case "login":
			return `Login for ${(secret.sites ?? []).join(", ")} as ${secret.username ?? "?"}${secret.totp === true ? ", with a code" : ""}`;
		case "passkey":
			return `Passkey for ${secret.rpId ?? "?"}${secret.userName !== undefined ? ` as ${secret.userName}` : ""}`;
		default:
			return secret.usedWith !== undefined ? `Key used with ${usedWithName(secret.usedWith, services)}` : "Variable";
	}
}

/** The same, short, beside a tick. */
function describeShort(secret: SharedSecret, services: readonly SecretService[], capital = false): string {
	const said = (() => {
		switch (secret.kind) {
			case "login":
				return `login for ${(secret.sites ?? []).map((site) => site.replace(/^https?:\/\//, "")).join(", ")}`;
			case "passkey":
				return `passkey for ${secret.rpId ?? "?"}`;
			default:
				return secret.usedWith !== undefined ? `key used with ${usedWithName(secret.usedWith, services)}` : "variable";
		}
	})();
	return capital ? said.charAt(0).toUpperCase() + said.slice(1) : said;
}

/**
 * The "Used with" choice for a key: none, one of the desk's services, or
 * Other with the host the key is sent to. A key's start may suggest one;
 * the person sees it picked and stores it, or picks another.
 */
function UsedWithRow({
	services,
	choice,
	host,
	suggested,
	busy,
	onChoice,
	onHost,
}: {
	services: readonly SecretService[];
	choice: string;
	host: string;
	suggested: boolean;
	busy: boolean;
	onChoice(choice: string): void;
	onHost(host: string): void;
}) {
	return (
		<>
			<div className="group-row items-start">
				<span className="w-24 shrink-0 pt-1.5 text-sm text-ink-2">Used with</span>
				<div className="min-w-0 flex-1">
					<Chips
						label="Used with"
						value={choice}
						choices={[{ id: "none", name: "None" }, ...services.map((service) => ({ id: service.id, name: service.name }))]}
						disabled={busy}
						onChange={onChoice}
					/>
					{suggested && choice !== "none" && <p className="hint">Picked from how the key starts.</p>}
				</div>
			</div>
			{choice === "other" && (
				<FieldRow label="Host" htmlFor="secret-host">
					<input
						id="secret-host"
						className="field font-mono text-sm"
						placeholder="api.example.com"
						autoComplete="off"
						spellCheck={false}
						value={host}
						onChange={(event) => onHost(event.target.value)}
					/>
				</FieldRow>
			)}
		</>
	);
}

/** A "Used with" choice as the desk takes it, or none. */
function chosen(choice: string, host: string): UsedWith | undefined {
	if (choice === "none") return undefined;
	return choice === "other" ? { service: "other", host: host.trim() } : { service: choice };
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
 * name. Under them, the browsers Hotline may read cookies from, and who
 * holds the person's cookies now. Every command here is desk-seat only.
 */
export function SecretsSection({
	onBack,
	team = [],
	onOpenTeammate,
}: {
	onBack?: (() => void) | undefined;
	team?: readonly { id: string; avatar?: Persona["avatar"] }[];
	onOpenTeammate?: ((personaId: string) => void) | undefined;
}) {
	const [stored, setStored] = useState<SharedSecret[] | null>(null);
	const [refusal, setRefusal] = useState<Refused | null>(null);
	// Which kind is being added, if one is.
	const [adding, setAdding] = useState<SharedSecretKind | null>(null);
	// The name being replaced, if one is.
	const [editing, setEditing] = useState<string | null>(null);
	const [removing, setRemoving] = useState<string | null>(null);
	// The key whose "Used with" is being changed, if one is.
	const [using, setUsing] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const services = useServices();

	const refresh = useCallback(() => {
		void wire
			.command("secrets.list", {})
			.then(setStored)
			.catch((error: unknown) => setRefusal(refused(UNREAD, error)));
	}, []);

	useEffect(() => {
		refresh();
	}, [refresh]);

	const close = () => {
		setAdding(null);
		setEditing(null);
		setRemoving(null);
		setUsing(null);
	};

	const storeVariable = async (name: string, value: string, usedWith?: UsedWith) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.set", { name, value, ...(usedWith !== undefined ? { usedWith } : {}) });
			close();
			refresh();
		} catch (error) {
			setRefusal(refused(UNSAVED, error));
		} finally {
			setBusy(false);
		}
	};

	const storeLogin = async (name: string, sites: string[], username: string, password: string, totp: string | null, use: SecretUse) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.login.set", { name, sites, username, password, use, ...(totp !== null ? { totp } : {}) });
			close();
			refresh();
		} catch (error) {
			setRefusal(refused(UNSAVED, error));
		} finally {
			setBusy(false);
		}
	};

	const changeUse = async (name: string, use: SecretUse) => {
		setBusy(true);
		setRefusal(null);
		try {
			const secret = await wire.command("secrets.use.set", { name, use });
			setStored((was) => was?.map((one) => one.name === name ? secret : one) ?? null);
		} catch (error) {
			setRefusal(refused("Couldn't change use", error));
		} finally {
			setBusy(false);
		}
	};

	const changeUsedWith = async (name: string, usedWith?: UsedWith) => {
		setBusy(true);
		setRefusal(null);
		try {
			const secret = await wire.command("secrets.used_with.set", { name, ...(usedWith !== undefined ? { usedWith } : {}) });
			setStored((was) => was?.map((one) => one.name === name ? secret : one) ?? null);
			setUsing(null);
		} catch (error) {
			setRefusal(refused("Couldn't change Used with", error));
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
			setRefusal(refused("Couldn't remove secret", error));
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
									<VariableForm key="variable" services={services} busy={busy} onStore={storeVariable} onCancel={close} />
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
													{describe(secret, services)} · stored {new Date(secret.updatedAt).toLocaleDateString()}
												</span>
											</span>
											{secret.kind !== "variable" && <Chips
												label={`Use ${secret.name}`} value={secret.use ?? "auto"} disabled={busy}
												choices={[{ id: "auto", name: "Automatic" }, { id: "ask", name: "Ask me" }]}
												onChange={(use) => void changeUse(secret.name, use as SecretUse)}
											/>}
											{secret.kind === "variable" && (
												<button
													type="button"
													className="control btn-quiet btn-sm"
													disabled={busy}
													onClick={() => {
														close();
														setUsing(secret.name);
													}}
												>
													Used with
												</button>
											)}
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
											<VariableForm name={secret.name} services={services} busy={busy} onStore={storeVariable} onCancel={close} />
										)}
										{using === secret.name && (
											<UsedWithForm secret={secret} services={services} busy={busy} onSave={(usedWith) => void changeUsedWith(secret.name, usedWith)} onCancel={close} />
										)}
										{editing === secret.name && secret.kind === "login" && (
											<LoginForm name={secret.name} usage={secret.use} sites={secret.sites ?? []} username={secret.username ?? ""} busy={busy} onStore={storeLogin} onCancel={close} />
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
							Kept in your keychain. Allow one for a teammate's computer in that teammate's settings.
						</p>
					</section>
					<CookieSourcesGroup />
					<CookiesInUse team={team} onOpenTeammate={onOpenTeammate} />
					{refusal !== null && <Refusal {...refusal} />}
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
	services,
	busy,
	onStore,
	onCancel,
}: {
	name?: string;
	services: readonly SecretService[];
	busy: boolean;
	onStore(name: string, value: string, usedWith?: UsedWith): Promise<void>;
	onCancel(): void;
}) {
	const [name, setName] = useState(fixed ?? "");
	const [value, setValue] = useState("");
	// Replacing keeps the "Used with" the key has; only a new key asks.
	const [choice, setChoice] = useState("none");
	const [host, setHost] = useState("");
	const [picked, setPicked] = useState(false);
	const ready = name.trim() !== "" && value.length >= MIN_VALUE_CHARS && (choice !== "other" || host.trim() !== "");
	const submit = () => {
		if (!ready || busy) return;
		void onStore(name.trim(), value, chosen(choice, host));
	};
	const typed = (next: string) => {
		setValue(next);
		if (fixed !== undefined || picked) return;
		setChoice(suggestedService(next, services) ?? "none");
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
						// biome-ignore lint/a11y/noAutofocus: the form opens on a press of Add a secret, and the name comes first.
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
					// biome-ignore lint/a11y/noAutofocus: on a press of Replace the name is fixed, so the new value is the field to type in.
					autoFocus={fixed !== undefined}
					spellCheck={false}
					placeholder="At least eight characters"
					value={value}
					onChange={(event) => typed(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
			</FieldRow>
			{fixed === undefined && (
				<UsedWithRow
					services={services}
					choice={choice}
					host={host}
					suggested={!picked}
					busy={busy}
					onChoice={(next) => {
						setPicked(true);
						setChoice(next);
					}}
					onHost={setHost}
				/>
			)}
			<FormFoot note="Never shown again." busy={busy} ready={ready} label={fixed === undefined ? "Store" : "Replace"} onCancel={onCancel} onSubmit={submit} />
		</>
	);
}

/** A stored key's "Used with", changed without its value typed again. */
function UsedWithForm({
	secret,
	services,
	busy,
	onSave,
	onCancel,
}: {
	secret: SharedSecret;
	services: readonly SecretService[];
	busy: boolean;
	onSave(usedWith?: UsedWith): void;
	onCancel(): void;
}) {
	const current = secret.usedWith;
	const known = current === undefined ? "none" : (services.find((service) => service.id !== "other" && usedWithName(current, [service]) === service.name)?.id ?? "other");
	const [choice, setChoice] = useState(known);
	const [host, setHost] = useState(known === "other" && current !== undefined ? (current.origins[0] ?? "").replace(/^https:\/\//, "") : "");
	const ready = choice !== "other" || host.trim() !== "";
	return (
		<>
			<UsedWithRow services={services} choice={choice} host={host} suggested={false} busy={busy} onChoice={setChoice} onHost={setHost} />
			<FormFoot
				note={choice === "none" ? READABLE : "Added only to requests there."}
				busy={busy}
				ready={ready}
				label="Save"
				onCancel={onCancel}
				onSubmit={() => onSave(chosen(choice, host))}
			/>
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
	usage = "auto",
	busy,
	onStore,
	onCancel,
}: {
	name?: string;
	sites?: string[];
	username?: string;
	usage?: SecretUse;
	busy: boolean;
	onStore(name: string, sites: string[], username: string, password: string, totp: string | null, use: SecretUse): Promise<void>;
	onCancel(): void;
}) {
	const [name, setName] = useState(fixed ?? "");
	const [sites, setSites] = useState((knownSites ?? []).join("\n"));
	const [username, setUsername] = useState(knownUsername ?? "");
	const [password, setPassword] = useState("");
	const [totp, setTotp] = useState("");
	const [codes, setCodes] = useState(false);
	const [use, setUse] = useState<SecretUse>(usage);
	const siteList = sites
		.split(/\s+/)
		.map((site) => site.trim())
		.filter((site) => site !== "");
	const ready = name.trim() !== "" && siteList.length > 0 && username.trim() !== "" && password.length >= MIN_VALUE_CHARS;
	const submit = () => {
		if (!ready || busy) return;
		void onStore(name.trim(), siteList, username.trim(), password, totp.trim() === "" ? null : totp.replace(/\s+/g, ""), use);
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
						// biome-ignore lint/a11y/noAutofocus: the form opens when Login is picked under Add a secret, and the name comes first.
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
			<div className="group-row">
				<span className="w-24 shrink-0 text-sm text-ink-2">Use</span>
				<Chips label="Login use" value={use} choices={[{ id: "auto", name: "Automatic" }, { id: "ask", name: "Ask me" }]} disabled={busy} onChange={(value) => setUse(value as SecretUse)} />
			</div>
			<FormFoot note="Typed only on these sites, never shown." busy={busy} ready={ready} label={fixed === undefined ? "Store" : "Replace"} onCancel={onCancel} onSubmit={submit} />
		</>
	);
}

/**
 * On a teammate, under its own Secrets: the stored names, one tick each. A
 * tick is the grant — nothing is ticked until the person ticks it — and a
 * name the keychain no longer has stays in the list with a warning until
 * it is unticked, so a stale grant is seen rather than silently dropped.
 * Unticking a passkey is one of the three ways to take it back. A plain
 * variable can only be given readable, so its tick asks first, and a
 * readable grant says so. Where the teammate has both a computer and a
 * desk shell, a variable's grant says which of them it reaches; a login or
 * a passkey goes to whichever browser the teammate has.
 */
export function TeammateSecrets({
	grants,
	bothPlaces,
	disabled,
	onChange,
}: {
	grants: SecretGrant[];
	bothPlaces: boolean;
	disabled: boolean;
	onChange(grants: SecretGrant[]): void;
}) {
	const [stored, setStored] = useState<SharedSecret[] | null>(null);
	const [note, setNote] = useState<Refused | null>(null);
	// The plain variable whose tick waits for the person to say yes.
	const [asking, setAsking] = useState<string | null>(null);
	const services = useServices();
	const granted = grants.map((grant) => grant.name);

	// Re-read whenever the ticks change from elsewhere — a passkey the room
	// just stored and ticked shows as what it is, not as "not stored".
	const grantedKey = granted.join("\n");
	// biome-ignore lint/correctness/useExhaustiveDependencies: ticks changed elsewhere are what re-read the list.
	useEffect(() => {
		let gone = false;
		wire
			.command("secrets.list", {})
			.then((list) => !gone && setStored(list))
			.catch((error: unknown) => !gone && setNote(refused(UNREAD, error)));
		return () => {
			gone = true;
		};
	}, [grantedKey]);

	const storedNames = stored === null ? null : stored.map((one) => one.name);
	const names = storedNames === null ? granted : [...storedNames, ...granted.filter((name) => !storedNames.includes(name))];
	const sorted = (next: SecretGrant[]) => [...next].sort((left, right) => left.name.localeCompare(right.name));
	const give = (grant: SecretGrant) => {
		setAsking(null);
		onChange(sorted([...grants, grant]));
	};
	const toggle = (name: string, record: SharedSecret | undefined) => {
		if (granted.includes(name)) {
			onChange(grants.filter((grant) => grant.name !== name));
			return;
		}
		if (record?.kind === "variable" && record.usedWith === undefined) {
			setAsking((was) => (was === name ? null : name));
			return;
		}
		give({ name });
	};
	const place = (grant: SecretGrant, where: GrantPlace) => {
		const on = grant.on ?? ["computer", "desk"];
		const next = on.includes(where) ? on.filter((one) => one !== where) : [...on, where];
		// A grant reaches somewhere; untick it to take it away.
		if (next.length === 0) return;
		const { on: _was, ...rest } = grant;
		onChange(grants.map((one) => (one.name === grant.name ? (next.length === 2 ? rest : { ...rest, on: next }) : one)));
	};

	return (
		<div className={`${NESTED} flex-col items-stretch gap-0.5 py-2`}>
			{stored === null && note === null && <span className="group-row-detail">Reading the keychain…</span>}
			{note !== null && <Refusal {...note} />}
			{stored !== null && names.length === 0 && <span className="group-row-detail">None stored yet.</span>}
			{names.map((name) => {
				const record = stored?.find((one) => one.name === name);
				const missing = stored !== null && record === undefined;
				const grant = grants.find((one) => one.name === name);
				return (
					<Fragment key={name}>
						<label className="flex cursor-pointer items-center gap-2.5 py-1.5">
							<input type="checkbox" className="check" checked={grant !== undefined} disabled={disabled} onChange={() => toggle(name, record)} />
							<span className="min-w-0 flex-1">
								<span className="block truncate text-sm">{record !== undefined ? describeShort(record, services, true) : name}</span>
								{record !== undefined && <span className="group-row-detail font-mono">{name}</span>}
								{grant?.readable === true && <span className="group-row-detail block">{READABLE}</span>}
							</span>
							{missing && (
								<span
									className="flex items-center gap-1 text-danger"
									title="Store it in Secrets"
								>
									<WarningIcon />
									<span className="group-row-detail text-danger">not stored</span>
								</span>
							)}
						</label>
						{asking === name && (
							<div className="flex items-center gap-2 pb-1.5 pl-7">
								<span className="min-w-0 flex-1 text-sm text-ink-2">{READABLE}</span>
								<button type="button" className="control btn-quiet btn-sm" disabled={disabled} onClick={() => setAsking(null)}>
									Cancel
								</button>
								<button type="button" className="control btn btn-sm" disabled={disabled} onClick={() => give({ name, readable: true })}>
									Give
								</button>
							</div>
						)}
						{grant !== undefined && bothPlaces && record?.kind === "variable" && (
							<fieldset className="chips min-w-0 pb-1.5 pl-7" aria-label={`Where ${name} goes`}>
								{([["computer", "Its computer"], ["desk", "Its desk shell"]] as const).map(([where, label]) => {
									const on = grant.on === undefined || grant.on.includes(where);
									return (
										<button key={where} type="button" aria-pressed={on} className="nt-chip" data-on={on ? "" : undefined} disabled={disabled} onClick={() => place(grant, where)}>
											{label}
										</button>
									);
								})}
							</fieldset>
						)}
					</Fragment>
				);
			})}
		</div>
	);
}
