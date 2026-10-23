import { type Dispatch, type SetStateAction, useEffect, useState } from "react";
import type { AccountAgent, AccountsList, AccountView, RosterEntry } from "../../generated/contract";
import { relativeTime } from "../../dock";
import { ChevronRightIcon } from "../../icons";
import { showPath } from "../../serverFiles";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { AccountSignIn, settledAccounts } from "../accounts/AccountSignIn";
import { RenameField } from "../accounts/RenameField";
import { namesList, namesOf, statusWords, usedByWords, whoLine } from "../accounts/words";
import { AccountCard, AccountFrame, Confirm } from "./AccountFrame";
import { AccountSetup } from "./AccountSetup";

type Signing = { accountId: string; email: string | undefined; renewing: boolean };

/**
 * A CLI account's sign-in outlives the page it started on: the provider's
 * page and the account's own page both show it, and leaving either cancels it.
 */
export type CliAccounts = {
	roster: RosterEntry[];
	signing: Signing | null;
	signInOnDesk: string | null;
	signedInAgain: Set<string>;
	check(view: AccountView): void;
	signIn(view: AccountView, email?: string): void;
	signInEnded(signedIn: boolean): void;
	added(view: AccountView, email: string | undefined): void;
	leave(): void;
};

export function useCliAccounts({
	roster,
	setList,
	refuse,
	reload,
}: {
	roster: RosterEntry[];
	setList: Dispatch<SetStateAction<AccountsList | null>>;
	refuse(message: string | null): void;
	reload(): Promise<void>;
}): CliAccounts {
	const [signing, setSigning] = useState<Signing | null>(null);
	const [settling, setSettling] = useState<{ accountId: string } | null>(null);
	const [signInOnDesk, setSignInOnDesk] = useState<string | null>(null);
	const [signedInAgain, setSignedInAgain] = useState<Set<string>>(new Set());

	// biome-ignore lint/correctness/useExhaustiveDependencies: one read per ended sign-in; setList and refuse are setters.
	useEffect(() => {
		if (settling === null) return;
		let cancelled = false;
		settledAccounts(settling.accountId, () => !cancelled)
			.then((settled) => {
				if (settled !== null) setList(settled);
			})
			.catch((error: Error) => {
				if (!cancelled) refuse(error.message);
			});
		return () => {
			cancelled = true;
		};
	}, [settling]);

	const patch = (id: string, change: (view: AccountView) => AccountView) =>
		setList((was) => was && { ...was, accounts: was.accounts.map((one) => (one.account.id === id ? change(one) : one)) });

	const signIn = (view: AccountView, email?: string) => {
		refuse(null);
		setSigning({ accountId: view.account.id, email, renewing: view.account.loginAt !== undefined });
	};

	return {
		roster,
		signing,
		signInOnDesk,
		signedInAgain,
		check: (view) => {
			const id = view.account.id;
			const before = view.status;
			refuse(null);
			patch(id, (one) => ({ ...one, status: { ...one.status, state: "checking" } }));
			wire
				.command("accounts.status", { accountId: id })
				.then((status) => patch(id, (one) => ({ ...one, status })))
				.catch((error: Error) => {
					patch(id, (one) => ({ ...one, status: before }));
					refuse(error.message);
				});
		},
		signIn,
		signInEnded: (signedIn) => {
			const ended = signing;
			if (ended === null) return;
			if (signedIn && ended.renewing) setSignedInAgain((was) => new Set(was).add(ended.accountId));
			setSigning(null);
			setSettling({ accountId: ended.accountId });
		},
		added: (made, email) => {
			setList((was) => was && { ...was, accounts: [...was.accounts.filter((one) => one.account.id !== made.account.id), made] });
			if (made.canSignInHere) signIn(made, email);
			else setSignInOnDesk(made.account.id);
			void reload();
		},
		/* Leaving a page leaves its sign-in: unmounting it cancels the attempt. */
		leave: () => {
			if (signing !== null) {
				setSigning(null);
				setSettling({ accountId: signing.accountId });
			}
			setSignInOnDesk(null);
		},
	};
}

function renewedWords(name: string): string {
	return `${name} has a new sign-in. Its teammates start fresh once their work ends.`;
}

function dotState(view: AccountView, signingHere: boolean): string | undefined {
	if (signingHere || view.attempt?.state === "running") return "signing";
	if (view.attempt?.state === "failed" && view.status.state !== "signed_in") return "failed";
	return view.status.state === "not_checked" ? undefined : view.status.state;
}

function checkedWords(agentName: string, checkedAt: number): string {
	const when = relativeTime(checkedAt);
	if (when === "now") return `Checked with ${agentName} just now`;
	return /^\d+[mhd]$/.test(when) ? `Checked with ${agentName} ${when} ago` : `Checked with ${agentName} on ${when}`;
}

export function CliAccountRow({
	view,
	cli,
	busy,
	onOpen,
	onRename,
}: {
	view: AccountView;
	cli: CliAccounts;
	busy: boolean;
	onOpen(): void;
	onRename(name: string): Promise<void>;
}) {
	const [renaming, setRenaming] = useState(false);
	const signingHere = cli.signing?.accountId === view.account.id;
	const state = dotState(view, signingHere);
	const status = view.status;
	const failed = !signingHere && view.attempt?.state === "failed";
	const dot = state === undefined ? <span className="agent-account-nodot" /> : <span className="agent-account-dot" data-state={state} aria-hidden="true" />;
	if (renaming) {
		return (
			<div className="group-row">
				{dot}
				<RenameField
					name={view.account.name}
					busy={busy}
					onSave={async (next) => {
						await onRename(next);
						setRenaming(false);
					}}
					onCancel={() => setRenaming(false)}
				/>
			</div>
		);
	}
	const detail =
		state === "signing"
			? "Signing in…"
			: failed
				? `${status.state === "signed_in" ? "Signed in · " : ""}Sign-in did not finish${view.attempt?.error ? `: ${view.attempt.error}` : ""}`
				: `${statusWords(status.state)}${status.state === "signed_in" && status.email ? ` · ${status.email}` : ""} · ${usedByWords(namesOf(view.usedBy, cli.roster))}`;
	return (
		<div className="group-row agent-account-row">
			<button type="button" className="agent-account-open" onClick={onOpen}>
				{dot}
				<span className="group-row-text">
					<span className="group-row-title">{view.account.name}</span>
					<span className="group-row-detail">{detail}</span>
					{cli.signedInAgain.has(view.account.id) && <span className="block text-sm text-ink-2">{renewedWords(view.account.name)}</span>}
				</span>
			</button>
			{failed ? (
				view.canSignInHere ? (
					<button type="button" className="control btn-quiet" onClick={() => cli.signIn(view)}>
						Try again
					</button>
				) : (
					<span className="shrink-0 text-sm text-ink-3">Sign in from the desk itself</span>
				)
			) : (
				state !== "signing" &&
				(status.state === "not_checked" || status.state === "unavailable") && (
					<button type="button" className="control btn-quiet" onClick={() => cli.check(view)}>
						Check
					</button>
				)
			)}
			<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setRenaming(true)}>
				Rename
			</button>
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</div>
	);
}

export function CliAccountPage({
	agent,
	view,
	cli,
	refusal,
	refuse,
	onBack,
	onRenamed,
	onRemoved,
}: {
	agent: AccountAgent;
	view: AccountView;
	cli: CliAccounts;
	refusal: string | null;
	refuse(message: string | null): void;
	onBack(): void;
	onRenamed(account: AccountView): void;
	onRemoved(): void;
}) {
	const [renaming, setRenaming] = useState(false);
	const [removing, setRemoving] = useState(false);
	const [busy, setBusy] = useState(false);
	const name = view.account.name;
	const status = view.status;
	const signingHere = cli.signing?.accountId === view.account.id;
	const signing = signingHere || view.attempt?.state === "running";
	const signedIn = status.state === "signed_in";
	const who = signedIn ? whoLine(status) : "";
	const users = namesOf(view.usedBy, cli.roster);
	const state = dotState(view, signingHere);
	const checkedLine =
		status.state === "signed_out"
			? "Its teammates ask you to sign in at their next message"
			: status.checkedAt !== undefined
				? checkedWords(agent.name, status.checkedAt)
				: undefined;

	const rename = async (next: string) => {
		onRenamed(await wire.command("accounts.rename", { accountId: view.account.id, name: next }));
		refuse(null);
		setRenaming(false);
	};

	const remove = async () => {
		if (busy) return;
		setBusy(true);
		refuse(null);
		try {
			await wire.command("accounts.remove", { accountId: view.account.id });
			onRemoved();
		} catch (error) {
			refuse(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	return (
		<AccountFrame title={name} onBack={onBack}>
			<AccountCard
				name={name}
				title={
					renaming ? (
						<RenameField name={name} busy={busy} onSave={rename} onCancel={() => setRenaming(false)} />
					) : (
						<span className="flex min-w-0 items-center gap-2">
							<span className="welcome-card-title truncate">{name}</span>
							<button type="button" className="control btn-quiet btn-sm" disabled={busy} onClick={() => setRenaming(true)}>
								Rename
							</button>
						</span>
					)
				}
				detail={
					<>
						{state === undefined ? null : <span className="agent-account-dot" data-state={state} aria-hidden="true" />}
						{signing ? "Signing in…" : statusWords(status.state)} · {agent.name}
					</>
				}
			/>
			<section>
				<h3 className="group-title">Sign-in</h3>
				<div className="grouped">
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">{who !== "" ? who : statusWords(status.state)}</span>
							{checkedLine !== undefined && <span className="group-row-detail">{checkedLine}</span>}
						</span>
						{!signing && (
							<button type="button" className="control btn-quiet" disabled={status.state === "checking"} onClick={() => cli.check(view)}>
								{status.state === "checking" ? "Checking…" : "Check"}
							</button>
						)}
					</div>
					{cli.signedInAgain.has(view.account.id) && <p className="group-row text-sm">{renewedWords(name)}</p>}
					{signingHere && (
						<div className="group-row flex-col items-stretch">
							<AccountSignIn account={view} agentName={agent.name} email={cli.signing?.email} onEnd={cli.signInEnded} />
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
								{name} can't be removed while {namesList(users)} {users.length === 1 ? "uses" : "use"} it.
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
						<Confirm
							question={`Remove ${name}?`}
							detail={`Hotline forgets ${name}. Its folder and the sign-in in it stay ${view.folder !== undefined ? `at ${view.folder}` : "on the desk"}; sign out there with ${agent.name}'s own sign-out if you no longer need it.`}
							action="Remove"
							busy={busy}
							onConfirm={() => void remove()}
							onCancel={() => setRemoving(false)}
						/>
					)}
				</section>
			)}
			{refusal !== null && <Refusal message={refusal} />}
			<div className="flex items-center justify-between gap-2">
				<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={() => setRemoving(true)}>
					Remove account
				</button>
				{!view.canSignInHere ? (
					<span className="text-sm text-ink-3">Sign in from the desk itself</span>
				) : (
					!signing && (
						<button type="button" className="control btn btn-primary" onClick={() => cli.signIn(view)}>
							{signedIn ? "Sign in again" : "Sign in"}
						</button>
					)
				)}
			</div>
		</AccountFrame>
	);
}
