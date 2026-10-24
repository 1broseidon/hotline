import { useEffect, useState } from "react";
import { openLink } from "../../native";
import type { CapabilityOptions, CatalogModel, Credential, Provider } from "../../generated/contract";
import { ChevronRightIcon, PlusIcon } from "../../icons";
import { useModelsRevision, useRawSetting } from "../../room";
import { BackKey, Band } from "../../ui/Band";
import { Chips } from "../../ui/Chips";
import { Fold } from "../../ui/Fold";
import { Refusal } from "../../ui/Refusal";
import { Scroll } from "../../ui/Scroll";
import { wire } from "../../wire";
import { accountName, nextAccountLabel } from "../../accounts";
import { ConnectProvider, ProviderRow } from "../ConnectProvider";
import { UseFor } from "../UseFor";
import { Budgets } from "../Budgets";
import { tagsFor } from "../../useFor";

function byName(a: { name: string }, b: { name: string }) {
	return a.name.localeCompare(b.name);
}

/**
 * Providers: one list of what this desk can run a model on. A row is a
 * connected provider, however it was connected — a pasted key or a login
 * is a detail of the row, not a heading — and pressing it opens that
 * provider's own page. The add row at the head of the list is the one way
 * in: it names every provider, connected or not, since a provider can hold
 * several accounts, and choosing one asks for a key or starts a sign-in in
 * place. With two or more accounts of a provider each row and the model
 * picker name the account ("Anthropic · Work"); its page renames it. The form exists only while you are
 * adding. Tools is drawn the same way, on purpose: a list the room has,
 * an add row over it, a page behind each row with the way out at its foot.
 *
 * The pane is this section's own rather than Settings', because it has
 * a page under it and the band has to say which one you are on.
 */
export function ProvidersSection({
	enabledModels,
	onBack,
}: {
	enabledModels: Record<string, string[]>;
	onBack?: (() => void) | undefined;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [held, setHeld] = useState<Credential[] | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	/* Adding: the provider chosen from the plus, and the connection it
	 * replaces when it was opened from a page's "Sign in again". */
	const [choosing, setChoosing] = useState(false);
	const [adding, setAdding] = useState<{ provider: Provider; replacing: Credential | undefined } | null>(null);
	const [open, setOpen] = useState<string | null>(null);
	/* What each job can run on, for the tags on a connection and the Use for
	 * block. Null until it arrives, and for good on a desk too old to say. */
	const [capabilities, setCapabilities] = useState<CapabilityOptions | null>(null);
	const modelsRevision = useModelsRevision();
	const images = useRawSetting("images");
	const voice = useRawSetting("voice");
	const spending = useRawSetting("spending");

	const reloadCapabilities = () =>
		wire
			.command("capabilities.options", {})
			.then(setCapabilities)
			.catch(() => setCapabilities(null));

	// A connection coming or going, or any of the settings the block writes,
	// changes what it shows.
	// biome-ignore lint/correctness/useExhaustiveDependencies: these settings and modelsRevision are what change the answer; reloadCapabilities reads only a setter.
	useEffect(() => {
		void reloadCapabilities();
	}, [modelsRevision, JSON.stringify([images, voice, spending])]);

	const reload = () =>
		wire
			.command("credential.list", {})
			.then(setHeld)
			.catch((error: Error) => {
				setHeld([]);
				setRefusal(error.message);
			});

	// biome-ignore lint/correctness/useExhaustiveDependencies: read once, on open; reload reads only setters.
	useEffect(() => {
		void reload();
		wire
			.command("providers.list", {})
			.then(setProviders)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	/* Connected: one row per credential, live ones first. A revoked login
	 * stays listed so it can be signed in again or removed. */
	const connected = (held ?? [])
		.map((credential) => ({ credential, provider: providers.find((one) => one.id === credential.providerId) }))
		.sort((a, b) => Number(a.credential.revoked) - Number(b.credential.revoked) || nameOf(a).localeCompare(nameOf(b)));
	const live = new Set(connected.filter((one) => !one.credential.revoked).map((one) => one.credential.providerId));
	// How many accounts each provider has: two or more and each is named.
	const accounts = new Map<string, number>();
	for (const one of connected) {
		if (!one.credential.revoked) accounts.set(one.credential.providerId, (accounts.get(one.credential.providerId) ?? 0) + 1);
	}
	const titleOf = (one: { credential: Credential; provider: Provider | undefined }) =>
		accountName(nameOf(one), one.credential.label, (accounts.get(one.credential.providerId) ?? 0) > 1);
	// Every provider can be added again: a second account of it is another row.
	const addable = providers.slice().sort(byName);

	const begin = (provider: Provider, replacing: Credential | undefined = undefined) => {
		setRefusal(null);
		setChoosing(false);
		setAdding({ provider, replacing });
	};

	// Keep the existing connection usable until its replacement has succeeded.
	const accept = async (made: Credential) => {
		const previous = adding?.replacing?.id ?? null;
		setAdding(null);
		try {
			if (previous !== null && previous !== made.id) await wire.command("credential.delete", { id: previous });
			setHeld((known) => [...(known ?? []).filter((one) => one.id !== previous && one.id !== made.id), made]);
			setOpen(made.id);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
			await reload();
		}
	};

	const opened = connected.find((one) => one.credential.id === open);
	if (opened !== undefined) {
		return (
			<ProviderPage
				credential={opened.credential}
				name={titleOf(opened)}
				onRenamed={(renamed) => setHeld((known) => (known ?? []).map((one) => (one.id === renamed.id ? renamed : one)))}
				discover={opened.provider?.modelDiscovery ?? false}
				enabledModels={enabledModels}
				onBack={() => setOpen(null)}
				onSignIn={() => {
					setOpen(null);
					const provider = opened.credential.custom ? providers.find((one) => one.id === "openai-compatible") : opened.provider;
					if (provider) begin(provider, opened.credential.custom && opened.credential.revoked ? undefined : opened.credential);
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
							<div className="flex items-center">
								<h3 className="group-title flex-1">Add provider</h3>
								<button type="button" className="control btn-quiet mb-2" onClick={() => setChoosing(false)}>
									Cancel
								</button>
							</div>
							{/* Settings lists every service by name; the welcome's cards are for someone choosing their first. */}
							<div className="grouped">
								{addable.map((provider) => (
									<ProviderRow key={provider.id} provider={provider} onPick={() => begin(provider)} />
								))}
							</div>
						</section>
					)}
					{adding !== null && (
						<ConnectProvider
							key={adding.provider.id}
							provider={adding.provider}
							replacing={adding.replacing}
							label={adding.replacing === undefined ? nextAccountLabel(adding.provider.name, accounts.get(adding.provider.id) ?? 0) : undefined}
							onConnected={(made) => void accept(made)}
							onCancel={() => {
								setAdding(null);
								// A sign-in cancelled late may still have landed.
								void reload();
							}}
						/>
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
								<p className="group-row text-sm text-ink-3">No providers yet. Hotline Agent needs one to run a model.</p>
							) : (
								connected.map((one) => (
									<button
										key={one.credential.id}
										type="button"
										className="group-row group-row-choice w-full text-left"
										onClick={() => setOpen(one.credential.id)}
									>
										<span className="group-row-text">
											<span className="group-row-title">{titleOf(one)}</span>
											<span className="group-row-detail">
												{one.credential.revoked
													? "Disconnected"
												: `${one.credential.custom ? (one.credential.custom.api === "responses" ? "Responses" : "Chat Completions") : one.credential.credentialKind === "oauth" ? "Signed in" : one.credential.credentialKind === "local" ? one.credential.baseUrl : "API key"} · ${modelsShownText(enabledModels[one.credential.providerId])}`}
											</span>
										</span>
										{capabilities !== null && !one.credential.revoked && (
											<span className="shrink-0 text-sm text-ink-3">{tagsFor(capabilities, one.credential.providerId).join(" · ")}</span>
										)}
										<ChevronRightIcon className="shrink-0 text-ink-3" />
									</button>
								))
							)}
						</div>
						<p className="group-hint">Keys and sign-ins stay on this computer.</p>
					</section>
					{capabilities !== null && <UseFor options={capabilities} voice={voice} connected={[...live]} onChanged={() => void reloadCapabilities()} />}
					{capabilities !== null && <Budgets spending={capabilities.spending} onChanged={() => void reloadCapabilities()} />}
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/**
 * The account's own name, which the model picker shows once its provider has
 * another account. Saved when the field is left or Enter is pressed.
 */
function AccountName({
	credential,
	onRenamed,
	onRefused,
}: {
	credential: Credential;
	onRenamed(credential: Credential): void;
	onRefused(message: string | null): void;
}) {
	const [draft, setDraft] = useState(credential.label);
	useEffect(() => setDraft(credential.label), [credential.label]);
	const save = () => {
		const label = draft.trim();
		if (label === credential.label) return;
		if (label === "") {
			setDraft(credential.label);
			return;
		}
		onRefused(null);
		void wire.command("credential.rename", { id: credential.id, label }).then(onRenamed, (error: Error) => {
			onRefused(error.message);
			setDraft(credential.label);
		});
	};
	return (
		<label className="flex items-center gap-3 text-sm">
			<span className="w-24 shrink-0 text-ink-2">Name</span>
			<input
				className="field min-w-0 flex-1"
				value={draft}
				maxLength={80}
				aria-label="Account name"
				onChange={(event) => setDraft(event.target.value)}
				onBlur={save}
				onKeyDown={(event) => {
					if (event.key === "Enter") event.currentTarget.blur();
					if (event.key === "Escape") setDraft(credential.label);
				}}
			/>
		</label>
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
	onRenamed,
}: {
	credential: Credential;
	name: string;
	onRenamed(credential: Credential): void;
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
	// A filter is a choice of some models; no filter is every model.
	const [some, setSome] = useState(enabledModels[providerId] !== undefined);
	// Whether the switches hold a choice of the person's, saved or made here.
	// Without one, Chosen starts with every switch off.
	const [picked, setPicked] = useState(enabledModels[providerId] !== undefined);
	const [adding, setAdding] = useState(false);

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

	// biome-ignore lint/correctness/useExhaustiveDependencies: the catalog is read once per provider, and applyCatalog reads that render's filter.
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
			setNotice("Model list refreshed. Your choice is unchanged.");
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
		setNotice(list.find((model) => model.id === id)?.enabled ? "Model added." : "Model added. Switch it on under Chosen to use it.");
	});

	const removeManualModel = (id: string) => run(async () => {
		if (catalog === null) return;
		await setManualModels(catalog.filter((model) => model.manual && model.id !== id).map((model) => model.id));
	});

	/* Every change is written at once: no filter for every model, else the ones switched on. */
	const write = (nextSome: boolean, nextOn: Set<string>) =>
		run(async () => {
			if (catalog === null) return;
			const next: Record<string, string[]> = { ...enabledModels };
			if (!nextSome) delete next[providerId];
			else next[providerId] = [...nextOn];
			await wire.command("settings.update", { patch: { enabledModels: next } });
		});

	const choose = (nextSome: boolean) => {
		const nextOn = nextSome && !picked ? new Set<string>() : on;
		setSome(nextSome);
		setOn(nextOn);
		if (nextSome) setPicked(true);
		void write(nextSome, nextOn);
	};

	const flipAll = (ids: string[], value: boolean) => {
		const next = new Set(on);
		for (const id of ids) {
			if (value) next.add(id);
			else next.delete(id);
		}
		setOn(next);
		void write(true, next);
	};

	const flip = (id: string, value: boolean) => flipAll([id], value);

	const remove = () =>
		run(async () => {
			await wire.command("credential.delete", { id: credential.id });
			onRemoved();
		});

	const needle = query.trim().toLowerCase();
	// The ones in the picker lead, in the order they were when the list came;
	// a switch flipped now does not move its row out from under the pointer.
	const [lead] = useState(() => new Set(enabledModels[providerId] ?? []));
	const listed = catalog === null ? [] : [...catalog].sort((a, b) => Number(lead.has(b.id)) - Number(lead.has(a.id)));
	const visible =
		needle === ""
			? listed
			: listed.filter((model) => model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle));
	const how = custom
		? `${custom.api === "responses" ? "Responses" : "Chat Completions"} · ${credential.baseUrl ?? ""}`
		: oauth
			? "Signed in"
			: local
				? (credential.baseUrl ?? "Server")
				: "API key";
	const manual = catalog?.filter((model) => model.manual) ?? [];

	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section className="nt-card">
						<div className="flex items-center gap-3">
							<span className="nt-avatar" aria-hidden="true">
								{name.charAt(0).toUpperCase()}
							</span>
							<span className="flex min-w-0 flex-1 flex-col">
								<span className="welcome-card-title">{name}</span>
								<span className="truncate text-sm text-ink-2">
									{credential.revoked ? (oauth ? "Signed out of Hotline." : local ? "Disconnected." : "The key no longer works.") : how}
								</span>
							</span>
						</div>
						<AccountName credential={credential} onRenamed={onRenamed} onRefused={setRefusal} />
						{oauth && providerId === "github-copilot" && (
							<p className="text-sm text-ink-2">
								Sign out removes this login from Hotline. To revoke GitHub access too, use{" "}
								<button type="button" className="control btn-quiet" onClick={() => void openLink("https://github.com/settings/applications").catch((error: unknown) => setRefusal(String(error)))}>Revoke at GitHub</button>.
							</p>
						)}
						<div className="flex flex-wrap items-center justify-end gap-2">
							{credential.revoked && (oauth || custom) ? (
								<button type="button" className="control btn btn-primary" disabled={busy} onClick={onSignIn}>
									{custom ? "Add connection again" : "Sign in again"}
								</button>
							) : (
								<button type="button" className="control btn" disabled={busy} onClick={onSignIn}>
									{custom ? "Edit connection" : "Change connection"}
								</button>
							)}
							<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void remove()}>
								{oauth ? "Sign out" : custom ? "Remove connection" : local ? "Disconnect" : "Remove key"}
							</button>
						</div>
					</section>
					{!credential.revoked && (
						<section>
							<h3 className="group-title">Models in the picker</h3>
							<div className="grouped">
								<div className="group-row">
									<span className="group-row-text">
										<span className="group-row-title">
											{catalog === null
												? refusal
													? "Model list unavailable"
													: "Reading…"
												: catalog.length === 0 && providerId === "ollama"
													? "No models yet. Pull one with Ollama, then refresh."
													: some
														? `${catalog.filter((model) => on.has(model.id)).length} of ${catalog.length}`
														: `All ${catalog.length}`}
										</span>
									</span>
									{discover && (
										<button type="button" className="control btn-quiet btn-sm" disabled={busy} aria-label="Refresh provider models" onClick={() => void refresh()}>
											Refresh
										</button>
									)}
									<Chips
										value={some ? "some" : "all"}
										choices={[
											{ id: "all", name: "All" },
											{ id: "some", name: "Chosen" },
										]}
										label="Which models are in the picker"
										disabled={busy || catalog === null}
										onChange={(id) => choose(id === "some")}
									/>
								</div>
								{some && catalog !== null && catalog.length > 0 && (
									<div className="group-row">
										{catalog.length > 6 ? (
											<input
												type="search"
												className="field min-w-0 flex-1"
												placeholder="Search models"
												value={query}
												onChange={(event) => setQuery(event.target.value)}
												aria-label="Search models"
											/>
										) : (
											<span className="flex-1" />
										)}
										{/* On a search these act on the models it shows. */}
										<button
											type="button"
											className="control btn-quiet btn-sm"
											disabled={busy || visible.every((model) => on.has(model.id))}
											onClick={() => flipAll(visible.map((model) => model.id), true)}
										>
											All on
										</button>
										<button
											type="button"
											className="control btn-quiet btn-sm"
											disabled={busy || visible.every((model) => !on.has(model.id))}
											onClick={() => flipAll(visible.map((model) => model.id), false)}
										>
											All off
										</button>
									</div>
								)}
								{some &&
									visible.map((model) => (
										<label key={model.id} className="group-row group-row-choice">
											<span className="group-row-text">
												<span className="group-row-title">{model.name}</span>
												<span className="group-row-detail">
													<span className="font-mono">{model.id}</span>
													{tokensText(model) !== "" && ` · ${tokensText(model)}`}
													{model.manual && " · added by you"}
												</span>
											</span>
											<input
												type="checkbox"
												role="switch"
												className="switch"
												aria-label={model.name}
												checked={on.has(model.id)}
												disabled={busy}
												onChange={(event) => flip(model.id, event.target.checked)}
											/>
										</label>
									))}
								{!custom && (
									<Fold
										title="A model by ID"
										value={manual.length === 0 ? "For one this list is missing" : manual.map((model) => model.id).join(", ")}
										action="Add"
										open={adding}
										onToggle={() => setAdding((was) => !was)}
									>
										<form
											className="flex gap-2"
											onSubmit={(event) => {
												event.preventDefault();
												void addManualModel();
											}}
										>
											<input
												className="field min-w-0 flex-1 font-mono text-sm"
												aria-label="Model ID to add"
												placeholder="Exact model ID"
												value={manualId}
												onChange={(event) => setManualId(event.target.value)}
												spellCheck={false}
												autoComplete="off"
												disabled={busy || catalog === null}
											/>
											<button type="submit" className="control btn" disabled={busy || catalog === null || !manualId.trim()}>
												Add
											</button>
										</form>
										{manual.map((model) => (
											<div className="flex items-center gap-2" key={model.id}>
												<span className="min-w-0 flex-1 truncate font-mono text-sm">{model.id}</span>
												<button type="button" className="control btn-quiet btn-sm" disabled={busy} aria-label={`Remove manual model ${model.id}`} onClick={() => void removeManualModel(model.id)}>
													Remove
												</button>
											</div>
										))}
										<p className="hint">
											{providerId === "github-copilot"
												? "It must also be in your Copilot account's model list."
												: "Use one that supports chat and tools."}
										</p>
									</Fold>
								)}
							</div>
						</section>
					)}
					{notice !== null && (
						<p className="group-hint" role="status">
							{notice}
						</p>
					)}
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/** A model's limits in a few characters: "1M context · 128K out". */
function tokensText(model: CatalogModel): string {
	const short = (n: number) => (n >= 1_000_000 ? `${+(n / 1_000_000).toFixed(1)}M` : n >= 1_000 ? `${Math.round(n / 1_000)}K` : String(n));
	return [model.contextLimit ? `${short(model.contextLimit)} context` : null, model.outputLimit ? `${short(model.outputLimit)} out` : null]
		.filter(Boolean)
		.join(" · ");
}
