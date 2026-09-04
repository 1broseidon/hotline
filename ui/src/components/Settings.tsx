import { useEffect, useRef, useState } from "react";
import type {
	BackendChoice,
	CatalogModel,
	ComputerRuntime,
	ConfigChoice,
	Credential,
	LoginPrompt,
	Provider,
	Report,
	RuntimeReport,
} from "../generated/contract";
import { openLink } from "../native";
import { chordKeys } from "../chords";
import { ArrowLeftIcon, ChevronRightIcon, PlusIcon } from "../icons";
import { mcpServerDetail, type McpHttpAuth, type McpServer } from "../mcp";
import type { McpOAuthStatus } from "../wire";
import { DEFAULT_IDLE_HOURS, useRoomSettings } from "../room";
import { BackKey, Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { BackendPicker } from "./BackendPicker";
import { PathField } from "./PathField";

const MIN_IDLE_HOURS = 1;
const MAX_IDLE_HOURS = 336;

export type SettingsSection = "general" | "providers" | "tools" | "computer" | "import";

const SECTIONS: { id: SettingsSection; title: string; detail: string }[] = [
	{ id: "general", title: "General", detail: "Chapters, the default harness, and the default model" },
	{ id: "providers", title: "Providers", detail: "What Toad Agent can run a model on" },
	{ id: "tools", title: "Tools", detail: "MCP gateway for your teammates" },
	{ id: "computer", title: "Computer", detail: "The desktop a teammate can be given" },
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

	// Providers and Tools have a page under them, so their bands are their own.
	if (section === "providers") return <ProvidersSection enabledModels={settings.enabledModels} onBack={onBack} />;
	if (section === "tools") return <ToolsSection servers={settings.mcpServers} onBack={onBack} />;

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
					{section === "computer" && (
						<ComputerSection
							runtime={settings.computerRuntime}
							image={settings.computerImage}
							onRuntime={(id) => patch({ computerRuntime: id })}
							onImage={(image) => patch({ computerImage: image })}
						/>
					)}
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

const RUNTIME_NAMES: Record<ComputerRuntime, string> = {
	docker: "Docker",
	podman: "Podman",
	container: "Apple container",
};

/**
 * Computer: which container runtime wakes a teammate's desktop, and which
 * image it wakes. Every runtime this machine could have is a row, the
 * missing ones greyed with the sentence that names what is missing, so
 * "no computer" is never a mystery. Automatic is the room's default and
 * leaves the pick to the desk, rootless first.
 */
function ComputerSection({
	runtime,
	image,
	onRuntime,
	onImage,
}: {
	runtime: string | null;
	image: string | null;
	onRuntime(id: string | null): void;
	onImage(image: string | null): void;
}) {
	const [reports, setReports] = useState<RuntimeReport[] | undefined>(undefined);
	const [draft, setDraft] = useState(image ?? "");

	useEffect(() => {
		setDraft(image ?? "");
	}, [image]);

	useEffect(() => {
		void wire
			.command("computer.runtimes", {})
			.then(setReports)
			.catch(() => setReports([]));
	}, []);

	const commitImage = () => {
		const trimmed = draft.trim();
		setDraft(trimmed);
		const next = trimmed === "" ? null : trimmed;
		if (next !== image) onImage(next);
	};

	const chosen = runtime ?? "";
	const rows: { id: string; name: string; detail: string; off: boolean }[] = [
		{ id: "", name: "Automatic", detail: "The first runtime that answers, rootless before a root daemon.", off: false },
		...(reports ?? []).map((one) => ({
			id: one.runtime,
			name: RUNTIME_NAMES[one.runtime],
			detail: one.available ? (one.rootless ? "Ready, rootless." : "Ready.") : (one.reason ?? "Not available."),
			off: !one.available,
		})),
	];

	return (
		<>
			<section>
				<h3 className="group-title" id="setting-computer-runtime">
					Runs on
				</h3>
				{reports === undefined ? (
					<div className="grouped">
						<p className="group-row text-sm text-ink-3">Looking for a container runtime…</p>
					</div>
				) : (
					<div role="radiogroup" aria-labelledby="setting-computer-runtime" className="grouped">
						{rows.map((row) => (
							<label key={row.id} className="group-row group-row-choice" data-off={row.off ? "true" : undefined}>
								<input
									type="radio"
									className="radio"
									name="setting-computer-runtime"
									checked={chosen === row.id}
									aria-disabled={row.off ? true : undefined}
									onChange={() => {
										if (row.off) return;
										onRuntime(row.id === "" ? null : row.id);
									}}
								/>
								<span className="group-row-text">
									<span className="group-row-title">{row.name}</span>
									<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
										{row.detail}
									</span>
								</span>
							</label>
						))}
					</div>
				)}
				<p className="group-hint">
					A computer is a Linux desktop in a container, one per teammate that asks for one. Nothing wakes until a
					teammate with one starts.
				</p>
			</section>
			<section>
				<h3 className="group-title">Image</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="group-row-text" htmlFor="setting-computer-image">
							<span className="group-row-title">Desktop image</span>
							<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
								Blank is the image pinned to this version of Toad.
							</span>
						</label>
						<input
							id="setting-computer-image"
							className="field w-56 min-w-0 font-mono text-sm"
							placeholder="Pinned default"
							autoComplete="off"
							spellCheck={false}
							value={draft}
							onChange={(event) => setDraft(event.target.value)}
							onBlur={commitImage}
							onKeyDown={(event) => {
								if (event.key !== "Enter") return;
								event.preventDefault();
								commitImage();
							}}
						/>
					</div>
				</div>
				<p className="group-hint">A teammate&rsquo;s own image, set in its pane, wins over this one.</p>
			</section>
		</>
	);
}

function byName(a: { name: string }, b: { name: string }) {
	return a.name.localeCompare(b.name);
}

/**
 * Providers: one list of what this desk can run a model on. A row is a
 * connected provider, however it was connected — a pasted key or a login
 * is a detail of the row, not a heading — and pressing it opens that
 * provider's own page. The add row at the head of the list is the one way
 * in: it names the providers not yet connected, and choosing one asks for
 * a key or starts a sign-in in place. The form exists only while you are
 * adding. Tools is drawn the same way, on purpose: a list the room has,
 * an add row over it, a page behind each row with the way out at its foot.
 *
 * The pane is this section's own rather than Settings', because it has
 * a page under it and the band has to say which one you are on.
 */
function ProvidersSection({
	enabledModels,
	onBack,
}: {
	enabledModels: Record<string, string[]>;
	onBack?: (() => void) | undefined;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [held, setHeld] = useState<Credential[] | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	/* Adding: the provider chosen from the plus, and for a login provider
	 * the prompt once the core has one. */
	const [choosing, setChoosing] = useState(false);
	const [adding, setAdding] = useState<Provider | null>(null);
	const [login, setLogin] = useState<{ providerId: string; prompt: LoginPrompt } | null>(null);
	const [busy, setBusy] = useState(false);
	const [open, setOpen] = useState<string | null>(null);

	useEffect(() => {
		wire
			.command("credential.list", {})
			.then(setHeld)
			.catch((error: Error) => {
				setHeld([]);
				setRefusal(error.message);
			});
		wire
			.command("providers.list", {})
			.then(setProviders)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

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
							const made = status.credential;
							setHeld((known) => [...(known ?? []).filter((one) => one.id !== made.id), made]);
						}
						setLogin(null);
						setAdding(null);
						setBusy(false);
						return;
					}
					if (status.state === "failed") {
						setRefusal(status.error ?? "Sign-in failed.");
						setLogin(null);
						setAdding(null);
						setBusy(false);
						return;
					}
					timer = setTimeout(tick, 2000);
				})
				.catch((error: Error) => {
					if (cancelled) return;
					setRefusal(error.message);
					setLogin(null);
					setAdding(null);
					setBusy(false);
				});
		};
		timer = setTimeout(tick, 2000);
		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [login]);

	/* Connected: one row per credential, live ones first. A revoked login
	 * stays listed so it can be signed in again or removed. */
	const connected = (held ?? [])
		.map((credential) => ({ credential, provider: providers.find((one) => one.id === credential.providerId) }))
		.sort((a, b) => Number(a.credential.revoked) - Number(b.credential.revoked) || nameOf(a).localeCompare(nameOf(b)));
	const live = new Set(connected.filter((one) => !one.credential.revoked).map((one) => one.credential.providerId));
	const addable = providers.filter((one) => !live.has(one.id)).slice().sort(byName);

	const begin = (provider: Provider) => {
		setRefusal(null);
		setChoosing(false);
		setAdding(provider);
		if (provider.credentialKind === "oauth") void signIn(provider.id);
	};

	const signIn = async (id: string) => {
		if (busy) return;
		setBusy(true);
		try {
			setLogin({ providerId: id, prompt: await wire.command("credential.login", { providerId: id }) });
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setAdding(null);
			setBusy(false);
		}
	};

	const saveKey = async (secret: string) => {
		if (adding === null || busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			const made = await wire.command("credential.create", { providerId: adding.id, label: adding.name, secret });
			setHeld((known) => [...(known ?? []), made]);
			setAdding(null);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const opened = connected.find((one) => one.credential.id === open);
	if (opened !== undefined) {
		return (
			<ProviderPage
				credential={opened.credential}
				name={nameOf(opened)}
				enabledModels={enabledModels}
				onBack={() => setOpen(null)}
				onSignIn={() => {
					setOpen(null);
					if (opened.provider) begin(opened.provider);
				}}
				onRemoved={() => {
					setHeld((known) => (known ?? []).filter((one) => one.id !== opened.credential.id));
					setOpen(null);
				}}
			/>
		);
	}

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Providers</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{choosing && (
						<section>
							<h3 className="group-title">Add provider</h3>
							<div className="grouped">
								{addable.length === 0 ? (
									<p className="group-row text-sm text-ink-3">Every provider Toad knows is already here.</p>
								) : (
									addable.map((provider) => (
										<button
											key={provider.id}
											type="button"
											className="group-row group-row-choice w-full text-left"
											disabled={busy}
											onClick={() => begin(provider)}
										>
											<span className="group-row-text">
												<span className="group-row-title">{provider.name}</span>
											</span>
											<span className="text-sm text-ink-3">{provider.credentialKind === "oauth" ? "Sign in" : "API key"}</span>
											<ChevronRightIcon className="shrink-0 text-ink-3" />
										</button>
									))
								)}
								<div className="group-row justify-end">
									<button type="button" className="control btn-quiet" onClick={() => setChoosing(false)}>
										Cancel
									</button>
								</div>
							</div>
							<p className="group-hint">A key is pasted; a sign-in opens the provider's page with a code.</p>
						</section>
					)}
					{adding !== null && adding.credentialKind === "api_key" && (
						<KeyForm provider={adding} busy={busy} onSave={(secret) => void saveKey(secret)} onCancel={() => setAdding(null)} />
					)}
					{adding !== null && adding.credentialKind === "oauth" && (
						<section>
							<h3 className="group-title">Sign in to {adding.name}</h3>
							<div className="grouped">
								{login === null ? (
									<p className="group-row text-sm text-ink-3">Asking {adding.name} for a code…</p>
								) : (
									<>
										<div className="group-row">
											<span className="selectable font-mono text-xl tracking-wide">{login.prompt.userCode}</span>
										</div>
										<div className="group-row">
											<button
												type="button"
												className="text-sm text-ink-2 underline"
												onClick={() => void openLink(login.prompt.verificationUri)}
											>
												{login.prompt.verificationUri}
											</button>
										</div>
										<p className="group-row text-sm text-ink-3">Waiting for you to sign in…</p>
									</>
								)}
								<div className="group-row justify-end">
									<button
										type="button"
										className="control btn-quiet"
										onClick={() => {
											setLogin(null);
											setAdding(null);
											setBusy(false);
										}}
									>
										Cancel
									</button>
								</div>
							</div>
							<p className="group-hint">Enter the code on that page. Toad keeps the login on this machine.</p>
						</section>
					)}
					<section>
						<div className="grouped">
							{adding === null && !choosing && (
								<button type="button" className="group-row group-row-add" onClick={() => setChoosing(true)}>
									<PlusIcon />
									Add provider
								</button>
							)}
							{held === null ? (
								<p className="group-row text-sm text-ink-3">Reading…</p>
							) : connected.length === 0 ? (
								<p className="group-row text-sm text-ink-3">No providers yet. Toad Agent needs one to run a model.</p>
							) : (
								connected.map((one) => (
									<button
										key={one.credential.id}
										type="button"
										className="group-row group-row-choice w-full text-left"
										onClick={() => setOpen(one.credential.id)}
									>
										<span className="group-row-text">
											<span className="group-row-title">{nameOf(one)}</span>
											<span className="group-row-detail">
												{one.credential.revoked
													? one.credential.credentialKind === "oauth"
														? "Signed out"
														: "Key revoked"
													: `${one.credential.credentialKind === "oauth" ? "Signed in" : "API key"} · ${modelsShownText(enabledModels[one.credential.providerId])}`}
											</span>
										</span>
										<ChevronRightIcon className="shrink-0 text-ink-3" />
									</button>
								))
							)}
						</div>
						<p className="group-hint">A key or a login never leaves this machine; the room remembers only that it exists.</p>
					</section>
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

function nameOf(one: { credential: Credential; provider: Provider | undefined }): string {
	return one.provider?.name ?? one.credential.label;
}

/** The row's own word for the filter: the count when there is one, else all. */
function modelsShownText(enabled: string[] | undefined): string {
	if (enabled === undefined) return "All models";
	return enabled.length === 1 ? "1 model" : `${enabled.length} models`;
}

/** The key field, in place, for the provider just chosen from the plus. */
function KeyForm({
	provider,
	busy,
	onSave,
	onCancel,
}: {
	provider: Provider;
	busy: boolean;
	onSave(secret: string): void;
	onCancel(): void;
}) {
	const [secret, setSecret] = useState("");
	const field = useRef<HTMLInputElement>(null);
	useEffect(() => field.current?.focus(), []);
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (secret.trim() !== "") onSave(secret.trim());
			}}
		>
			<h3 className="group-title">Add {provider.name}</h3>
			<div className="grouped">
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="key-secret">
						API key
					</label>
					<input
						ref={field}
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
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
						Cancel
					</button>
					<button type="submit" className="control btn-primary" disabled={busy || secret.trim() === ""}>
						{busy ? "Saving…" : "Save key"}
					</button>
				</div>
			</div>
			{provider.doc !== undefined && (
				<p className="group-hint">
					<button type="button" className="underline" onClick={() => void openLink(provider.doc ?? "")}>
						Where to get one
					</button>
				</p>
			)}
		</form>
	);
}

/**
 * One provider's page: which of its models the pickers show, and the way
 * out. Every model checked is the same as no filter, so Save removes the
 * provider's entry rather than writing a list that means "all".
 */
function ProviderPage({
	credential,
	name,
	enabledModels,
	onBack,
	onSignIn,
	onRemoved,
}: {
	credential: Credential;
	name: string;
	enabledModels: Record<string, string[]>;
	onBack(): void;
	onSignIn(): void;
	onRemoved(): void;
}) {
	const providerId = credential.providerId;
	const oauth = credential.credentialKind === "oauth";
	const [refusal, setRefusal] = useState<string | null>(null);
	const [catalog, setCatalog] = useState<CatalogModel[] | null>(null);
	const [query, setQuery] = useState("");
	const [on, setOn] = useState<Set<string>>(new Set());
	const [busy, setBusy] = useState(false);

	const applyCatalog = (list: CatalogModel[]) => {
		setCatalog(list);
		setOn(new Set(list.filter((model) => model.enabled).map((model) => model.id)));
	};

	useEffect(() => {
		if (credential.revoked) {
			setCatalog([]);
			return;
		}
		wire
			.command("models.catalog", { providerId })
			.then(applyCatalog)
			.catch((error: Error) => {
				setRefusal(error.message);
				setCatalog([]);
			});
	}, [providerId, credential.revoked]);

	const run = async (work: () => Promise<void>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await work();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const refresh = () =>
		run(async () => applyCatalog(await wire.command("credential.refresh_models", { providerId })));

	const save = () =>
		run(async () => {
			if (catalog === null) return;
			const next: Record<string, string[]> = { ...enabledModels };
			if (on.size === catalog.length && catalog.every((model) => on.has(model.id))) {
				delete next[providerId];
			} else {
				next[providerId] = catalog.filter((model) => on.has(model.id)).map((model) => model.id);
			}
			await wire.command("settings.update", { patch: { enabledModels: next } });
			onBack();
		});

	const remove = () =>
		run(async () => {
			await wire.command("credential.delete", { id: credential.id });
			onRemoved();
		});

	const needle = query.trim().toLowerCase();
	const visible =
		catalog === null
			? []
			: needle === ""
				? catalog
				: catalog.filter(
						(model) => model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle),
					);

	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{credential.revoked ? (
						<section>
							<div className="grouped">
								<div className="group-row">
									<span className="group-row-text">
										<span className="group-row-title">{oauth ? "Signed out" : "Key revoked"}</span>
										<span className="group-row-detail">
											{oauth ? "The login no longer works." : "The key no longer works."}
										</span>
									</span>
									{oauth && (
										<button type="button" className="control btn-primary" disabled={busy} onClick={onSignIn}>
											Sign in again
										</button>
									)}
								</div>
							</div>
						</section>
					) : (
						<section>
							<h3 className="group-title">Models shown</h3>
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
									{oauth && (
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
									{catalog === null ? "Reading…" : `${on.size} of ${catalog.length} shown`}
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
							<p className="group-hint">Every model checked is the same as no filter.</p>
						</section>
					)}
					<section>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{oauth ? "Sign out" : "Remove key"}</span>
									<span className="group-row-detail">
										{oauth ? "Forgets the login on this machine." : "Forgets the key on this machine."}
									</span>
								</span>
								<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void remove()}>
									{oauth ? "Sign out" : "Remove"}
								</button>
							</div>
						</div>
					</section>
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

type ServerDraft = {
	name: string;
	kind: "stdio" | "http";
	command: string;
	url: string;
	authMode: "none" | "oauth" | "keep";
};

const EMPTY_DRAFT: ServerDraft = { name: "", kind: "stdio", command: "", url: "", authMode: "none" };

/**
 * Tools: the MCP servers the room has, drawn the way Providers is. An add
 * row over the list opens the form in place; a row opens the server's own
 * page, where it is edited and, at the foot, removed. Which teammate may
 * use a server is a different question, answered on that teammate.
 */
function ToolsSection({
	servers,
	onBack,
}: {
	servers: McpServer[];
	onBack?: (() => void) | undefined;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [adding, setAdding] = useState(false);
	const [open, setOpen] = useState<string | null>(null);
	const [writing, setWriting] = useState(false);

	const persist = async (next: McpServer[]): Promise<boolean> => {
		setWriting(true);
		setRefusal(null);
		try {
			await wire.command("settings.update", { patch: { mcpServers: next } });
			return true;
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			return false;
		} finally {
			setWriting(false);
		}
	};

	const opened = servers.find((one) => one.id === open);
	if (opened !== undefined) {
		return (
			<ServerPage
				server={opened}
				writing={writing}
				onBack={() => setOpen(null)}
				onSave={(next) => void persist(servers.map((one) => (one.id === opened.id ? next : one))).then((ok) => ok && setOpen(null))}
				onRemove={() => void persist(servers.filter((one) => one.id !== opened.id)).then((ok) => ok && setOpen(null))}
				refusal={refusal}
			/>
		);
	}

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Tools</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{adding && (
						<ServerForm
							title="Add a server"
							submit="Add server"
							writing={writing}
							onSubmit={(next) => void persist([...servers, next]).then((ok) => ok && setAdding(false))}
							onCancel={() => setAdding(false)}
						/>
					)}
					<section>
						<h3 className="label">MCP gateway</h3>
						<p className="hint">New teammates have no server access. Grant selected servers or all servers on each teammate.</p>
						<div className="grouped">
							{!adding && (
								<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
									<PlusIcon />
									Add server
								</button>
							)}
							{servers.length === 0 ? (
								<p className="group-row text-sm text-ink-3">
									No servers yet. Add a server here, then grant access on a teammate.
								</p>
							) : (
								servers.map((server) => (
									<button
										key={server.id}
										type="button"
										className="group-row group-row-choice w-full text-left"
										onClick={() => setOpen(server.id)}
									>
										<span className="group-row-text">
											<span className="group-row-title">{server.name}</span>
											<span className="group-row-detail">
												{server.type === "stdio" ? "Command" : "HTTP"} · <span className="font-mono">{mcpServerDetail(server)}</span>
											</span>
										</span>
										<ChevronRightIcon className="shrink-0 text-ink-3" />
									</button>
								))
							)}
						</div>
						<p className="group-hint">Teammates with All servers also receive servers added later.</p>
					</section>
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

/** One server's page: its fields, and the way out at the foot. */
function ServerPage({
	server,
	writing,
	refusal,
	onBack,
	onSave,
	onRemove,
}: {
	server: McpServer;
	writing: boolean;
	refusal: string | null;
	onBack(): void;
	onSave(next: McpServer): void;
	onRemove(): void;
}) {
	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{server.name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<ServerForm title="Server" submit="Save" server={server} writing={writing} onSubmit={onSave} />
					{server.type === "http" && server.auth.mode === "oauth" && <McpOAuthControls server={server} />}
					<section>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">Remove server</span>
									<span className="group-row-detail">Teammates that used it lose its tools on their next start.</span>
								</span>
								<button type="button" className="control btn-quiet text-danger" disabled={writing} onClick={onRemove}>
									Remove
								</button>
							</div>
						</div>
					</section>
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

function McpOAuthControls({ server }: { server: Extract<McpServer, { type: "http" }> }) {
	const [status, setStatus] = useState<McpOAuthStatus | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const refresh = async () => {
		try {
			setStatus(await wire.command("mcp.auth_status", { serverId: server.id }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	useEffect(() => {
		void refresh();
		const timer = window.setInterval(() => void refresh(), 2_000);
		return () => window.clearInterval(timer);
	}, [server.id]);

	const signIn = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			const next = await wire.command("mcp.auth_start", { serverId: server.id });
			setStatus(next);
			if (next.authorizationUrl) await openLink(next.authorizationUrl);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const signOut = async () => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("mcp.auth_sign_out", { serverId: server.id });
			await refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const state = status?.status ?? "signed_out";
	const detail =
		state === "signed_in"
			? "Signed in on this machine. Teammate access still follows each teammate's MCP policy."
			: state === "pending"
				? "Waiting for consent in your browser…"
				: state === "failed"
					? (status?.error ?? "Sign-in failed; try again.")
					: "This server has no saved sign-in.";

	return (
		<section>
			<h3 className="group-title">OAuth sign-in</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">
							{state === "signed_in" ? "Signed in" : state === "pending" ? "Signing in…" : state === "failed" ? "Sign-in needs attention" : "Signed out"}
						</span>
						<span className="group-row-detail">{detail}</span>
					</span>
					<div className="flex shrink-0 gap-2">
						<button type="button" className="control btn-primary" disabled={busy || state === "pending"} onClick={() => void signIn()}>
							{state === "signed_in" ? "Reconnect" : "Sign in"}
						</button>
						<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void signOut()}>
							Sign out
						</button>
					</div>
				</div>
			</div>
			{refusal !== null && (
				<p role="status" className="selectable text-sm text-danger">
					{refusal}
				</p>
			)}
		</section>
	);
}

/** The server's fields: one form for adding and for the page. */
function ServerForm({
	title,
	submit,
	server,
	writing,
	onSubmit,
	onCancel,
}: {
	title: string;
	submit: string;
	server?: McpServer;
	writing: boolean;
	onSubmit(next: McpServer): void;
	onCancel?: () => void;
}) {
	const [draft, setDraft] = useState<ServerDraft>(() =>
		server === undefined
			? EMPTY_DRAFT
			: server.type === "stdio"
				? { name: server.name, kind: "stdio", command: [server.command, ...server.args].join(" "), url: "", authMode: "none" }
				: {
					name: server.name,
					kind: "http",
					command: "",
					url: server.url,
					authMode:
						server.auth.mode === "oauth"
							? "oauth"
							: server.auth.mode === "none"
								? "none"
								: "keep",
				},
	);
	const nameField = useRef<HTMLInputElement>(null);
	useEffect(() => {
		if (server === undefined) nameField.current?.focus();
	}, [server]);

	const ready =
		draft.name.trim().length > 0 &&
		(draft.kind === "stdio" ? draft.command.trim().length > 0 : draft.url.trim().length > 0);

	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (!ready || writing) return;
				const name = draft.name.trim();
				onSubmit(
					draft.kind === "stdio"
						? stdioFromDraft(name, draft.command, server)
						: httpFromDraft(name, draft.url, draft.authMode, server),
				);
			}}
		>
			<h3 className="group-title">{title}</h3>
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
				{draft.kind === "http" && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2">Auth</label>
						<div className="flex-1">
							<Picker
								field
								value={draft.authMode}
								choices={[
									{ id: "none", name: "None", detail: "Connect without an OAuth sign-in" },
									{ id: "oauth", name: "OAuth 2.1", detail: "Sign in with the server's authorization page" },
									{ id: "keep", name: "Existing auth", detail: "Keep this server's existing authentication settings" },
								]}
								placeholder="Authentication"
								label="HTTP authentication"
								onChange={(authMode) =>
									setDraft({
										...draft,
										authMode: authMode === "oauth" || authMode === "keep" ? authMode : "none",
									})
								}
							/>
						</div>
					</div>
				)}
				<div className="group-row justify-end">
					{onCancel !== undefined && (
						<button type="button" className="control btn-quiet" disabled={writing} onClick={onCancel}>
							Cancel
						</button>
					)}
					<button type="submit" className="control btn-primary" disabled={writing || !ready}>
						{writing ? "Saving…" : submit}
					</button>
				</div>
			</div>
			<p className="group-hint">OAuth sign-in stores tokens on this machine and does not grant server access to a teammate.</p>
		</form>
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
function httpFromDraft(name: string, url: string, authMode: "none" | "oauth" | "keep", previous?: McpServer): McpServer {
	const auth: McpHttpAuth =
		authMode === "oauth"
			? previous?.type === "http" && previous.auth.mode === "oauth"
				? previous.auth
				: { mode: "oauth" }
			: authMode === "keep" && previous?.type === "http"
				? previous.auth
				: { mode: "none" };
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
