import { Fragment, type ReactNode, useState } from "react";
import { shortDate } from "../../clock";
import type { AccountAgent, AccountSection, AccountsList, AccountView, Credential, CredentialAccount, CredentialKind, Provider, RosterEntry } from "../../generated/contract";
import { PlusIcon } from "../../icons";
import { openLink } from "../../native";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { AccountSignIn } from "../accounts/AccountSignIn";
import { AddAccountForm } from "../accounts/AddAccountForm";
import { kindsIn } from "../accounts/fits";
import { RenameField } from "../accounts/RenameField";
import { NEVER_SEES_SIGN_IN, namesOf, ownSignIn, THIS_COMPUTER, usedByWords } from "../accounts/words";
import { ConnectProvider } from "../ConnectProvider";
import { CustomProviderForm } from "../CustomProviderForm";
import { AccountCard, AccountFrame, Confirm } from "./AccountFrame";
import { CliAccountRow, type CliAccounts } from "./CliAccountPage";
import { ProviderModels } from "./ProviderModels";

export type Holding = {
	providerId: string;
	name: string;
	provider: Provider | undefined;
	credentials: CredentialAccount[];
	agent: AccountAgent | undefined;
	accounts: AccountView[];
};

export function holdingOf(list: AccountsList, providers: Provider[], providerId: string): Holding {
	const credentials = list.credentials.filter((one) => one.credential.providerId === providerId);
	const agent = list.agents.find((one) => one.supportsAccounts && one.providerId === providerId);
	const provider = providers.find((one) => one.id === providerId);
	return {
		providerId,
		name: provider?.name ?? credentials[0]?.credential.label ?? providerId,
		provider,
		credentials,
		agent,
		accounts: agent === undefined ? [] : list.accounts.filter((one) => one.account.backendId === agent.backendId),
	};
}

export function liveOf(holding: Holding): CredentialAccount[] {
	return holding.credentials.filter((one) => !one.credential.revoked);
}

function plural(count: number, noun: string): string {
	return count === 0 ? "" : `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/** What a provider holds that works, by section; empty when it holds nothing that does. */
export function countsText(holding: Holding): string {
	const live = liveOf(holding);
	const inSection = (section: AccountSection) => live.filter((one) => one.section === section).length;
	return [
		plural(holding.accounts.length + inSection("subscription"), "subscription"),
		plural(inSection("api_key"), "API key"),
		plural(inSection("server"), "server"),
	]
		.filter((part) => part !== "")
		.join(" · ");
}

const SECTIONS: { section: AccountSection; title: string; add: string }[] = [
	{ section: "subscription", title: "Subscriptions", add: "Add subscription" },
	{ section: "api_key", title: "API keys", add: "Add API key" },
	{ section: "server", title: "Servers", add: "Add server" },
];

const CHANGE: Record<CredentialKind, string> = { api_key: "Change key", local: "Change server", oauth: "Sign in again" };
const REMOVE: Record<CredentialKind, string> = { api_key: "Remove key", local: "Disconnect", oauth: "Sign out" };

function removalWords(credential: Credential, providerName: string): { question: string; detail: string } {
	if (credential.credentialKind === "oauth") return { question: `Sign out of ${credential.label}?`, detail: "Hotline forgets this sign-in." };
	if (credential.credentialKind === "local") {
		return { question: `Disconnect ${credential.label}?`, detail: "Hotline forgets this server. The server itself keeps running." };
	}
	return { question: `Remove ${credential.label}?`, detail: `Hotline forgets this key. It still works at ${providerName} until you revoke it there.` };
}

type Form = { add: AccountSection } | { change: CredentialAccount };

export function ProviderPage({
	holding,
	cli,
	enabledModels,
	refusal,
	refuse,
	reload,
	onBack,
	onOpenAccount,
}: {
	holding: Holding;
	cli: CliAccounts;
	enabledModels: Record<string, string[]>;
	refusal: string | null;
	refuse(message: string | null): void;
	reload(): Promise<void>;
	onBack(): void;
	onOpenAccount(accountId: string): void;
}) {
	const [form, setForm] = useState<Form | null>(null);
	const [renaming, setRenaming] = useState<string | null>(null);
	const [removing, setRemoving] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const { provider, agent } = holding;
	const live = liveOf(holding);
	const rowsIn = (section: AccountSection) => holding.credentials.filter((one) => one.section === section);
	const offers = (section: AccountSection) => kindsIn(provider, section).length > 0 || (section === "subscription" && agent !== undefined);
	const adding = (section: AccountSection) => form !== null && "add" in form && form.add === section;

	const run = async (work: () => Promise<unknown>) => {
		if (busy) return;
		setBusy(true);
		refuse(null);
		try {
			await work();
		} catch (error) {
			refuse(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	// A delete may revoke and then fail; what is left is listed as signed out, to remove again.
	const remove = (id: string) =>
		run(async () => {
			try {
				await wire.command("credential.delete", { id });
			} finally {
				setRemoving(null);
				await reload();
			}
		});

	// The form stays until the list has the account it made, so the page never shows it missing.
	const connected = async () => {
		await reload();
		setForm(null);
	};

	// A sign-in cancelled late may still have landed, so a cancel reads the list too.
	const cancelled = () => {
		setForm(null);
		void reload();
	};

	const formView = (open: Form): ReactNode => {
		if ("change" in open) {
			const { credential } = open.change;
			if (provider === undefined) return null;
			return (
				<ConnectProvider
					key={credential.id}
					provider={provider}
					credential={credential}
					onConnected={() => void connected()}
					onCancel={cancelled}
				/>
			);
		}
		if (open.add === "subscription" && agent !== undefined) {
			return (
				<AddAccountForm
					backendId={agent.backendId}
					agentName={agent.name}
					takesEmail={agent.takesEmail}
					taken={holding.accounts.map((one) => one.account.name)}
					onAdded={(made, email) => {
						setForm(null);
						cli.added(made, email);
					}}
					onRefused={() => void reload()}
					onCancel={() => setForm(null)}
				/>
			);
		}
		if (provider === undefined) return null;
		return (
			<ConnectProvider
				key={open.add}
				provider={provider}
				kinds={kindsIn(provider, open.add)}
				taken={live.map((one) => one.credential.label)}
				onConnected={() => void connected()}
				onCancel={cancelled}
			/>
		);
	};

	const twinned = (one: CredentialAccount) =>
		holding.credentials.filter((other) => other.section === one.section && other.credential.label.toLowerCase() === one.credential.label.toLowerCase())
			.length > 1;

	const detailOf = (one: CredentialAccount): string => {
		const { credential } = one;
		if (credential.revoked) return "Signed out";
		const how =
			one.section === "server"
				? (credential.baseUrl ?? "Server")
				: one.section === "subscription"
					? "Signed in"
					: credential.credentialKind === "oauth"
						? `Signed in with ${holding.name} · pay per use`
						: "API key · pay per use";
		return [
			how,
			// Two old accounts may share a name from before names were checked.
			twinned(one) ? `Added ${shortDate.format(credential.createdAt)}` : "",
			one.default && live.length > 1 ? "Default" : "",
			usedByWords(namesOf(one.usedBy, cli.roster)),
		]
			.filter((part) => part !== "")
			.join(" · ");
	};

	const credentialRow = (one: CredentialAccount) => {
		const { credential } = one;
		const dot = <span className="agent-account-dot" data-state={credential.revoked ? undefined : "signed_in"} aria-hidden="true" />;
		if (renaming === credential.id) {
			return (
				<div key={credential.id} className="group-row">
					{dot}
					<RenameField
						name={credential.label}
						busy={busy}
						onSave={async (label) => {
							await wire.command("credential.update", { id: credential.id, label });
							refuse(null);
							await reload();
							setRenaming(null);
						}}
						onCancel={() => setRenaming(null)}
					/>
				</div>
			);
		}
		const confirming = removing === credential.id && !credential.revoked;
		return (
			<Fragment key={credential.id}>
				<div className="group-row">
					{dot}
					<span className="group-row-text">
						<span className="group-row-title">{credential.label}</span>
						<span className="group-row-detail whitespace-normal">{detailOf(one)}</span>
					</span>
					{credential.revoked ? (
						<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => void remove(credential.id)}>
							Remove
						</button>
					) : (
						!confirming && (
							<>
								<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setRenaming(credential.id)}>
									Rename
								</button>
								{provider !== undefined && (
									<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setForm({ change: one })}>
										{CHANGE[credential.credentialKind]}
									</button>
								)}
								<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => setRemoving(credential.id)}>
									{REMOVE[credential.credentialKind]}
								</button>
							</>
						)
					)}
				</div>
				{confirming && (
					<Confirm
						{...removalWords(credential, holding.name)}
						action={REMOVE[credential.credentialKind]}
						busy={busy}
						onConfirm={() => void remove(credential.id)}
						onCancel={() => setRemoving(null)}
					/>
				)}
			</Fragment>
		);
	};

	const cliRows =
		agent === undefined
			? null
			: holding.accounts.map((view) => (
					<Fragment key={view.account.id}>
						<CliAccountRow
							view={view}
							cli={cli}
							busy={busy}
							onOpen={() => onOpenAccount(view.account.id)}
							onRename={async (name) => {
								await wire.command("accounts.rename", { accountId: view.account.id, name });
								refuse(null);
								await reload();
							}}
						/>
						{cli.signing?.accountId === view.account.id && (
							<div className="group-row flex-col items-stretch">
								<AccountSignIn account={view} agentName={agent.name} email={cli.signing.email} onEnd={cli.signInEnded} />
							</div>
						)}
						{cli.signInOnDesk === view.account.id && <p className="group-row text-sm text-ink-3">Sign in from the desk itself</p>}
					</Fragment>
				));

	const shown = SECTIONS.filter(({ section }) => offers(section) || rowsIn(section).length > 0);

	return (
		<AccountFrame title={holding.name} onBack={onBack}>
			<AccountCard
				name={holding.name}
				detail={<span className="truncate">{countsText(holding) || (holding.credentials.length > 0 ? "Signed out" : "No accounts yet")}</span>}
			/>
			{form !== null && formView(form)}
			<section>
				<div className="flex flex-col gap-6">
					{shown.map(({ section, title, add }) => (
						<div key={section}>
							<h3 className="group-title">{title}</h3>
							<div className="grouped">
								{offers(section) && !adding(section) && (
									<button type="button" className="group-row group-row-add" onClick={() => setForm({ add: section })}>
										<PlusIcon />
										{add}
									</button>
								)}
								{section === "subscription" && cliRows}
								{rowsIn(section).map(credentialRow)}
								{section === "subscription" && agent !== undefined && <ThisComputerRow agent={agent} roster={cli.roster} />}
							</div>
							{section === "subscription" && agent !== undefined && (
								<p className="group-hint">
									{NEVER_SEES_SIGN_IN} {agent.name} keeps each account's sign-in, history and settings in its own folder. An API key or token in
									Hotline's environment is not passed to a teammate on an account, so it cannot stand in for the sign-in.
								</p>
							)}
							{section === "subscription" && holding.providerId === "github-copilot" && rowsIn(section).length > 0 && (
								<p className="group-hint">
									Sign out removes a login from Hotline. To revoke GitHub access too, use{" "}
									<button
										type="button"
										className="control btn-quiet"
										onClick={() => void openLink("https://github.com/settings/applications").catch((error: unknown) => refuse(String(error)))}
									>
										Revoke at GitHub
									</button>
									.
								</p>
							)}
						</div>
					))}
				</div>
				<p className="group-hint">A teammate on {holding.name} picks one of these when it has more than one to choose from.</p>
			</section>
			{refusal !== null && <Refusal message={refusal} />}
			{live.length > 0 && (
				<ProviderModels providerId={holding.providerId} discover={provider?.modelDiscovery ?? false} custom={false} enabledModels={enabledModels} />
			)}
		</AccountFrame>
	);
}

function ThisComputerRow({ agent, roster }: { agent: AccountAgent; roster: RosterEntry[] }) {
	// Only this desk's own teammates run on This computer, as only they can be on its accounts.
	const onDefault = roster
		.filter((entry) => entry.persona.node === undefined && entry.persona.backendId === agent.backendId && entry.persona.accountId === undefined)
		.map((entry) => entry.persona.name);
	return (
		<div className="group-row">
			<span className="agent-account-dot" aria-hidden="true" />
			<span className="group-row-text">
				<span className="group-row-title">{THIS_COMPUTER}</span>
				<span className="group-row-detail">
					{ownSignIn(agent.name)} · {usedByWords(onDefault)}
				</span>
			</span>
			<span className="agent-account-chip">Built in</span>
		</div>
	);
}

/**
 * A custom connection is a provider of its own with one account, so its
 * page has no sections: the connection, its edit form, and its models.
 */
export function ConnectionPage({
	credential,
	enabledModels,
	refusal,
	refuse,
	reload,
	onBack,
}: {
	credential: Credential;
	enabledModels: Record<string, string[]>;
	refusal: string | null;
	refuse(message: string | null): void;
	reload(): Promise<void>;
	onBack(): void;
}) {
	const [editing, setEditing] = useState(false);
	const [removing, setRemoving] = useState(false);
	const [busy, setBusy] = useState(false);
	const how = `${credential.custom?.api === "responses" ? "Responses" : "Chat Completions"} · ${credential.baseUrl ?? ""}`;

	const remove = async () => {
		if (busy) return;
		setBusy(true);
		refuse(null);
		try {
			await wire.command("credential.delete", { id: credential.id });
			onBack();
		} catch (error) {
			refuse(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
			setRemoving(false);
			await reload();
		}
	};

	return (
		<AccountFrame title={credential.label} onBack={onBack}>
			<AccountCard name={credential.label} detail={<span className="truncate">{credential.revoked ? "Signed out" : how}</span>}>
				<div className="flex flex-wrap items-center justify-end gap-2">
					{!credential.revoked && (
						<button type="button" className="control btn" disabled={busy || editing} onClick={() => setEditing(true)}>
							Edit connection
						</button>
					)}
					{!removing && (
						<button
							type="button"
							className="control btn-quiet text-danger"
							disabled={busy}
							onClick={() => (credential.revoked ? void remove() : setRemoving(true))}
						>
							{credential.revoked ? "Remove" : "Remove connection"}
						</button>
					)}
				</div>
			</AccountCard>
			{removing && (
				<section className="grouped">
					<Confirm
						question={`Remove ${credential.label}?`}
						detail="Hotline forgets this connection and any key it keeps for it."
						action="Remove connection"
						busy={busy}
						onConfirm={() => void remove()}
						onCancel={() => setRemoving(false)}
					/>
				</section>
			)}
			{editing && (
				<CustomProviderForm
					credential={credential}
					onSaved={() => {
						setEditing(false);
						void reload();
					}}
					onCancel={() => setEditing(false)}
				/>
			)}
			{refusal !== null && <Refusal message={refusal} />}
			{!credential.revoked && <ProviderModels providerId={credential.providerId} discover={false} custom enabledModels={enabledModels} />}
		</AccountFrame>
	);
}
