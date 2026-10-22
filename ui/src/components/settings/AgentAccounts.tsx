import { Fragment, useEffect, useState } from "react";
import type { AccountAgent, AccountsList, AccountView, RosterEntry } from "../../generated/contract";
import { relativeTime } from "../../dock";
import { ChevronRightIcon, PlusIcon } from "../../icons";
import { showPath } from "../../serverFiles";
import { initialOf } from "../../ui/Avatar";
import { BackKey, Band } from "../../ui/Band";
import { Refusal } from "../../ui/Refusal";
import { Scroll } from "../../ui/Scroll";
import { wire } from "../../wire";
import { AccountSignIn } from "../accounts/AccountSignIn";
import { AddAccountForm } from "../accounts/AddAccountForm";
import { namesList, statusWords, whoLine } from "../accounts/words";
import { AccountSetup } from "./AccountSetup";

/** A sign-in this window started: whose, the email to fill in, and whether the account had one already. */
type Signing = { accountId: string; email: string | undefined; renewing: boolean };

/** What the section's pages share: who is on the desk, the sign-in in flight, the one refusal, and what changes them. */
type Shared = {
	roster: RosterEntry[];
	signing: Signing | null;
	fromDesk: string | null;
	renewed: Set<string>;
	refusal: string | null;
	refuse(message: string | null): void;
	check(view: AccountView): void;
	signIn(view: AccountView, email?: string): void;
	signInEnded(signedIn: boolean): void;
};

/**
 * Settings › Agent accounts: the agents this desk runs, an agent's accounts,
 * and one account's page — Providers' shape, a level deeper. An account is
 * one sign-in to an agent's own CLI, kept in a folder of its own; Hotline
 * holds its name and asks the agent whether it is signed in, and never sees
 * a password or token. The list is read on open and again after anything
 * here changes it.
 */
export function AgentAccountsSection({
	roster,
	opening,
	onBack,
}: {
	roster: RosterEntry[];
	opening?: string | undefined;
	onBack?: (() => void) | undefined;
}) {
	const [list, setList] = useState<AccountsList | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [agentId, setAgentId] = useState<string | null>(null);
	const [accountId, setAccountId] = useState<string | null>(opening ?? null);
	const [signing, setSigning] = useState<Signing | null>(null);
	/* An account added from a seat that cannot sign in (an owner, on a Windows desk): its row says where to. */
	const [fromDesk, setFromDesk] = useState<string | null>(null);
	/* Accounts signed in again here: their teammates start fresh, and the rows say so until the section closes. */
	const [renewed, setRenewed] = useState<Set<string>>(new Set());

	const reload = () =>
		wire
			.command("accounts.list", {})
			.then(setList)
			.catch((error: Error) => setRefusal(error.message));

	// biome-ignore lint/correctness/useExhaustiveDependencies: read once, on open; reload reads only setters.
	useEffect(() => {
		void reload();
	}, []);

	const patch = (id: string, change: (view: AccountView) => AccountView) =>
		setList((was) => was && { ...was, accounts: was.accounts.map((one) => (one.account.id === id ? change(one) : one)) });

	const shared: Shared = {
		roster,
		signing,
		fromDesk,
		renewed,
		refusal,
		refuse: setRefusal,
		check: (view) => {
			const id = view.account.id;
			const before = view.status;
			setRefusal(null);
			patch(id, (one) => ({ ...one, status: { ...one.status, state: "checking" } }));
			wire
				.command("accounts.status", { accountId: id })
				.then((status) => patch(id, (one) => ({ ...one, status })))
				.catch((error: Error) => {
					patch(id, (one) => ({ ...one, status: before }));
					setRefusal(error.message);
				});
		},
		signIn: (view, email) => {
			setRefusal(null);
			setSigning({ accountId: view.account.id, email, renewing: view.account.loginAt !== undefined });
		},
		signInEnded: (signedIn) => {
			const ended = signing;
			if (signedIn && ended?.renewing) setRenewed((was) => new Set(was).add(ended.accountId));
			setSigning(null);
			void reload();
		},
	};

	/* Leaving a page leaves its sign-in: unmounting it cancels the attempt. */
	const go = (agent: string | null, account: string | null) => {
		if (signing !== null) {
			setSigning(null);
			void reload();
		}
		setRefusal(null);
		setFromDesk(null);
		setAgentId(agent);
		setAccountId(account);
	};

	const agentOf = agentId ?? list?.accounts.find((one) => one.account.id === accountId)?.account.backendId ?? null;
	const agent = list?.agents.find((one) => one.backendId === agentOf && one.supportsAccounts);
	const accounts = list?.accounts.filter((one) => one.account.backendId === agent?.backendId) ?? [];
	const opened = accounts.find((one) => one.account.id === accountId);

	if (agent !== undefined && opened !== undefined) {
		return (
			<AccountPage
				key={opened.account.id}
				agent={agent}
				view={opened}
				shared={shared}
				onBack={() => go(agent.backendId, null)}
				onRenamed={(renamed) => {
					patch(renamed.account.id, () => renamed);
					void reload();
				}}
				onRemoved={() => {
					go(agent.backendId, null);
					void reload();
				}}
			/>
		);
	}
	if (agent !== undefined) {
		return (
			<AgentPage
				agent={agent}
				accounts={accounts}
				shared={shared}
				onBack={() => go(null, null)}
				onOpen={(id) => go(agent.backendId, id)}
				onAdded={(made, email) => {
					setList((was) => was && { ...was, accounts: [...was.accounts.filter((one) => one.account.id !== made.account.id), made] });
					// A sign-in starts only where this seat may run one; elsewhere the row says where to.
					if (made.canSignInHere) shared.signIn(made, email);
					else setFromDesk(made.account.id);
					void reload();
				}}
			/>
		);
	}

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Agent accounts</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<p className="text-ink-2">
						An account is one sign-in to an agent's own CLI, kept in a folder on this computer. Teammates on the same agent can each use a
						different account and work at the same time.
					</p>
					<section>
						<h3 className="group-title">Agents</h3>
						<div className="grouped">
							{list === null ? (
								<p className="group-row text-sm text-ink-3">Reading…</p>
							) : (
								list.agents.map((one) =>
									one.supportsAccounts ? (
										<AgentRow key={one.backendId} agent={one} accounts={list.accounts} onOpen={() => go(one.backendId, null)} />
									) : (
										<div key={one.backendId} className="group-row">
											<span className="agent-account-initial" aria-hidden="true">
												{initialOf(one.name)}
											</span>
											<span className="group-row-text">
												<span className="group-row-title">{one.name}</span>
												<span className="group-row-detail">Uses its own sign-in</span>
											</span>
										</div>
									),
								)
							)}
						</div>
						<p className="group-hint">
							Accounts are usually added while making a teammate. Hotline never sees a password or token: each agent's CLI keeps its own sign-in.
						</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/** An agent with accounts: how many, and how many are not signed in, which is the thing to see from here. */
function AgentRow({ agent, accounts, onOpen }: { agent: AccountAgent; accounts: AccountView[]; onOpen(): void }) {
	const its = accounts.filter((one) => one.account.backendId === agent.backendId);
	const out = its.filter((one) => one.status.state === "signed_out").length;
	const detail =
		its.length === 0 ? "No accounts yet" : `${its.length} ${its.length === 1 ? "account" : "accounts"}${out > 0 ? ` · ${out} not signed in` : ""}`;
	return (
		<button type="button" className="group-row group-row-choice w-full text-left" onClick={onOpen}>
			<span className="agent-account-initial" aria-hidden="true">
				{initialOf(agent.name)}
			</span>
			<span className="group-row-text">
				<span className="group-row-title">{agent.name}</span>
				<span className={out > 0 ? "group-row-detail text-warn" : "group-row-detail"}>{detail}</span>
			</span>
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</button>
	);
}

/** The names of the teammates on this desk with these ids. */
function namesOf(ids: string[], roster: RosterEntry[]): string[] {
	return ids.map((id) => roster.find((entry) => entry.persona.id === id)?.persona.name ?? id);
}

function usedByWords(names: string[]): string {
	return names.length === 0 ? "Not used yet" : `Used by ${names.join(", ")}`;
}

function renewedWords(name: string): string {
	return `${name} has a new sign-in. Its teammates start fresh once their work ends.`;
}

/** The dot's state: the sign-in in flight, then a last one that failed, then what the agent said; none before anyone asked. */
function dotState(view: AccountView, signingHere: boolean): string | undefined {
	if (signingHere || view.attempt?.state === "running") return "signing";
	if (view.attempt?.state === "failed") return "failed";
	return view.status.state === "not_checked" ? undefined : view.status.state;
}

/**
 * One agent's accounts: the add row, System default — the sign-in the agent
 * has without Hotline, built in and never removed — and a row per account,
 * which opens the account's page. A sign-in started here runs under its row.
 */
function AgentPage({
	agent,
	accounts,
	shared,
	onBack,
	onOpen,
	onAdded,
}: {
	agent: AccountAgent;
	accounts: AccountView[];
	shared: Shared;
	onBack(): void;
	onOpen(accountId: string): void;
	onAdded(account: AccountView, email: string | undefined): void;
}) {
	const [adding, setAdding] = useState(false);
	// Only this desk's own teammates run on its System default, as only they can be on its accounts.
	const onDefault = shared.roster
		.filter((entry) => entry.persona.node === undefined && entry.persona.backendId === agent.backendId && entry.persona.agentAccountId === undefined)
		.map((entry) => entry.persona.name);
	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{agent.name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					{adding && (
						<AddAccountForm
							backendId={agent.backendId}
							agentName={agent.name}
							takesEmail={agent.takesEmail}
							taken={accounts.map((one) => one.account.name)}
							onAdded={(made, email) => {
								setAdding(false);
								onAdded(made, email);
							}}
							onCancel={() => setAdding(false)}
						/>
					)}
					<section>
						<h3 className="group-title">Accounts</h3>
						<div className="grouped">
							{!adding && (
								<button type="button" className="group-row group-row-add" onClick={() => setAdding(true)}>
									<PlusIcon />
									Add account
								</button>
							)}
							<div className="group-row">
								<span className="agent-account-dot" aria-hidden="true" />
								<span className="group-row-text">
									<span className="group-row-title">System default</span>
									<span className="group-row-detail">
										{agent.name}'s own sign-in on this computer, as before · {usedByWords(onDefault)}
									</span>
								</span>
								<span className="agent-account-chip">Built in</span>
							</div>
							{accounts.map((view) => {
								const signingHere = shared.signing?.accountId === view.account.id;
								return (
									<Fragment key={view.account.id}>
										<AccountRow view={view} shared={shared} signingHere={signingHere} onOpen={() => onOpen(view.account.id)} />
										{signingHere && (
											<div className="group-row flex-col items-stretch">
												<AccountSignIn account={view} agentName={agent.name} email={shared.signing?.email} onEnd={shared.signInEnded} />
											</div>
										)}
										{shared.fromDesk === view.account.id && <p className="group-row text-sm text-ink-3">Sign in from the desk itself</p>}
									</Fragment>
								);
							})}
						</div>
						<p className="group-hint">
							{agent.name} keeps each account's sign-in, history and settings in its own folder. An API key or token in Hotline's environment is not
							passed to a teammate on an account, so it cannot stand in for the sign-in.
						</p>
					</section>
					{shared.refusal !== null && <Refusal message={shared.refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/**
 * An account's row: its dot, its name, and one line — the sign-in in
 * flight, a last one that did not finish, or what the agent said, the email
 * and who uses it. The row opens the account's page; the one key beside it
 * is what the line asks for: Check, or Try again — or, from a seat that
 * cannot sign in, where to.
 */
function AccountRow({ view, shared, signingHere, onOpen }: { view: AccountView; shared: Shared; signingHere: boolean; onOpen(): void }) {
	const state = dotState(view, signingHere);
	const status = view.status;
	const failed = !signingHere && view.attempt?.state === "failed";
	const detail =
		state === "signing"
			? "Signing in…"
			: failed
				? `Sign-in did not finish${view.attempt?.error ? `: ${view.attempt.error}` : ""}`
				: `${statusWords(status.state)}${status.state === "signed_in" && status.email ? ` · ${status.email}` : ""} · ${usedByWords(namesOf(view.usedBy, shared.roster))}`;
	return (
		<div className="group-row agent-account-row">
			<button type="button" className="agent-account-open" onClick={onOpen}>
				{state === undefined ? <span className="agent-account-nodot" /> : <span className="agent-account-dot" data-state={state} aria-hidden="true" />}
				<span className="group-row-text">
					<span className="group-row-title">{view.account.name}</span>
					<span className="group-row-detail">{detail}</span>
					{shared.renewed.has(view.account.id) && <span className="block text-sm text-ink-2">{renewedWords(view.account.name)}</span>}
				</span>
			</button>
			{failed ? (
				view.canSignInHere ? (
					<button type="button" className="control btn-quiet" onClick={() => shared.signIn(view)}>
						Try again
					</button>
				) : (
					<span className="shrink-0 text-sm text-ink-3">Sign in from the desk itself</span>
				)
			) : (
				state !== "signing" &&
				(status.state === "not_checked" || status.state === "unavailable") && (
					<button type="button" className="control btn-quiet" onClick={() => shared.check(view)}>
						Check
					</button>
				)
			)}
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</div>
	);
}

/** When the agent last said so: "just now", "5m ago", or the day. */
function checkedWords(agentName: string, checkedAt: number): string {
	const when = relativeTime(checkedAt);
	if (when === "now") return `Checked with ${agentName} just now`;
	return /^\d+[mhd]$/.test(when) ? `Checked with ${agentName} ${when} ago` : `Checked with ${agentName} on ${when}`;
}

/**
 * One account's page: its name, its sign-in and Check, who uses it, its
 * setup, its folder, and the way out at the foot. Remove asks in place, and
 * an account a teammate uses is not removed: Hotline never moves a teammate
 * to another account for you.
 */
function AccountPage({
	agent,
	view,
	shared,
	onBack,
	onRenamed,
	onRemoved,
}: {
	agent: AccountAgent;
	view: AccountView;
	shared: Shared;
	onBack(): void;
	onRenamed(account: AccountView): void;
	onRemoved(): void;
}) {
	const [renaming, setRenaming] = useState<string | null>(null);
	const [removing, setRemoving] = useState(false);
	const [busy, setBusy] = useState(false);
	const name = view.account.name;
	const status = view.status;
	const signingHere = shared.signing?.accountId === view.account.id;
	const signedIn = status.state === "signed_in";
	const who = signedIn ? whoLine(status) : "";
	const users = namesOf(view.usedBy, shared.roster);
	const state = dotState(view, signingHere);
	const checkedLine =
		status.state === "signed_out"
			? "Its teammates ask you to sign in at their next message"
			: status.checkedAt !== undefined
				? checkedWords(agent.name, status.checkedAt)
				: undefined;

	const rename = async () => {
		const next = (renaming ?? "").trim();
		setRenaming(null);
		if (next === "" || next === name) return;
		shared.refuse(null);
		try {
			onRenamed(await wire.command("accounts.rename", { accountId: view.account.id, name: next }));
		} catch (error) {
			shared.refuse(error instanceof Error ? error.message : String(error));
		}
	};

	const remove = async () => {
		if (busy) return;
		setBusy(true);
		shared.refuse(null);
		try {
			await wire.command("accounts.remove", { accountId: view.account.id });
			onRemoved();
		} catch (error) {
			shared.refuse(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

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
								{initialOf(name)}
							</span>
							<span className="flex min-w-0 flex-1 flex-col gap-1">
								{renaming !== null ? (
									<span className="flex gap-2">
										<input
											className="field min-w-0 flex-1"
											aria-label="Account name"
											value={renaming}
											onChange={(event) => setRenaming(event.target.value)}
											onKeyDown={(event) => {
												if (event.key === "Enter") void rename();
												if (event.key === "Escape") {
													// Escape here undoes the rename; it does not also leave Settings.
													event.stopPropagation();
													setRenaming(null);
												}
											}}
										/>
										<button type="button" className="control btn btn-primary" onClick={() => void rename()}>
											Save
										</button>
									</span>
								) : (
									<span className="flex min-w-0 items-center gap-2">
										<span className="welcome-card-title truncate">{name}</span>
										<button type="button" className="control btn-quiet btn-sm" onClick={() => setRenaming(name)}>
											Rename
										</button>
									</span>
								)}
								<span className="flex items-center gap-2 text-sm text-ink-2">
									{state === undefined ? null : <span className="agent-account-dot" data-state={state} aria-hidden="true" />}
									{signingHere ? "Signing in…" : statusWords(status.state)} · {agent.name}
								</span>
							</span>
						</div>
					</section>
					<section>
						<h3 className="group-title">Sign-in</h3>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{who !== "" ? who : statusWords(status.state)}</span>
									{checkedLine !== undefined && <span className="group-row-detail">{checkedLine}</span>}
								</span>
								{!signingHere && (
									<button type="button" className="control btn-quiet" disabled={status.state === "checking"} onClick={() => shared.check(view)}>
										{status.state === "checking" ? "Checking…" : "Check"}
									</button>
								)}
							</div>
							{shared.renewed.has(view.account.id) && <p className="group-row text-sm">{renewedWords(name)}</p>}
							{signingHere && (
								<div className="group-row flex-col items-stretch">
									<AccountSignIn account={view} agentName={agent.name} email={shared.signing?.email} onEnd={shared.signInEnded} />
								</div>
							)}
						</div>
					</section>
					<section>
						<h3 className="group-title">Used by</h3>
						<div className="grouped">
							{users.length > 0 ? (
								<div className="group-row flex-wrap gap-2">
									{users.map((one) => (
										<span key={one} className="agent-account-chip">
											{one}
										</span>
									))}
								</div>
							) : (
								<p className="group-row text-sm text-ink-3">No teammate uses it yet. Pick it in a {agent.name} teammate's settings.</p>
							)}
						</div>
					</section>
					<AccountSetup accountId={view.account.id} agentName={agent.name} />
					{view.folder !== undefined && (
						<section>
							<h3 className="group-title">Folder</h3>
							<div className="grouped">
								<div className="group-row">
									<span className="selectable min-w-0 flex-1 break-all font-mono text-sm text-ink-3">{view.folder}</span>
									<button type="button" className="control btn-quiet" onClick={() => showPath(view.folder ?? "")}>
										Show folder
									</button>
								</div>
							</div>
							<p className="group-hint">{agent.name} keeps this account's sign-in, history and settings here.</p>
						</section>
					)}
					{removing && (
						<section className="grouped">
							{users.length > 0 ? (
								<div className="group-row flex-col items-stretch gap-2">
									<p>
										{name} can't be removed while {namesList(users)} use it.
									</p>
									<p className="text-sm text-ink-3">
										Give them another account in their settings first; Hotline never moves a teammate to another account for you.
									</p>
									<div className="flex justify-end">
										<button type="button" className="control btn" onClick={() => setRemoving(false)}>
											OK
										</button>
									</div>
								</div>
							) : (
								<div className="group-row flex-col items-stretch gap-2">
									<p>Remove {name}?</p>
									<p className="text-sm text-ink-3">
										Hotline forgets {name}. Its folder and the sign-in in it stay {view.folder !== undefined ? `at ${view.folder}` : "on the desk"}; sign out
										there with {agent.name}'s own sign-out if you no longer need it.
									</p>
									<div className="flex justify-end gap-2">
										<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setRemoving(false)}>
											Cancel
										</button>
										<button type="button" className="control btn btn-danger" disabled={busy} onClick={() => void remove()}>
											Remove
										</button>
									</div>
								</div>
							)}
						</section>
					)}
					{shared.refusal !== null && <Refusal message={shared.refusal} />}
					<div className="flex items-center justify-between gap-2">
						<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => setRemoving(true)}>
							Remove account
						</button>
						{!view.canSignInHere ? (
							<span className="text-sm text-ink-3">Sign in from the desk itself</span>
						) : (
							!signingHere && (
								<button type="button" className="control btn btn-primary" onClick={() => shared.signIn(view)}>
									{signedIn ? "Sign in again" : "Sign in"}
								</button>
							)
						)}
					</div>
				</div>
			</Scroll>
		</div>
	);
}
