import { useEffect, useRef, useState } from "react";
import type {
	BackendChoice,
	CatalogModel,
	ConfigChoice,
	Credential,
	CredentialKind,
	LoginPrompt,
	Provider,
	Report,
} from "../generated/contract";
import { openLink } from "../native";
import { chordKeys } from "../chords";
import { ArrowLeftIcon, GearIcon } from "../icons";
import { mcpServerDetail, type McpHttpAuth, type McpServer } from "../mcp";
import { DEFAULT_IDLE_HOURS, useRoomSettings } from "../room";
import { BackKey, Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { BackendPicker } from "./BackendPicker";
import { PathField } from "./PathField";

const MIN_IDLE_HOURS = 1;
const MAX_IDLE_HOURS = 336;

export type SettingsSection = "general" | "providers" | "tools" | "import";

const SECTIONS: { id: SettingsSection; title: string; detail: string }[] = [
	{ id: "general", title: "General", detail: "Chapters, the default harness, and the default model" },
	{ id: "providers", title: "Providers", detail: "Keys, and the models they unlock" },
	{ id: "tools", title: "Tools", detail: "MCP servers teammates may use" },
	{ id: "import", title: "Import", detail: "A previous Toad's room" },
];

/**
 * The rail while settings are open: the sections stand where the team
 * stood, one row each, and the band carries the way back where the team's
 * plus was. Settings is a place you go, not a card over the room, so the
 * room steps aside until you come back.
 */
export function SettingsRail({
	section,
	onSection,
	onBack,
}: {
	section: SettingsSection;
	onSection(section: SettingsSection): void;
	onBack(): void;
}) {
	return (
		<nav aria-label="Settings" className="rail flex flex-col">
			<Band rail>
				<button
					type="button"
					className="control btn-icon -ml-1"
					title={`Back (${chordKeys("close")})`}
					aria-label="Back to the team"
					onClick={onBack}
				>
					<ArrowLeftIcon />
				</button>
				<h1 className="eyebrow min-w-0 flex-1 truncate pl-1">Settings</h1>
			</Band>
			<div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2 pt-1">
				{SECTIONS.map((one) => (
					<button
						key={one.id}
						type="button"
						className="rail-row"
						aria-current={section === one.id ? "true" : undefined}
						onClick={() => onSection(one.id)}
					>
						<span className="min-w-0 flex-1">
							<span className="block h-[18px] truncate font-medium text-ink">{one.title}</span>
							<span className="block h-4 truncate text-sm text-ink-3">{one.detail}</span>
						</span>
					</button>
				))}
			</div>
		</nav>
	);
}

/**
 * The room's settings, as a pane in the conversation's place: one section
 * at a time, chosen in the rail, each a column of grouped rows. What a
 * teammate is, is not here; that is the teammate's own pane.
 */
export function Settings({ section, onBack }: { section: SettingsSection; onBack?: () => void }) {
	const settings = useRoomSettings();
	const [refusal, setRefusal] = useState<string | null>(null);

	const patch = (patch: Record<string, unknown>) => {
		setRefusal(null);
		void wire.command("settings.update", { patch }).catch((error: Error) => setRefusal(error.message));
	};

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">
					{SECTIONS.find((one) => one.id === section)?.title}
				</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{section === "general" && (
						<GeneralSection
							idleHours={settings.chapterIdleHours}
							defaultBackendId={settings.defaultBackendId}
							defaultModelId={settings.defaultModelId}
							onIdleHours={(hours) => patch({ chapterIdleHours: hours })}
							onBackend={(id) => patch({ defaultBackendId: id })}
							onDefaultModel={(id) => patch({ defaultModelId: id })}
						/>
					)}
					{section === "providers" && (
						<ProvidersSection enabledModels={settings.enabledModels} onRefuse={setRefusal} />
					)}
					{section === "tools" && <ToolsSection servers={settings.mcpServers} onRefuse={setRefusal} />}
					{section === "import" && <ImportSection onRefuse={setRefusal} />}
					{refusal !== null && (
						<p role="status" className="selectable text-sm text-danger">
							{refusal}
						</p>
					)}
				</div>
			</Scroll>
		</div>
	);
}

function GeneralSection({
	idleHours,
	defaultBackendId,
	defaultModelId,
	onIdleHours,
	onBackend,
	onDefaultModel,
}: {
	idleHours: number;
	defaultBackendId: string;
	defaultModelId: string | null;
	onIdleHours(hours: number): void;
	onBackend(id: string): void;
	onDefaultModel(id: string | null): void;
}) {
	const [hours, setHours] = useState(String(idleHours));
	const [backends, setBackends] = useState<BackendChoice[]>([]);
	const [models, setModels] = useState<ConfigChoice[]>([]);

	useEffect(() => {
		setHours(String(idleHours));
	}, [idleHours]);

	useEffect(() => {
		void wire
			.command("backends.list", {})
			.then(setBackends)
			.catch(() => setBackends([]));
		void wire
			.command("models.list", {})
			.then(setModels)
			.catch(() => setModels([]));
	}, []);

	const commitHours = (raw: string) => {
		setHours(raw);
		const next = Number(raw);
		if (!Number.isInteger(next) || next < MIN_IDLE_HOURS || next > MAX_IDLE_HOURS) return;
		if (next !== idleHours) onIdleHours(next);
	};

	return (
		<>
			<section>
				<h3 className="group-title">Chapters</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="group-row-text" htmlFor="setting-idle">
							<span className="group-row-title">Close a chapter after</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								How long a teammate sits quiet before its working context closes.
							</span>
						</label>
						<span className="flex items-center gap-2 text-sm text-ink-2">
							<input
								id="setting-idle"
								type="number"
								className="field w-16 text-right"
								min={MIN_IDLE_HOURS}
								max={MAX_IDLE_HOURS}
								step={1}
								value={hours}
								onChange={(event) => commitHours(event.target.value)}
							/>
							hours
						</span>
					</div>
				</div>
				<p className="group-hint">Eight hours is a night&rsquo;s sleep. The default is {DEFAULT_IDLE_HOURS}.</p>
			</section>
			<section>
				<h3 className="group-title" id="setting-backend">
					New teammates run on
				</h3>
				{backends.length > 0 ? (
					<BackendPicker
						backends={backends}
						selected={defaultBackendId}
						name="setting-backend"
						labelledBy="setting-backend"
						onSelect={onBackend}
					/>
				) : (
					<div className="grouped">
						<p className="group-row text-sm text-ink-3">Reading which harnesses this machine can start…</p>
					</div>
				)}
				<p className="group-hint">The new-teammate form can still pick another.</p>
			</section>
			<section>
				<h3 className="group-title">Default model</h3>
				<div className="grouped">
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">Toad Agent starts on</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								A new teammate takes this when its draft leaves the model blank.
							</span>
						</span>
						<Picker
							value={defaultModelId ?? ""}
							choices={[{ id: "", name: "Last used" }, ...models]}
							placeholder="Last used"
							label="Default model"
							onChange={(id) => onDefaultModel(id === "" ? null : id)}
						/>
					</div>
				</div>
				<p className="group-hint">Clearing it falls back to the last model a Toad Agent teammate ran on.</p>
			</section>
		</>
	);
}

function byName(a: { name: string }, b: { name: string }) {
	return a.name.localeCompare(b.name);
}

/** The live oauth credential for a provider, or the revoked one if that is all that is held. */
function oauthHeld(held: Credential[] | null, providerId: string): Credential | undefined {
	const of = (held ?? []).filter((one) => one.providerId === providerId && one.credentialKind === "oauth");
	return of.find((one) => !one.revoked) ?? of[0];
}

function ProvidersSection({
	enabledModels,
	onRefuse,
}: {
	enabledModels: Record<string, string[]>;
	onRefuse(message: string | null): void;
}) {
	const [held, setHeld] = useState<Credential[] | null>(null);
	/* The providers a credential can be for come from the core's model
	 * catalogue, so a provider added there is offered here without the
	 * window knowing its name. The first key provider, sorted by name, is
	 * the default until the person picks. */
	const [providers, setProviders] = useState<Provider[]>([]);
	const [providerId, setProviderId] = useState("");
	const [secret, setSecret] = useState("");
	const [busy, setBusy] = useState(false);
	const [login, setLogin] = useState<{ providerId: string; prompt: LoginPrompt } | null>(null);
	const [filterId, setFilterId] = useState<string | null>(null);

	useEffect(() => {
		wire
			.command("credential.list", {})
			.then(setHeld)
			.catch((error: Error) => {
				setHeld([]);
				onRefuse(error.message);
			});
		wire
			.command("providers.list", {})
			.then((list) => {
				const keys = list.filter((one) => one.credentialKind === "api_key").slice().sort(byName);
				setProviders(list);
				setProviderId((current) => current || (keys[0]?.id ?? ""));
			})
			.catch((error: Error) => onRefuse(error.message));
	}, [onRefuse]);

	useEffect(() => {
		if (login === null) return;
		let cancelled = false;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const tick = () => {
			void wire
				.command("credential.login_status", { loginId: login.prompt.loginId })
				.then((status) => {
					if (cancelled) return;
					if (status.state === "done") {
						if (status.credential) {
							setHeld((known) => [...(known ?? []), status.credential!]);
						}
						setLogin(null);
						setBusy(false);
						return;
					}
					if (status.state === "failed") {
						onRefuse(status.error ?? "Sign-in failed.");
						setLogin(null);
						setBusy(false);
						return;
					}
					timer = setTimeout(tick, 2000);
				})
				.catch((error: Error) => {
					if (cancelled) return;
					onRefuse(error.message);
					setLogin(null);
					setBusy(false);
				});
		};
		timer = setTimeout(tick, 2000);
		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [login, onRefuse]);

	const keyProviders = providers.filter((one) => one.credentialKind === "api_key").slice().sort(byName);
	const oauthProviders = providers.filter((one) => one.credentialKind === "oauth").slice().sort(byName);
	const keys = (held ?? []).filter((one) => one.credentialKind === "api_key");
	const picked = keyProviders.find((one) => one.id === providerId);

	const save = async () => {
		if (!secret.trim() || !providerId || busy) return;
		setBusy(true);
		onRefuse(null);
		try {
			const label = picked?.name ?? providerId;
			const made = await wire.command("credential.create", { providerId, label, secret: secret.trim() });
			setHeld((known) => [...(known ?? []), made]);
			setSecret("");
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const signIn = async (id: string) => {
		if (busy) return;
		setBusy(true);
		onRefuse(null);
		try {
			setLogin({ providerId: id, prompt: await wire.command("credential.login", { providerId: id }) });
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	if (filterId !== null) {
		const named = providers.find((one) => one.id === filterId);
		return (
			<ModelsShown
				providerId={filterId}
				providerName={named?.name ?? filterId}
				credentialKind={named?.credentialKind ?? "api_key"}
				enabledModels={enabledModels}
				onCancel={() => setFilterId(null)}
				onRefuse={onRefuse}
			/>
		);
	}

	return (
		<>
			<section>
				<h3 className="group-title">Keys</h3>
				<div className="grouped">
					{held === null ? (
						<p className="group-row text-sm text-ink-3">Reading…</p>
					) : keys.length === 0 ? (
						<p className="group-row text-sm text-ink-3">No keys yet. Toad Agent needs one to run a model.</p>
					) : (
						keys.map((one) => (
							<div key={one.id} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{one.label}</span>
									<span className="group-row-detail font-mono">{one.providerId}</span>
								</span>
								<span className="flex items-center gap-2">
									{!one.revoked && (
										<button
											type="button"
											className="control btn-icon"
											title="Models shown"
											aria-label={`Models shown — ${one.label}`}
											onClick={() => setFilterId(one.providerId)}
										>
											<GearIcon />
										</button>
									)}
									<span className={`text-sm ${one.revoked ? "text-ink-3" : "text-ink-2"}`}>
										{one.revoked ? "Revoked" : "In use"}
									</span>
								</span>
							</div>
						))
					)}
				</div>
				<p className="group-hint">A key never leaves this machine; the room remembers only that it exists.</p>
			</section>
			<form
				onSubmit={(event) => {
					event.preventDefault();
					void save();
				}}
			>
				<h3 className="group-title">Add a key</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" id="key-provider">
							Provider
						</label>
						<div className="flex-1">
							<Picker
								field
								value={providerId}
								choices={keyProviders}
								placeholder="Provider"
								label="Provider"
								onChange={setProviderId}
							/>
						</div>
					</div>
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="key-secret">
							API key
						</label>
						<input
							id="key-secret"
							type="password"
							className="field flex-1 font-mono text-sm"
							spellCheck={false}
							autoComplete="off"
							value={secret}
							onChange={(event) => setSecret(event.target.value)}
						/>
					</div>
					<div className="group-row justify-end">
						<button
							type="submit"
							className="control btn-primary"
							disabled={busy || !providerId || secret.trim() === ""}
						>
							{busy ? "Saving…" : "Save key"}
						</button>
					</div>
				</div>
			</form>
			<section>
				<h3 className="group-title">Sign in with</h3>
				<div className="grouped">
					{held === null || providers.length === 0 ? (
						<p className="group-row text-sm text-ink-3">Reading…</p>
					) : (
						oauthProviders.flatMap((provider) => {
							const cred = oauthHeld(held, provider.id);
							const prompt = login?.providerId === provider.id ? login.prompt : null;
							if (prompt !== null) {
								return [
									<div key={provider.id} className="group-row">
										<span className="group-row-text">
											<span className="group-row-title">{provider.name}</span>
										</span>
									</div>,
									<div key={`${provider.id}-code`} className="group-row">
										<span className="selectable font-mono text-xl tracking-wide">{prompt.userCode}</span>
									</div>,
									<div key={`${provider.id}-link`} className="group-row">
										<button
											type="button"
											className="text-sm text-ink-2 underline"
											onClick={() => void openLink(prompt.verificationUri)}
										>
											{prompt.verificationUri}
										</button>
									</div>,
									<p key={`${provider.id}-wait`} className="group-row text-sm text-ink-3">
										Waiting for you to sign in…
									</p>,
								];
							}
							const signedIn = cred !== undefined && !cred.revoked;
							const revoked = cred !== undefined && cred.revoked;
							return [
								<div key={provider.id} className="group-row">
									<span className="group-row-text">
										<span className="group-row-title">{provider.name}</span>
									</span>
									<span className="flex items-center gap-2">
										{signedIn && (
											<button
												type="button"
												className="control btn-icon"
												title="Models shown"
												aria-label={`Models shown — ${provider.name}`}
												onClick={() => setFilterId(provider.id)}
											>
												<GearIcon />
											</button>
										)}
										{signedIn ? (
											<span className="text-sm text-ink-2">Signed in</span>
										) : (
											<>
												{revoked && <span className="text-sm text-ink-3">Revoked</span>}
												<button
													type="button"
													className="control btn-primary"
													disabled={busy}
													onClick={() => void signIn(provider.id)}
												>
													Sign in
												</button>
											</>
										)}
									</span>
								</div>,
							];
						})
					)}
				</div>
			</section>
		</>
	);
}

/**
 * The filter for one provider, in the pane's place. Every id checked is the
 * same as no filter, so Save removes the provider's entry rather than writing
 * a list that means "all".
 */
function ModelsShown({
	providerId,
	providerName,
	credentialKind,
	enabledModels,
	onCancel,
	onRefuse,
}: {
	providerId: string;
	providerName: string;
	credentialKind: CredentialKind;
	enabledModels: Record<string, string[]>;
	onCancel(): void;
	onRefuse(message: string | null): void;
}) {
	const [catalog, setCatalog] = useState<CatalogModel[] | null>(null);
	const [query, setQuery] = useState("");
	const [on, setOn] = useState<Set<string>>(new Set());
	const [busy, setBusy] = useState(false);

	const applyCatalog = (list: CatalogModel[]) => {
		setCatalog(list);
		setOn(new Set(list.filter((model) => model.enabled).map((model) => model.id)));
	};

	useEffect(() => {
		wire
			.command("models.catalog", { providerId })
			.then(applyCatalog)
			.catch((error: Error) => {
				onRefuse(error.message);
				setCatalog([]);
			});
	}, [providerId, onRefuse]);

	const refresh = async () => {
		if (busy) return;
		setBusy(true);
		onRefuse(null);
		try {
			applyCatalog(await wire.command("credential.refresh_models", { providerId }));
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const save = async () => {
		if (catalog === null || busy) return;
		setBusy(true);
		onRefuse(null);
		const next: Record<string, string[]> = { ...enabledModels };
		if (on.size === catalog.length && catalog.every((model) => on.has(model.id))) {
			delete next[providerId];
		} else {
			next[providerId] = catalog.filter((model) => on.has(model.id)).map((model) => model.id);
		}
		try {
			await wire.command("settings.update", { patch: { enabledModels: next } });
			onCancel();
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const needle = query.trim().toLowerCase();
	const visible =
		catalog === null
			? []
			: needle === ""
				? catalog
				: catalog.filter(
						(model) => model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle),
					);
	const shown = catalog?.length ?? 0;

	return (
		<section>
			<h3 className="group-title">Models shown — {providerName}</h3>
			<div className="grouped">
				<div className="group-row">
					<input
						type="search"
						className="field flex-1"
						placeholder="Search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						aria-label="Search models"
					/>
					<button
						type="button"
						className="control btn-quiet"
						disabled={catalog === null}
						onClick={() => catalog && setOn(new Set(catalog.map((model) => model.id)))}
					>
						All
					</button>
					<button type="button" className="control btn-quiet" disabled={catalog === null} onClick={() => setOn(new Set())}>
						None
					</button>
					{credentialKind === "oauth" && (
						<button
							type="button"
							className="control btn-quiet"
							disabled={catalog === null || busy}
							onClick={() => void refresh()}
						>
							Refresh
						</button>
					)}
				</div>
				<p className="group-row text-sm text-ink-3">
					{catalog === null ? "Reading…" : `${on.size} of ${shown} shown`}
				</p>
				{visible.map((model) => (
					<label key={model.id} className="group-row group-row-choice">
						<span className="group-row-text">
							<span className="group-row-title">{model.name}</span>
							<span className="group-row-detail font-mono">{model.id}</span>
						</span>
						<input
							type="checkbox"
							className="check"
							checked={on.has(model.id)}
							onChange={(event) => {
								setOn((known) => {
									const next = new Set(known);
									if (event.target.checked) next.add(model.id);
									else next.delete(model.id);
									return next;
								});
							}}
						/>
					</label>
				))}
				<div className="group-row justify-end">
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
						Cancel
					</button>
					<button
						type="button"
						className="control btn-primary"
						disabled={busy || catalog === null}
						onClick={() => void save()}
					>
						{busy ? "Saving…" : "Save"}
					</button>
				</div>
			</div>
		</section>
	);
}

type ServerDraft = { name: string; kind: "stdio" | "http"; command: string; url: string };

const EMPTY_DRAFT: ServerDraft = { name: "", kind: "stdio", command: "", url: "" };

/**
 * Servers are defined once for the room. Which teammate may use them is a
 * different question, answered on that teammate.
 */
function ToolsSection({
	servers,
	onRefuse,
}: {
	servers: McpServer[];
	onRefuse(message: string | null): void;
}) {
	const [draft, setDraft] = useState<ServerDraft>(EMPTY_DRAFT);
	const [editingId, setEditingId] = useState<string | null>(null);
	const [writing, setWriting] = useState(false);
	const nameField = useRef<HTMLInputElement>(null);

	const ready =
		draft.name.trim().length > 0 &&
		(draft.kind === "stdio" ? draft.command.trim().length > 0 : draft.url.trim().length > 0);

	const persist = async (next: McpServer[]): Promise<boolean> => {
		setWriting(true);
		onRefuse(null);
		try {
			await wire.command("settings.update", { patch: { mcpServers: next } });
			return true;
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
			return false;
		} finally {
			setWriting(false);
		}
	};

	const reset = () => {
		setDraft(EMPTY_DRAFT);
		setEditingId(null);
	};

	const save = () => {
		if (!ready || writing) return;
		const name = draft.name.trim();
		const previous = editingId ? servers.find((one) => one.id === editingId) : undefined;
		const next =
			draft.kind === "stdio"
				? stdioFromDraft(name, draft.command, previous)
				: httpFromDraft(name, draft.url, previous);
		const list = editingId ? servers.map((one) => (one.id === editingId ? next : one)) : [...servers, next];
		void persist(list).then((ok) => ok && reset());
	};

	const startEdit = (server: McpServer) => {
		setEditingId(server.id);
		setDraft(
			server.type === "stdio"
				? { name: server.name, kind: "stdio", command: [server.command, ...server.args].join(" "), url: "" }
				: { name: server.name, kind: "http", command: "", url: server.url },
		);
		nameField.current?.focus();
	};

	return (
		<>
			<section>
				<h3 className="group-title">MCP servers</h3>
				<div className="grouped">
					{servers.length === 0 ? (
						<p className="group-row text-sm text-ink-3">
							No servers yet. A teammate runs with its agent&rsquo;s own tools until you add one.
						</p>
					) : (
						servers.map((server) => (
							<div key={server.id} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">
										{server.name}
										<span className="ml-2 text-sm text-ink-3">{server.type === "stdio" ? "Command" : "HTTP"}</span>
									</span>
									<span className="group-row-detail font-mono">{mcpServerDetail(server)}</span>
								</span>
								<button
									type="button"
									className="control btn-quiet btn-sm"
									aria-label={`Edit ${server.name}`}
									disabled={writing}
									onClick={() => startEdit(server)}
								>
									Edit
								</button>
								<button
									type="button"
									className="control btn-quiet btn-sm btn-danger"
									aria-label={`Remove ${server.name}`}
									disabled={writing}
									onClick={() =>
										void persist(servers.filter((one) => one.id !== server.id)).then((ok) => {
											if (ok && editingId === server.id) reset();
										})
									}
								>
									Remove
								</button>
							</div>
						))
					)}
				</div>
				<p className="group-hint">Which teammates may use a server is set on each teammate.</p>
			</section>
			<form
				onSubmit={(event) => {
					event.preventDefault();
					save();
				}}
			>
				<h3 className="group-title">{editingId ? "Edit server" : "Add a server"}</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2">Type</label>
						<div className="flex-1">
							<Picker
								field
								value={draft.kind}
								choices={[
									{ id: "stdio", name: "Command", detail: "Started on this machine and spoken to over stdio" },
									{ id: "http", name: "HTTP", detail: "Reached at a URL" },
								]}
								placeholder="Type"
								label="Server type"
								onChange={(kind) => setDraft({ ...draft, kind: kind === "http" ? "http" : "stdio" })}
							/>
						</div>
					</div>
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-name">
							Name
						</label>
						<input
							id="tool-name"
							ref={nameField}
							className="field flex-1"
							value={draft.name}
							onChange={(event) => setDraft({ ...draft, name: event.target.value })}
						/>
					</div>
					{draft.kind === "stdio" ? (
						<div className="group-row">
							<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-command">
								Command
							</label>
							<input
								id="tool-command"
								className="field flex-1 font-mono text-sm"
								spellCheck={false}
								placeholder="npx -y @modelcontextprotocol/server-filesystem /some/path"
								value={draft.command}
								onChange={(event) => setDraft({ ...draft, command: event.target.value })}
							/>
						</div>
					) : (
						<div className="group-row">
							<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-url">
								URL
							</label>
							<input
								id="tool-url"
								className="field flex-1 font-mono text-sm"
								spellCheck={false}
								placeholder="https://example.com/mcp"
								value={draft.url}
								onChange={(event) => setDraft({ ...draft, url: event.target.value })}
							/>
						</div>
					)}
					<div className="group-row justify-end">
						{editingId !== null && (
							<button type="button" className="control btn" onClick={reset}>
								Cancel
							</button>
						)}
						<button type="submit" className="control btn-primary" disabled={writing || !ready}>
							{editingId ? "Save" : "Add server"}
						</button>
					</div>
				</div>
				<p className="group-hint">Sign-in and headers for HTTP servers come later.</p>
			</form>
		</>
	);
}

/** The form does not edit env, so an edit of a stdio server keeps the map it already had. */
function stdioFromDraft(name: string, commandLine: string, previous?: McpServer): McpServer {
	const [command, ...args] = commandLine.trim().split(/\s+/);
	const id = previous?.id ?? crypto.randomUUID();
	const env = previous?.type === "stdio" ? previous.env : undefined;
	return env
		? { id, type: "stdio", name, command: command ?? "", args, env }
		: { id, type: "stdio", name, command: command ?? "", args };
}

/** A new HTTP server is none; an edit keeps whatever auth was already stored. */
function httpFromDraft(name: string, url: string, previous?: McpServer): McpServer {
	const auth: McpHttpAuth = previous?.type === "http" ? previous.auth : { mode: "none" };
	return { id: previous?.id ?? crypto.randomUUID(), type: "http", name, url: url.trim(), auth };
}

function ImportSection({ onRefuse }: { onRefuse(message: string | null): void }) {
	const [from, setFrom] = useState(previousToadDir);
	const [report, setReport] = useState<Report | null>(null);
	const [busy, setBusy] = useState(false);

	const run = async () => {
		const path = from.trim();
		if (!path || busy) return;
		setBusy(true);
		onRefuse(null);
		setReport(null);
		try {
			setReport(await wire.command("room.import", { from: path }));
		} catch (error) {
			onRefuse(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<>
			<section>
				<h3 className="group-title">Bring over a previous Toad</h3>
				<div className="grouped">
					<div className="group-row flex-col items-stretch gap-1.5">
						<label className="label mb-0" htmlFor="import-from">
							Its data directory
						</label>
						<PathField id="import-from" value={from} onChange={setFrom} />
					</div>
					<div className="group-row justify-end">
						<button type="button" className="control btn-primary" disabled={busy || from.trim() === ""} onClick={() => void run()}>
							{busy ? "Importing…" : "Import"}
						</button>
					</div>
				</div>
				<p className="group-hint">
					Teammates, their conversations, the peer threads those conversations name, schedules, settings and keys are
					copied. The other Toad is never written to. A teammate keeps its working directory inside the old data
					directory; deleting that directory takes those workspaces with it.
				</p>
			</section>
			{report !== null && (
				<section>
					<h3 className="group-title">Imported</h3>
					<div className="grouped">
						<div className="group-row">
							<span className="group-row-text">
								<span className="group-row-title">
									{count(report.teammates, "teammate")} · {count(report.tapes, "conversation")} ·{" "}
									{count(report.threads, "thread")} · {count(report.schedules, "schedule")} ·{" "}
									{count(report.settings, "setting")} · {count(report.keys, "key")}
								</span>
							</span>
						</div>
						{report.notes.map((one, index) => (
							<div key={`note:${one.item}:${index}`} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title font-mono text-sm">{one.item}</span>
									<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
										Note: {one.reason}
									</span>
								</span>
							</div>
						))}
						{report.skipped.map((one, index) => (
							<div key={`skipped:${one.item}:${index}`} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title font-mono text-sm">{one.item}</span>
									<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
										Skipped: {one.reason}
									</span>
								</span>
							</div>
						))}
					</div>
				</section>
			)}
		</>
	);
}

function count(n: number, noun: string): string {
	return `${n} ${noun}${n === 1 ? "" : "s"}`;
}

/**
 * Where the previous Toad keeps its data. The window does not know `$HOME`,
 * so this is the path that edition uses, written the way a person would type
 * it. The core receives the string as typed.
 */
function previousToadDir(): string {
	const here = window.__toadDesk?.platform ?? "linux";
	if (here === "macos") return "~/Library/Application Support/Toad";
	if (here === "windows") return "~/AppData/Roaming/Toad";
	return "~/.local/share/toad";
}
