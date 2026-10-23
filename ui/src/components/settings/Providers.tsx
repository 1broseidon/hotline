import { useEffect, useState } from "react";
import type { AccountAgent, AccountsList, AccountView, CapabilityOptions, CredentialKind, Provider, RosterEntry } from "../../generated/contract";
import { ChevronRightIcon, PlusIcon } from "../../icons";
import { useModelsRevision, useRawSetting } from "../../room";
import { BackKey, Band } from "../../ui/Band";
import { Refusal } from "../../ui/Refusal";
import { Scroll } from "../../ui/Scroll";
import { tagsFor } from "../../useFor";
import { wire } from "../../wire";
import { AddAccountForm } from "../accounts/AddAccountForm";
import { NEVER_SEES_SIGN_IN } from "../accounts/words";
import { Budgets } from "../Budgets";
import { ConnectProvider, connectionMethod, ProviderRow } from "../ConnectProvider";
import { UseFor } from "../UseFor";
import { CliAccountPage, useCliAccounts } from "./CliAccountPage";
import { ConnectionPage, countsText, type Holding, holdingOf, liveOf, ProviderPage } from "./ProviderPage";

const NOTHING: AccountsList = { agents: [], accounts: [], credentials: [] };

function byName(a: { name: string }, b: { name: string }) {
	return a.name.localeCompare(b.name);
}

type Way = { agent: AccountAgent } | { kinds: CredentialKind[] };

/**
 * Providers: one list of what this desk can run a model on. A row is a
 * provider that holds an account, however many and of whatever kind — a
 * Claude Code sign-in, a pasted key, a login, a server — and pressing it
 * opens that provider's own page, where each account is. The add row at the
 * head of the list is the one way in: it names the providers not yet here,
 * and choosing one asks for a key or starts a sign-in in place. The form
 * exists only while you are adding. Tools is drawn the same way, on purpose:
 * a list the room has, an add row over it, a page behind each row.
 *
 * The pane is this section's own rather than Settings', because it has
 * pages under it and the band has to say which one you are on. `opening`
 * lands on a CLI account's page, with its provider's page behind Back.
 */
export function ProvidersSection({
	enabledModels,
	roster,
	opening,
	onBack,
}: {
	enabledModels: Record<string, string[]>;
	roster: RosterEntry[];
	opening?: string | undefined;
	onBack?: (() => void) | undefined;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [list, setList] = useState<AccountsList | null>(null);
	const [providers, setProviders] = useState<Provider[]>([]);
	const [choosing, setChoosing] = useState(false);
	const [adding, setAdding] = useState<{ provider: Provider; way: Way | null } | null>(null);
	const [open, setOpen] = useState<string | null>(null);
	const [accountId, setAccountId] = useState<string | null>(opening ?? null);
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
			.command("accounts.list", {})
			.then(setList)
			.catch((error: Error) => {
				setList((was) => was ?? NOTHING);
				setRefusal(error.message);
			});

	const cli = useCliAccounts({ roster, setList, refuse: setRefusal, reload });

	// biome-ignore lint/correctness/useExhaustiveDependencies: read once, on open; reload reads only setters.
	useEffect(() => {
		void reload();
		wire
			.command("providers.list", {})
			.then(setProviders)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	const go = (provider: string | null, account: string | null) => {
		cli.leave();
		setRefusal(null);
		setOpen(provider);
		setAccountId(account);
	};

	const held = list === null ? [] : rowsOf(list, providers);
	const live = new Set(held.filter((one) => liveOf(one).length > 0).map((one) => one.providerId));
	const listed = new Set(held.map((one) => one.providerId));
	const addable = providers.filter((one) => !listed.has(one.id)).sort(byName);

	const opened = list?.accounts.find((one) => one.account.id === accountId);
	const openedAgent = opened && list?.agents.find((one) => one.backendId === opened.account.backendId && one.supportsAccounts);
	if (opened !== undefined && openedAgent !== undefined) {
		const toProvider = () => go(openedAgent.providerId ?? null, null);
		return (
			<CliAccountPage
				key={opened.account.id}
				agent={openedAgent}
				view={opened}
				cli={cli}
				refusal={refusal}
				refuse={setRefusal}
				onBack={toProvider}
				onRenamed={(renamed) => {
					setList((was) => was && { ...was, accounts: was.accounts.map((one) => (one.account.id === renamed.account.id ? renamed : one)) });
					void reload();
				}}
				onRemoved={() => {
					toProvider();
					void reload();
				}}
			/>
		);
	}

	if (list !== null && open !== null) {
		const holding = holdingOf(list, providers, open);
		const page = { enabledModels, refusal, refuse: setRefusal, reload, onBack: () => go(null, null) };
		const connection = holding.credentials[0]?.credential;
		if (connection?.custom) return <ConnectionPage credential={connection} {...page} />;
		return <ProviderPage key={open} holding={holding} {...page} cli={cli} onOpenAccount={(id) => go(open, id)} />;
	}

	const begin = (provider: Provider) => {
		setRefusal(null);
		setChoosing(false);
		const agent = list?.agents.find((one) => one.supportsAccounts && one.providerId === provider.id);
		setAdding({ provider, way: agent === undefined ? { kinds: provider.credentialKinds } : null });
	};

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
								{addable.length === 0 ? (
									<p className="group-row text-sm text-ink-3">Every provider Hotline knows is already here.</p>
								) : (
									addable.map((provider) => <ProviderRow key={provider.id} provider={provider} onPick={() => begin(provider)} />)
								)}
							</div>
						</section>
					)}
					{adding !== null && (
						<Adding
							provider={adding.provider}
							way={adding.way}
							agents={list?.agents ?? []}
							onWay={(way) => setAdding({ ...adding, way })}
							onAdded={(made, email, providerId) => {
								setAdding(null);
								go(providerId, null);
								cli.added(made, email);
							}}
							onConnected={async (providerId) => {
								// Its page opens once the list has it, never on a page that lacks it.
								await reload();
								setAdding(null);
								go(providerId, null);
							}}
							onRefused={() => void reload()}
							onCancel={() => {
								setAdding(null);
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
							{list === null ? (
								<p className="group-row text-sm text-ink-3">Reading…</p>
							) : held.length === 0 ? (
								<p className="group-row text-sm text-ink-3">No providers yet. Hotline Agent needs one to run a model.</p>
							) : (
								held.map((holding) => (
									<button
										key={holding.providerId}
										type="button"
										className="group-row group-row-choice w-full text-left"
										onClick={() => go(holding.providerId, null)}
									>
										<span className="group-row-text">
											<span className="group-row-title">{holding.name}</span>
											<span className="group-row-detail">{rowDetail(holding, enabledModels)}</span>
										</span>
										{capabilities !== null && live.has(holding.providerId) && (
											<span className="shrink-0 text-sm text-ink-3">{tagsFor(capabilities, holding.providerId).join(" · ")}</span>
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

function rowsOf(list: AccountsList, providers: Provider[]): Holding[] {
	const ids = new Set(list.credentials.map((one) => one.credential.providerId));
	for (const view of list.accounts) {
		const agent = list.agents.find((one) => one.backendId === view.account.backendId && one.supportsAccounts);
		if (agent?.providerId !== undefined) ids.add(agent.providerId);
	}
	const signedOut = (holding: Holding) => Number(countsText(holding) === "");
	return [...ids].map((id) => holdingOf(list, providers, id)).sort((a, b) => signedOut(a) - signedOut(b) || byName(a, b));
}

function rowDetail(holding: Holding, enabledModels: Record<string, string[]>): string {
	const counts = countsText(holding);
	if (counts === "") return "Signed out";
	const live = liveOf(holding);
	if (live.length === 0) return counts;
	const models = modelsShownText(enabledModels[holding.providerId]);
	const custom = live[0]?.credential.custom;
	if (custom) return `${custom.api === "responses" ? "Responses" : "Chat Completions"} · ${models}`;
	return `${counts} · ${models}`;
}

/** The row's own word for the filter: the count when there is one, else all. */
function modelsShownText(enabled: string[] | undefined): string {
	if (enabled === undefined) return "All models";
	return enabled.length === 1 ? "1 model" : `${enabled.length} models`;
}

function Adding({
	provider,
	way,
	agents,
	onWay,
	onAdded,
	onConnected,
	onRefused,
	onCancel,
}: {
	provider: Provider;
	way: Way | null;
	agents: AccountAgent[];
	onWay(way: Way): void;
	onAdded(made: AccountView, email: string | undefined, providerId: string): void;
	onConnected(providerId: string): Promise<void>;
	onRefused(): void;
	onCancel(): void;
}) {
	if (way === null) {
		const agent = agents.find((one) => one.supportsAccounts && one.providerId === provider.id);
		return (
			<section>
				<h3 className="group-title">Connect {provider.name}</h3>
				<div className="grouped">
					{agent !== undefined && (
						<button type="button" className="group-row group-row-choice w-full text-left" onClick={() => onWay({ agent })}>
							<span className="group-row-text">{subscriptionWay(provider)}</span>
							<ChevronRightIcon />
						</button>
					)}
					{provider.credentialKinds.map((kind) => (
						<button key={kind} type="button" className="group-row group-row-choice w-full text-left" onClick={() => onWay({ kinds: [kind] })}>
							<span className="group-row-text">{connectionMethod(kind)}</span>
							<ChevronRightIcon />
						</button>
					))}
					<div className="group-row justify-end">
						<button type="button" className="control btn-quiet" onClick={onCancel}>
							Cancel
						</button>
					</div>
				</div>
			</section>
		);
	}
	// A provider on the add list holds no account yet, so no name is taken.
	if ("agent" in way) {
		return (
			<div>
				<AddAccountForm
					backendId={way.agent.backendId}
					agentName={way.agent.name}
					takesEmail={way.agent.takesEmail}
					taken={[]}
					onAdded={(made, email) => onAdded(made, email, provider.id)}
					onRefused={onRefused}
					onCancel={onCancel}
				/>
				<p className="group-hint">{NEVER_SEES_SIGN_IN}</p>
			</div>
		);
	}
	return (
		<ConnectProvider
			key={provider.id}
			provider={provider}
			kinds={way.kinds}
			taken={[]}
			onConnected={(made) => void onConnected(made.providerId)}
			onCancel={onCancel}
		/>
	);
}

/** Anthropic's plan is sold as Claude, so its way in says so. */
function subscriptionWay(provider: Provider): string {
	return provider.id === "anthropic" ? "Claude subscription" : `${provider.name} subscription`;
}
