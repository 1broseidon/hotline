import { Fragment, useEffect, useRef, useState } from "react";
import type { BackendChoice, CatalogModel, ComputerReleases, ComputerRuntime, ConfigChoice, Credential, CredentialKind, LoginPrompt, Provider, Report, RuntimeReport, RuntimeState } from "../generated/contract";
import { openLink, pinnedComputerImage } from "../native";
import { chordKeys } from "../chords";
import { ArrowLeftIcon, ChevronRightIcon, InfoIcon, PlusIcon } from "../icons";
import { mcpServerDetail, type McpHttpAuth, type McpServer } from "../mcp";
import type { McpOAuthStatus } from "../wire";
import { DEFAULT_IDLE_HOURS, useRoomSettings } from "../room";
import { BackKey, Band } from "../ui/Band";
import { Picker } from "../ui/Menu";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";
import { BackendPicker } from "./BackendPicker";
import { PathField } from "./PathField";
import { CustomProviderForm } from "./CustomProviderForm";
import { SkillsSection } from "./Skills";

import { UpdatesSection } from "./UpdatesSection";
import { RemoteSection } from "./RemoteSection";

const MIN_IDLE_HOURS = 1;
const MAX_IDLE_HOURS = 336;

export type SettingsSection = "general" | "providers" | "tools" | "skills" | "computer" | "remote" | "updates" | "import";

const SECTIONS: { id: SettingsSection; title: string }[] = [
	{ id: "general", title: "General" },
	{ id: "providers", title: "Providers" },
	{ id: "tools", title: "Tools" },
	{ id: "skills", title: "Skills" },
	{ id: "computer", title: "Computer" },
	{ id: "remote", title: "Remote" },
	{ id: "updates", title: "Updates" },
	{ id: "import", title: "Import" },
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
			<Band>
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
						<span className="block min-w-0 flex-1 truncate font-medium text-ink">{one.title}</span>
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

	// Providers, Tools and Skills have a page under them, so their bands are their own.
	if (section === "providers") return <ProvidersSection enabledModels={settings.enabledModels} onBack={onBack} />;
	if (section === "tools") return <ToolsSection servers={settings.mcpServers} onBack={onBack} />;
	if (section === "skills") return <SkillsSection onBack={onBack} />;

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
					{section === "updates" && <UpdatesSection />}
					{section === "remote" && <RemoteSection />}
					{section === "import" && <ImportSection onRefuse={setRefusal} />}
					{refusal !== null && <Refusal message={refusal} />}
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
				<p className="group-hint">The default is {DEFAULT_IDLE_HOURS}.</p>
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
			</section>
			<section>
				<h3 className="group-title">Default model</h3>
				<div className="grouped">
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">Toad Agent starts on</span>
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
			</section>
		</>
	);
}

const RUNTIME_NAMES: Record<ComputerRuntime, string> = {
	docker: "Docker",
	podman: "Podman",
	container: "Apple container",
};

/** Two words per state; the runtime's own words go behind the info key. */
const STATE_NAMES: Record<Exclude<RuntimeState, "ready">, string> = {
	not_installed: "Not installed",
	not_running: "Not running",
	not_responding: "Not responding",
	failed: "Failed",
	unsupported: "macOS only",
};

/** What to do about a runtime that is not ready, and where it is written up. */
const RUNTIME_HELP: Record<ComputerRuntime, { install: string; start: string; docs: string }> = {
	docker: { install: "Install Docker Desktop or OrbStack.", start: "Start Docker Desktop or OrbStack.", docs: "https://docs.docker.com/get-started/get-docker/" },
	podman: { install: "Install Podman.", start: "Run podman machine start.", docs: "https://podman.io/docs/installation" },
	container: { install: "Install Apple container.", start: "Run container system start.", docs: "https://github.com/apple/container" },
};

function runtimeAdvice(report: RuntimeReport): string | null {
	const help = RUNTIME_HELP[report.runtime];
	switch (report.state) {
		case "not_installed":
			return help.install;
		case "not_running":
			return help.start;
		case "not_responding":
		case "failed":
			return `Run ${report.runtime} version in a terminal.`;
		default:
			return null;
	}
}

/**
 * Computer: which container runtime wakes a teammate's desktop, and which
 * image it wakes. Every runtime this machine could have is a row, the
 * missing ones greyed with two words for how, and an info key that opens
 * what the runtime said and what to do about it, so "no computer" is never
 * a mystery and never a paragraph. Automatic is the room's default and
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
	const [releases, setReleases] = useState<ComputerReleases | null>(null);
	const [draft, setDraft] = useState(image ?? "");
	const [shown, setShown] = useState<ComputerRuntime | null>(null);

	useEffect(() => {
		setDraft(image ?? "");
	}, [image]);

	useEffect(() => {
		void wire
			.command("computer.runtimes", {})
			.then(setReports)
			.catch(() => setReports([]));
		void wire
			.command("computer.releases", {})
			.then(setReleases)
			.catch(() => setReleases(null));
	}, []);

	const commitImage = () => {
		const trimmed = draft.trim();
		setDraft(trimmed);
		const next = trimmed === "" ? null : trimmed;
		if (next !== image) onImage(next);
	};

	const chosen = runtime ?? "";
	const rows: { id: string; name: string; detail: string; off: boolean; report?: RuntimeReport }[] = [
		{ id: "", name: "Automatic", detail: "First runtime that answers.", off: false },
		...(reports ?? []).map((one) => ({
			id: one.runtime,
			name: RUNTIME_NAMES[one.runtime],
			detail: one.state === "ready" ? (one.rootless ? "Ready, rootless" : "Ready") : STATE_NAMES[one.state],
			off: one.state !== "ready",
			report: one,
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
						{rows.map((row) => {
							const advice = row.report === undefined ? null : runtimeAdvice(row.report);
							const more = row.report !== undefined && (row.report.detail !== undefined || advice !== null);
							const open = more && shown === row.report?.runtime;
							return (
								<Fragment key={row.id}>
									<label className="group-row group-row-choice" data-off={row.off ? "true" : undefined}>
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
											<span className="group-row-detail">{row.detail}</span>
										</span>
										{more && (
											<button
												type="button"
												className="control btn-icon"
												title="Why"
												aria-label={`Why ${row.name} is ${row.detail.toLowerCase()}`}
												aria-expanded={open}
												onClick={(event) => {
													event.preventDefault();
													setShown(open ? null : (row.report?.runtime ?? null));
												}}
											>
												<InfoIcon />
											</button>
										)}
									</label>
									{open && row.report !== undefined && (
										<div className="group-row flex-col items-stretch gap-1.5 pl-10">
											{advice !== null && <span className="text-sm text-ink-2">{advice}</span>}
											{row.report.detail !== undefined && (
												<pre className="refusal-detail selectable">{row.report.detail}</pre>
											)}
											<button
												type="button"
												className="self-start text-sm underline"
												onClick={() => void openLink(RUNTIME_HELP[row.report?.runtime ?? "docker"].docs)}
											>
												{row.name} docs
											</button>
										</div>
									)}
								</Fragment>
							);
						})}
					</div>
				)}
				<p className="group-hint">A teammate&rsquo;s computer is a Linux desktop in a container.</p>
			</section>
			<section>
				<h3 className="group-title">Image</h3>
				<div className="grouped">
					<div className="group-row">
						<label className="group-row-text" htmlFor="setting-computer-image">
							<span className="group-row-title">Desktop image</span>
						</label>
						<input
							id="setting-computer-image"
							className="field w-72 min-w-0 font-mono text-sm"
							placeholder={releases?.newest !== undefined ? `Newest release, ${releases.newest}` : pinnedComputerImage() || "Newest release"}
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
				<p className="group-hint">
					Blank creates new computers on the newest toad-computer release, checked every six hours
					{releases !== null ? ` (never below ${releases.floor})` : ""}. Set an image to pin one; a pinned computer is never offered an update.
				</p>
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
	const [method, setMethod] = useState<CredentialKind | null>(null);
	const replacing = useRef<string | null>(null);
	const loginAttempt = useRef(0);
	useEffect(() => () => {
		loginAttempt.current += 1;
	}, []);
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
						setLogin(null);
						if (status.credential) {
							void accept(status.credential);
						} else {
							setAdding(null);
							setBusy(false);
						}
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
			void wire.command("credential.login_cancel", { loginId: login.prompt.loginId }).catch(() => {});
		};
	}, [login]);

	/* Connected: one row per credential, live ones first. A revoked login
	 * stays listed so it can be signed in again or removed. */
	const connected = (held ?? [])
		.map((credential) => ({ credential, provider: providers.find((one) => one.id === credential.providerId) }))
		.sort((a, b) => Number(a.credential.revoked) - Number(b.credential.revoked) || nameOf(a).localeCompare(nameOf(b)));
	const live = new Set(connected.filter((one) => !one.credential.revoked).map((one) => one.credential.providerId));
	const addable = providers.filter((one) => !live.has(one.id)).slice().sort(byName);

	const begin = (provider: Provider, replaceId: string | null = null) => {
		setRefusal(null);
		setChoosing(false);
		setAdding(provider);
		replacing.current = replaceId;
		const initial = provider.credentialKinds.length === 1 ? provider.credentialKinds[0] ?? null : null;
		setMethod(initial);
		if (initial === "oauth") void signIn(provider.id);
	};

	const signIn = async (id: string) => {
		if (busy) return;
		setBusy(true);
		setMethod("oauth");
		const attempt = ++loginAttempt.current;
		try {
			const prompt = await wire.command("credential.login", { providerId: id });
			if (attempt !== loginAttempt.current) {
				await wire.command("credential.login_cancel", { loginId: prompt.loginId });
				return;
			}
			setLogin({ providerId: id, prompt });
		} catch (error) {
			if (attempt !== loginAttempt.current) return;
			setRefusal(error instanceof Error ? error.message : String(error));
			setAdding(null);
			setBusy(false);
		}
	};

	const cancelLogin = async () => {
		loginAttempt.current += 1;
		try {
			if (login) await wire.command("credential.login_cancel", { loginId: login.prompt.loginId });
			setHeld(await wire.command("credential.list", {}));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setLogin(null);
			setAdding(null);
			setBusy(false);
		}
	};

	// Keep the existing connection usable until its replacement has succeeded.
	const accept = async (made: Credential) => {
		const previous = replacing.current;
		replacing.current = null;
		try {
			if (previous !== null && previous !== made.id) await wire.command("credential.delete", { id: previous });
			setHeld((known) => [...(known ?? []).filter((one) => one.id !== previous && one.id !== made.id), made]);
			setAdding(null);
			const provider = providers.find((one) => one.id === made.providerId);
			if (provider?.modelDiscovery && made.providerId !== "ollama" && made.providerId !== "github-copilot") {
				await wire.command("credential.refresh_models", { providerId: made.providerId });
			}
			setOpen(made.id);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setAdding(null);
			await wire.command("credential.list", {}).then(setHeld).catch(() => {});
		} finally {
			setBusy(false);
		}
	};

	const saveKey = async (secret: string) => {
		if (adding === null || busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await accept(await wire.command("credential.create", { providerId: adding.id, label: adding.name, secret }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const connectLocal = async (baseUrl: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await accept(await wire.command("credential.connect_local", { baseUrl }));
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	const opened = connected.find((one) => one.credential.id === open);
	if (opened !== undefined) {
		return (
			<ProviderPage
				credential={opened.credential}
				name={nameOf(opened)}
				discover={opened.provider?.modelDiscovery ?? false}
				enabledModels={enabledModels}
				onBack={() => setOpen(null)}
				onSignIn={() => {
					setOpen(null);
					const provider = opened.credential.custom ? providers.find((one) => one.id === "openai-compatible") : opened.provider;
					if (provider) begin(provider, opened.credential.custom && opened.credential.revoked ? null : opened.credential.id);
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
											<span className="text-sm text-ink-3">{provider.credentialKinds.map(connectionMethod).join(" or ")}</span>
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
						</section>
					)}
					{adding?.id === "openai-compatible" && (
						<CustomProviderForm key={replacing.current ?? "new"} credential={held?.find((one) => one.id === replacing.current)} onSaved={(made) => void accept(made)} onCancel={() => setAdding(null)} />
					)}
					{adding !== null && adding.id !== "openai-compatible" && method === null && (
						<section>
							<h3 className="group-title">Connect {adding.name}</h3>
							<div className="grouped">
								{adding.credentialKinds.map((kind) => (
									<button
										key={kind}
										type="button"
										className="group-row group-row-choice w-full text-left"
										disabled={busy}
										onClick={() => kind === "oauth" ? void signIn(adding.id) : setMethod(kind)}
									>
										<span className="group-row-text">{adding.id === "xai" && kind === "oauth" ? "Sign in with SuperGrok or X Premium+" : connectionMethod(kind)}</span>
										<ChevronRightIcon />
									</button>
								))}
								<div className="group-row justify-end">
									<button type="button" className="control btn-quiet" onClick={() => setAdding(null)}>Cancel</button>
								</div>
							</div>
						</section>
					)}
					{adding !== null && method === "local" && (
						<LocalProviderForm busy={busy} onSave={(url) => void connectLocal(url)} onCancel={() => setAdding(null)} />
					)}
					{adding !== null && method === "api_key" && (
						<KeyForm provider={adding} busy={busy} onSave={(secret) => void saveKey(secret)} onCancel={() => setAdding(null)} />
					)}
					{adding !== null && method === "oauth" && (
						<section>
							<h3 className="group-title">Sign in to {adding.name}</h3>
							<div className="grouped">
								{login === null ? (
									<p className="group-row text-sm text-ink-3">Preparing sign-in…</p>
								) : (
									<>
										{login.prompt.userCode !== "" && (
											<div className="group-row">
												<span className="selectable font-mono text-xl tracking-wide">{login.prompt.userCode}</span>
											</div>
										)}
										<div className="group-row">
											<button
												type="button"
												className="text-sm text-ink-2 underline"
												onClick={() => void openLink(login.prompt.verificationUri)}
											>
												Open {adding.name} sign-in
											</button>
										</div>
										<p className="group-row text-sm text-ink-3">Waiting for you to sign in…</p>
									</>
								)}
								<div className="group-row justify-end">
									<button
										type="button"
										className="control btn-quiet"
										onClick={() => void cancelLogin()}
									>
										Cancel
									</button>
								</div>
							</div>
							<p className="group-hint">{login?.prompt.userCode ? "Enter the code on that page." : "Finish signing in with your browser, then return here."}</p>
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
													? "Disconnected"
												: `${one.credential.custom ? (one.credential.custom.api === "responses" ? "Responses" : "Chat Completions") : one.credential.credentialKind === "oauth" ? "Signed in" : one.credential.credentialKind === "local" ? one.credential.baseUrl : "API key"} · ${modelsShownText(enabledModels[one.credential.providerId])}`}
											</span>
										</span>
										<ChevronRightIcon className="shrink-0 text-ink-3" />
									</button>
								))
							)}
						</div>
						<p className="group-hint">API keys, OpenRouter and Grok sign-ins use your OS credential store. ChatGPT and Copilot keep tokens in permission-restricted files.</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

function connectionMethod(kind: CredentialKind): string {
	return kind === "oauth" ? "Sign in" : kind === "local" ? "Server URL" : "API key";
}

function LocalProviderForm({ busy, onSave, onCancel }: { busy: boolean; onSave(url: string): void; onCancel(): void }) {
	const [url, setUrl] = useState("http://localhost:11434");
	return (
		<form onSubmit={(event) => {
			event.preventDefault();
			if (url.trim()) onSave(url.trim());
		}}>
			<h3 className="group-title">Connect Ollama Local</h3>
			<div className="grouped">
				<div className="group-row">
					<label htmlFor="ollama-url" className="w-24 shrink-0 text-sm text-ink-2">Server URL</label>
					<input
						id="ollama-url"
						type="url"
						className="field flex-1 font-mono text-sm"
						value={url}
						onChange={(event) => setUrl(event.target.value)}
						disabled={busy}
						spellCheck={false}
						autoComplete="off"
					/>
				</div>
				<div className="group-row justify-end">
					<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>Cancel</button>
					<button type="submit" className="control btn-primary" disabled={busy || !url.trim()}>{busy ? "Connecting…" : "Connect"}</button>
				</div>
			</div>
			<p className="group-hint">Start Ollama first. Toad discovers the models installed on this server. Cloud models available through your Ollama sign-in work here too.</p>
		</form>
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
	discover,
	enabledModels,
	onBack,
	onSignIn,
	onRemoved,
}: {
	credential: Credential;
	name: string;
	discover: boolean;
	enabledModels: Record<string, string[]>;
	onBack(): void;
	onSignIn(): void;
	onRemoved(): void;
}) {
	const providerId = credential.providerId;
	const oauth = credential.credentialKind === "oauth";
	const local = credential.credentialKind === "local";
	const custom = credential.custom;
	const [refusal, setRefusal] = useState<string | null>(null);
	const [catalog, setCatalog] = useState<CatalogModel[] | null>(null);
	const [manualId, setManualId] = useState("");
	const [notice, setNotice] = useState<string | null>(null);
	const [query, setQuery] = useState("");
	const [on, setOn] = useState<Set<string>>(new Set());
	const [busy, setBusy] = useState(false);

	const applyCatalog = (list: CatalogModel[], preserveSelections = false) => {
		setCatalog(list);
		setOn((known) => {
			if (!preserveSelections) return new Set([...(enabledModels[providerId] ?? []), ...list.filter((model) => model.enabled).map((model) => model.id)]);
			const next = new Set(known);
			const previousIds = new Set(catalog?.map((model) => model.id));
			for (const model of list) {
				if (!previousIds.has(model.id) && model.enabled) next.add(model.id);
			}
			return next;
		});
	};

	useEffect(() => {
		if (credential.revoked) {
			setCatalog([]);
			return;
		}
		wire
			.command("models.catalog", { providerId })
			.then((list) => applyCatalog(list))
			.catch((error: Error) => {
				setRefusal(error.message);
				setCatalog(null);
			});
	}, [providerId, credential.revoked]);

	const run = async (work: () => Promise<void>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		setNotice(null);
		try {
			await work();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const refresh = () =>
		run(async () => {
			applyCatalog(await wire.command("credential.refresh_models", { providerId }), true);
			setNotice("Model list refreshed. Your model selection is unchanged.");
		});

	const setManualModels = async (modelIds: string[]) => {
		const list = await wire.command("models.manual_set", { providerId, modelIds });
		applyCatalog(list, true);
		return list;
	};

	const addManualModel = () => run(async () => {
		const id = manualId.trim();
		if (!id || catalog === null) return;
		const list = await setManualModels([...new Set([...catalog.filter((model) => model.manual).map((model) => model.id), id])]);
		setManualId("");
		setNotice(list.find((model) => model.id === id)?.enabled
			? "Model ID added to this connection."
			: "Model ID added. Check it under Models shown and save to include it in the picker.");
	});

	const removeManualModel = (id: string) => run(async () => {
		if (catalog === null) return;
		await setManualModels(catalog.filter((model) => model.manual && model.id !== id).map((model) => model.id));
		setNotice("Manual entry removed. A model listed by the provider or catalogue can still appear.");
	});

	const save = () =>
		run(async () => {
			if (catalog === null) return;
			const next: Record<string, string[]> = { ...enabledModels };
			if (on.size === catalog.length && catalog.every((model) => on.has(model.id))) {
				delete next[providerId];
			} else {
				next[providerId] = [...on];
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
					{custom && <section>
						<h3 className="group-title">Connection</h3>
						<div className="grouped"><div className="group-row"><span className="group-row-text">
							<span className="group-row-title">{custom.api === "responses" ? "Responses" : "Chat Completions"}</span>
							<span className="group-row-detail break-all">{credential.baseUrl}</span>
						</span></div></div>
					</section>}
					{credential.revoked ? (
						<section>
							<div className="grouped">
								<div className="group-row">
									<span className="group-row-text">
										<span className="group-row-title">{oauth ? "Signed out" : local ? "Disconnected" : "Key revoked"}</span>
										<span className="group-row-detail">
											{oauth ? "The login no longer works." : local ? "Connect the server again to use its models." : "The key no longer works."}
										</span>
									</span>
									{(oauth || custom) && (
										<button type="button" className="control btn-primary" disabled={busy} onClick={onSignIn}>
											{custom ? "Add connection again" : "Sign in again"}
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
									{discover && (
										<button
											type="button"
											className="control btn-quiet"
											disabled={busy}
											aria-label="Refresh provider models"
											onClick={() => void refresh()}
										>
											Refresh
										</button>
									)}
								</div>
								<p className="group-row text-sm text-ink-3">
									{catalog === null ? (refusal ? "Model list unavailable." : "Reading…") : catalog.length === 0 && providerId === "ollama" ? "No models found. Pull a model with Ollama, then refresh." : `${catalog.filter((model) => on.has(model.id)).length} of ${catalog.length} shown`}
								</p>
								{visible.map((model) => (
									<label key={model.id} className="group-row group-row-choice">
										<span className="group-row-text">
											<span className="group-row-title">{model.name}</span>
											<span className="group-row-detail font-mono">{model.id}</span>
											{((model.contextLimit ?? 0) > 0 || (model.outputLimit ?? 0) > 0) && <span className="group-row-detail">{[model.contextLimit ? `${model.contextLimit.toLocaleString()} context tokens` : null, model.outputLimit ? `${model.outputLimit.toLocaleString()} output tokens` : null].filter(Boolean).join(" · ")}</span>}
											{(model.manual || !model.metadataKnown) && <span className="group-row-detail">{[model.manual ? "Manually added" : null, !model.metadataKnown ? "Catalogue metadata unavailable" : null].filter(Boolean).join(" · ")}</span>}
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
										aria-label="Save model visibility"
										onClick={() => void save()}
									>
										{busy ? "Saving…" : "Save"}
									</button>
								</div>
							</div>
							<p className="group-hint">Every model checked is the same as no filter.</p>
						</section>
					)}
					{!credential.revoked && !custom && <section>
						<h3 className="group-title">Manual model IDs</h3>
						<div className="grouped">
							<form className="group-row" onSubmit={(event) => { event.preventDefault(); void addManualModel(); }}>
								<input className="field min-w-0 flex-1 font-mono text-sm" aria-label="Model ID to add" placeholder="Exact model ID" value={manualId} onChange={(event) => setManualId(event.target.value)} spellCheck={false} autoComplete="off" disabled={busy || catalog === null} />
								<button type="submit" className="control btn-quiet" disabled={busy || catalog === null || !manualId.trim()}>Add model</button>
							</form>
							{catalog?.filter((model) => model.manual).map((model) => <div className="group-row" key={model.id}>
								<span className="group-row-text"><span className="group-row-title font-mono">{model.id}</span></span>
								<button type="button" className="control btn-quiet" disabled={busy} aria-label={`Remove manual model ${model.id}`} onClick={() => void removeManualModel(model.id)}>Remove</button>
							</div>)}
						</div>
						<p className="group-hint">Add an ID from this provider when it is missing above. Use a model that supports chat and tools. Entries save immediately and survive refreshes and app updates.</p>
						{providerId === "github-copilot" && <p className="group-hint">Copilot IDs must also appear in your account’s refreshed model list.</p>}
					</section>}
					{notice !== null && <p className="group-hint" role="status">{notice}</p>}
					<section>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{custom ? "Remove connection" : oauth ? "Sign out" : local ? "Disconnect server" : "Remove key"}</span>
									<span className="group-row-detail">
										{oauth ? "Forgets the login on this machine." : local ? credential.baseUrl : "Forgets the key on this machine."}
									</span>
								</span>
								<button type="button" className="control btn-quiet" disabled={busy} onClick={onSignIn}>{custom ? "Edit connection" : "Change connection"}</button>
								<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void remove()}>
									{oauth ? "Sign out" : "Remove"}
								</button>
							</div>
						</div>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

type AuthMode = "none" | "oauth" | "bearer" | "header";

type ServerDraft = {
	name: string;
	kind: "stdio" | "http";
	command: string;
	url: string;
	authMode: AuthMode;
	/** The header a custom-header server sends its token in. */
	headerName: string;
	/** A token to save; empty on an edit keeps the one already saved. Never part of the server. */
	secret: string;
};

const EMPTY_DRAFT: ServerDraft = { name: "", kind: "stdio", command: "", url: "", authMode: "none", headerName: "", secret: "" };

function authModeOf(auth: McpHttpAuth): AuthMode {
	return auth.mode === "oauth" || auth.mode === "bearer" || auth.mode === "header" ? auth.mode : "none";
}

function headerNameOf(auth: McpHttpAuth): string {
	const name = (auth as { name?: unknown }).name;
	return typeof name === "string" ? name : "";
}

/** A server that sends a pasted token, kept in the vault rather than in the server. */
function holdsToken(auth: McpHttpAuth): boolean {
	return auth.mode === "bearer" || auth.mode === "header";
}

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
						<h3 className="group-title">MCP servers</h3>
						<div className="grouped">
							{!adding && (
								<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
									<PlusIcon />
									Add server
								</button>
							)}
							{servers.length === 0 ? (
								<p className="group-row text-sm text-ink-3">No servers yet.</p>
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
						<p className="group-hint">Grant servers per teammate, in its pane.</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
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
					{server.type === "http" && holdsToken(server.auth) && <McpTokenControls server={server} />}
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
					{refusal !== null && <Refusal message={refusal} />}
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
			? "Signed in."
			: state === "pending"
				? "Waiting for consent in your browser…"
				: state === "failed"
					? "Sign-in failed."
					: "No saved sign-in.";

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
			{state === "failed" && status?.error !== undefined && <Refusal message="Sign-in failed." detail={status.error} />}
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}

/** Whether the vault holds a token for this server, and the way to forget it. */
function McpTokenControls({ server }: { server: Extract<McpServer, { type: "http" }> }) {
	const [saved, setSaved] = useState<boolean | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	const refresh = async () => {
		try {
			const status = await wire.command("mcp.auth_status", { serverId: server.id });
			setSaved(status.status === "signed_in");
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	useEffect(() => {
		void refresh();
		const timer = window.setInterval(() => void refresh(), 2_000);
		return () => window.clearInterval(timer);
	}, [server.id]);

	const forget = async () => {
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

	return (
		<section>
			<h3 className="group-title">Token</h3>
			<div className="grouped">
				<div className="group-row">
					<span className="group-row-text">
						<span className="group-row-title">{saved === true ? "Token saved" : saved === false ? "No token saved" : "…"}</span>
						<span className="group-row-detail">
							{saved === true ? "Sent on every request." : "Not connected until a token is saved."}
						</span>
					</span>
					<button type="button" className="control btn-quiet text-danger" disabled={busy || saved !== true} onClick={() => void forget()}>
						Forget token
					</button>
				</div>
			</div>
			{refusal !== null && <Refusal message={refusal} />}
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
				? { name: server.name, kind: "stdio", command: [server.command, ...server.args].join(" "), url: "", authMode: "none", headerName: "", secret: "" }
				: {
					name: server.name,
					kind: "http",
					command: "",
					url: server.url,
					authMode: authModeOf(server.auth),
					headerName: headerNameOf(server.auth),
					secret: "",
				},
	);
	const [refusal, setRefusal] = useState<string | null>(null);
	const savedLaunch = server?.type === "stdio" && (!!server.credentialRef || !!server.launchValuesPending);
	const [replaceLaunch, setReplaceLaunch] = useState(false);
	const [environment, setEnvironment] = useState(() => server?.type === "stdio" && server.env ? JSON.stringify(server.env, null, 2) : "{}");
	const keepLaunch = savedLaunch && !replaceLaunch;
	const nameField = useRef<HTMLInputElement>(null);
	useEffect(() => {
		if (server === undefined) nameField.current?.focus();
	}, [server]);

	const takesToken = draft.kind === "http" && (draft.authMode === "bearer" || draft.authMode === "header");
	/* A new token server needs its token now; an edit may keep the saved one. */
	const ready =
		draft.name.trim().length > 0 &&
		(draft.kind === "stdio" ? draft.command.trim().length > 0 : draft.url.trim().length > 0) &&
		(!takesToken || server !== undefined || draft.secret.length > 0) &&
		(draft.kind !== "http" || draft.authMode !== "header" || draft.headerName.trim().length > 0);

	const submitDraft = async () => {
		const name = draft.name.trim();
		let next = draft.kind === "stdio" ? stdioFromDraft(name, draft.command, server) : httpFromDraft(name, draft.url, draft, server);
		if (next.type === "stdio" && !keepLaunch) {
			try {
				const env: unknown = JSON.parse(environment);
				if (!env || typeof env !== "object" || Array.isArray(env) || !Object.values(env).every((value) => typeof value === "string")) throw new Error("Use a JSON object with string values for environment variables.");
				const { credentialRef: _savedReference, launchValuesPending: _pendingValues, ...launch } = next;
				next = { ...launch, env: env as Record<string, string> };
			} catch (error) {
				setRefusal(error instanceof Error ? error.message : String(error));
				return;
			}
		}
		/* The token goes to the vault first, so the settings write that
		 * follows reattaches teammates with it in hand. */
		if (next.type === "http" && holdsToken(next.auth) && draft.secret.length > 0) {
			setRefusal(null);
			try {
				await wire.command("mcp.secret_set", { serverId: next.id, url: next.url, secret: draft.secret });
			} catch (error) {
				setRefusal(error instanceof Error ? error.message : String(error));
				return;
			}
			setDraft({ ...draft, secret: "" });
		}
		onSubmit(next);
	};

	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (!ready || writing) return;
				void submitDraft();
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
							readOnly={keepLaunch}
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
				{draft.kind === "stdio" && savedLaunch && (
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">{server?.type === "stdio" && server.launchValuesPending ? "Launch values need credential migration" : "Launch values stored securely"}</span>
							<span className="group-row-detail">{keepLaunch ? server?.type === "stdio" && server.launchValuesPending ? "Unlock the OS credential store and retry this source, or replace its launch values." : "Saved arguments and environment variables are kept when you rename this source." : "Enter the complete command and environment to replace the saved launch values."}</span>
						</span>
						<button type="button" className="control btn-quiet" onClick={() => {
							setReplaceLaunch(!replaceLaunch);
							if (replaceLaunch && server?.type === "stdio") setDraft({ ...draft, command: server.command });
						}}>{keepLaunch ? "Replace launch values" : "Keep saved values"}</button>
					</div>
				)}
				{draft.kind === "stdio" && !keepLaunch && (
					<div className="group-row items-start">
						<label htmlFor="tool-environment" className="w-24 shrink-0 text-sm text-ink-2">Environment</label>
						<textarea id="tool-environment" className="field flex-1 font-mono text-sm" rows={3} spellCheck={false} value={environment} onChange={(event) => setEnvironment(event.target.value)} />
					</div>
				)}
				{draft.kind === "http" && server?.type === "http" && server.urlNeedsRepair && (
					<div className="group-row text-sm text-ink-2">Re-enter the endpoint without credentials, a query, or a fragment. Put tokens in the authentication fields below.</div>
				)}
				{draft.kind === "http" && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2">Auth</label>
						<div className="flex-1">
							<Picker
								field
								value={draft.authMode}
								choices={[
									{ id: "none", name: "None", detail: "Connect without credentials" },
									{ id: "oauth", name: "OAuth 2.1", detail: "Sign in with the server's authorization page" },
									{ id: "bearer", name: "Bearer token", detail: "Send a token you paste as Authorization: Bearer" },
									{ id: "header", name: "Custom header", detail: "Send a token you paste in a header you name" },
								]}
								placeholder="Authentication"
								label="HTTP authentication"
								onChange={(authMode) =>
									setDraft({
										...draft,
										authMode: authMode === "oauth" || authMode === "bearer" || authMode === "header" ? authMode : "none",
									})
								}
							/>
						</div>
					</div>
				)}
				{draft.kind === "http" && draft.authMode === "header" && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-header">
							Header
						</label>
						<input
							id="tool-header"
							className="field flex-1 font-mono text-sm"
							spellCheck={false}
							placeholder="X-API-Key"
							value={draft.headerName}
							onChange={(event) => setDraft({ ...draft, headerName: event.target.value })}
						/>
					</div>
				)}
				{takesToken && (
					<div className="group-row">
						<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="tool-token">
							Token
						</label>
						<input
							id="tool-token"
							type="password"
							className="field flex-1 font-mono text-sm"
							autoComplete="off"
							placeholder={server === undefined ? "Paste the token" : "Saved · paste to replace"}
							value={draft.secret}
							onChange={(event) => setDraft({ ...draft, secret: event.target.value })}
						/>
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
			{refusal !== null && <Refusal message={refusal} />}
			<p className="group-hint">Kept on this machine.</p>
		</form>
	);
}

/** Renaming a source keeps its saved launch reference until explicitly replaced. */
function stdioFromDraft(name: string, commandLine: string, previous?: McpServer): McpServer {
	const [command, ...args] = commandLine.trim().split(/\s+/);
	const id = previous?.id ?? crypto.randomUUID();
	if (previous?.type === "stdio" && (previous.credentialRef || previous.launchValuesPending) && commandLine.trim() === previous.command) {
		return { ...previous, name };
	}
	const env = previous?.type === "stdio" ? previous.env : undefined;
	return env
		? { id, type: "stdio", name, command: command ?? "", args, env }
		: { id, type: "stdio", name, command: command ?? "", args };
}

/**
 * The server as settings hold it: the auth names a mode, never a token. An
 * edit that stays on OAuth keeps the scopes and client the server already
 * had; a pasted token lives in the vault and is not here to keep.
 */
function httpFromDraft(name: string, url: string, draft: ServerDraft, previous?: McpServer): McpServer {
	const auth: McpHttpAuth =
		draft.authMode === "oauth"
			? previous?.type === "http" && previous.auth.mode === "oauth"
				? previous.auth
				: { mode: "oauth" }
			: draft.authMode === "bearer"
				? { mode: "bearer" }
				: draft.authMode === "header"
					? { mode: "header", name: draft.headerName.trim() }
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
					Copies teammates, conversations, schedules, settings and keys. The previous Toad is left as it is.
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
