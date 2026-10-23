import { useEffect, useState } from "react";
import type { AccountSection, AccountsList, AccountView, Credential, Persona, Provider, SessionInfo } from "../../generated/contract";
import { PlusIcon } from "../../icons";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { AccountSignIn, settledAccounts } from "../accounts/AccountSignIn";
import { AddAccountForm } from "../accounts/AddAccountForm";
import { accountsFor, detailLine, HOTLINE_AGENT, inUse, kindsIn, providerOf } from "../accounts/fits";
import { NEVER_SEES_SIGN_IN, SYSTEM_DEFAULT } from "../accounts/words";
import { ConnectProvider } from "../ConnectProvider";

const ADDS: readonly { section: AccountSection; label: string }[] = [
	{ section: "subscription", label: "Add subscription" },
	{ section: "api_key", label: "Add API key" },
];

type Signing = { view: AccountView; email: string | undefined };

/**
 * The accounts that fit the teammate, as radios. Hotline Agent picks among
 * its model's provider's accounts, a CLI agent among its own sign-ins and
 * This computer; with fewer than two there is nothing to pick, so nothing shows.
 */
export function AccountRow({
	persona,
	session,
	modelId,
	disabled,
	onSetup,
}: {
	persona: Persona;
	session: SessionInfo;
	modelId: string | undefined;
	disabled: boolean;
	onSetup?: ((accountId: string) => void) | undefined;
}) {
	const hotline = persona.backendId === HOTLINE_AGENT;
	const [list, setList] = useState<AccountsList | null>(null);
	const [providers, setProviders] = useState<Provider[] | null>(null);
	const [settling, setSettling] = useState<{ accountId: string } | null>(null);
	const [adding, setAdding] = useState<AccountSection | null>(null);
	const [signing, setSigning] = useState<Signing | null>(null);
	const [fromDesk, setFromDesk] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		let cancelled = false;
		const read = settling === null ? wire.command("accounts.list", {}) : settledAccounts(settling.accountId, () => !cancelled);
		void read.then(
			(answer) => {
				if (!cancelled && answer !== null) setList(answer);
			},
			() => {
				if (!cancelled) setList(null);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [settling]);

	useEffect(() => {
		if (!hotline) return;
		let cancelled = false;
		void wire.command("providers.list", {}).then(
			(answer) => {
				if (!cancelled) setProviders(answer);
			},
			() => {
				if (!cancelled) setProviders([]);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [hotline]);

	if (list === null || (hotline && providers === null)) return null;
	const choices = accountsFor(list, { backendId: persona.backendId, modelId });
	const providerId = providerOf(modelId);
	// The core refuses to run on an account this desk does not have, and on Hotline Agent on a signed-out key of
	// the model's provider; any other account it ignores. A teammate refused so is offered even the only one that fits.
	const strandedOn = (id: string) => {
		const credential = list.credentials.find((one) => one.credential.id === id)?.credential;
		if (credential !== undefined) return hotline && credential.revoked && credential.providerId === providerId;
		return !list.accounts.some((one) => one.account.id === id);
	};
	const set = persona.accountId;
	const stranded = set !== undefined && strandedOn(set);
	const offered = choices.length >= 2 || (stranded && choices.length === 1);
	if (!offered) return null;
	const agent = hotline ? undefined : list.agents.find((one) => one.backendId === persona.backendId && one.supportsAccounts);
	const provider = providers?.find((one) => one.id === providerId);
	const title = `${agent?.name ?? provider?.name ?? providerId} account`;
	// No account is the provider's default on Hotline Agent and This computer on a CLI agent, as is one that does not fit.
	const nameOf = (id: string) => choices.find((one) => one.id === inUse(choices, id))?.name ?? id;

	// An id the core ignores runs on the default, so the default is what is checked; one it refuses checks nothing.
	const chosen = stranded ? set : inUse(choices, set);
	const running = session.account?.running ?? SYSTEM_DEFAULT;
	// The session says where it is going, never the record, which can name an id the core ignores. A switch to no account has no id.
	const wanted = session.account === undefined ? running : (session.account.wanted ?? SYSTEM_DEFAULT);
	const held = session.account?.held;
	const up = session.state === "ready" || session.state === "thinking";
	// As the accounts they resolve to: a switch that lands where the session already runs is no switch.
	const runningNow = inUse(choices, running);
	const switching = inUse(choices, wanted) !== runningNow;
	const pending = up && held === undefined && switching;
	// A teammate whose every turn is refused runs on nothing.
	const inUseNow = up && !stranded && !switching ? runningNow : undefined;
	// The core refuses an account that does not fit the model, so Keep is offered only for one that does.
	const keepable = running === SYSTEM_DEFAULT || choices.some((one) => one.id === running);

	const send = async (step: () => Promise<unknown>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await step();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};
	const assign = (accountId: string) =>
		send(() => wire.command("persona.account", { personaId: persona.id, accountId: accountId === SYSTEM_DEFAULT ? null : accountId }));
	// On Hotline Agent the default's row means following the default, so picking it never pins the default.
	const pick = (accountId: string) => {
		if (accountId === chosen) return;
		const followsDefault = hotline && choices.some((one) => one.id === accountId && one.isDefault);
		void assign(followsDefault ? SYSTEM_DEFAULT : accountId);
	};
	const stopAndSwitch = () =>
		void send(async () => {
			await wire.command("session.stop", { personaId: persona.id });
			await wire.command("session.start", { personaId: persona.id });
		});
	const reload = () => wire.command("accounts.list", {}).then(setList, () => {});

	const added = (made: AccountView, email: string | undefined) => {
		setAdding(null);
		setList((was) => was && { ...was, accounts: [...was.accounts.filter((one) => one.account.id !== made.account.id), made] });
		if (made.canSignInHere) setSigning({ view: made, email });
		else setFromDesk(made.account.id);
	};
	const signInEnded = (signedIn: boolean) => {
		const ended = signing;
		setSigning(null);
		if (ended === null) return;
		if (signedIn) void assign(ended.view.account.id);
		setSettling({ accountId: ended.view.account.id });
	};
	const connected = async (made: Credential) => {
		await reload();
		setAdding(null);
		void assign(made.id);
	};
	// A sign-in cancelled late may still have landed.
	const connectCancelled = () => {
		setAdding(null);
		void reload();
	};
	const offers = (section: AccountSection) => (hotline ? kindsIn(provider, section).length > 0 : section === "subscription");

	const note = hotline
		? `On ${nameOf(running)} · Fresh: a new session on this account, handed this conversation.`
		: session.contextRestored
			? `On ${nameOf(running)} · Restored: ${agent?.name} reopened its own session for this account.`
			: `On ${nameOf(running)} · Fresh: a new ${agent?.name} session for this account, handed this conversation. Another account's session is never reopened here.`;
	return (
		<section>
			<h3 className="label">{title}</h3>
			<div className="grouped" role="radiogroup" aria-label={title}>
				{choices.map((choice) => {
					const isSigning = signing?.view.account.id === choice.id;
					const signedOut = !isSigning && list.accounts.find((view) => view.account.id === choice.id)?.status.state === "signed_out";
					const detail = isSigning ? "Signing in…" : signedOut ? "Not signed in · sign in to use it" : detailLine(choice);
					return (
						<label key={choice.id === SYSTEM_DEFAULT ? "this-computer" : choice.id} className="group-row group-row-choice">
							<input
								type="radio"
								className="radio"
								name={`account-${persona.id}`}
								checked={chosen === choice.id}
								disabled={disabled || busy}
								onChange={() => pick(choice.id)}
							/>
							<span className="group-row-text">
								<span className="group-row-title">{choice.name}</span>
								<span className={signedOut ? "group-row-detail text-warn" : "group-row-detail"} style={{ whiteSpace: "normal" }}>
									{detail}
								</span>
							</span>
							{inUseNow === choice.id && <span className="shrink-0 text-sm text-ink-3">In use</span>}
						</label>
					);
				})}
				{signing !== null && agent !== undefined && (
					<div className="group-row flex-col items-stretch">
						<AccountSignIn account={signing.view} agentName={agent.name} email={signing.email} onEnd={signInEnded} />
					</div>
				)}
				{fromDesk !== null && signing === null && <p className="group-row text-sm text-ink-3">Sign in from the desk itself</p>}
				{adding !== null && agent !== undefined && (
					<AddAccountForm
						compact
						backendId={agent.backendId}
						agentName={agent.name}
						takesEmail={agent.takesEmail}
						taken={list.accounts.filter((one) => one.account.backendId === agent.backendId).map((one) => one.account.name)}
						onAdded={added}
						onRefused={() => void reload()}
						onCancel={() => setAdding(null)}
					/>
				)}
				{adding === null &&
					signing === null &&
					ADDS.filter(({ section }) => offers(section)).map(({ section, label }) => (
						<button
							key={section}
							type="button"
							className="group-row group-row-add"
							disabled={disabled}
							onClick={() => {
								setFromDesk(null);
								setAdding(section);
							}}
						>
							<PlusIcon />
							{label}
						</button>
					))}
				{pending && (
					<div className="group-row flex-col items-stretch gap-1">
						<p>Switches to {nameOf(wanted)} when its work ends.</p>
						<p className="text-sm text-ink-3">
							What is running now stays on {nameOf(running)}. Stop and switch now stops all of {persona.name}'s work.
						</p>
						<div className="flex justify-end gap-2">
							{keepable && (
								<button type="button" className="control btn-quiet" disabled={disabled || busy} onClick={() => void assign(running)}>
									Keep {nameOf(running)}
								</button>
							)}
							<button type="button" className="control btn" disabled={disabled || busy} onClick={stopAndSwitch}>
								Stop and switch now
							</button>
						</div>
					</div>
				)}
				{held !== undefined && (
					<p className="group-row">
						{held === "signing_in"
							? `${nameOf(wanted)} is still signing in. ${persona.name} keeps ${nameOf(running)} until it is.`
							: `${nameOf(wanted)} is not signed in. ${persona.name} keeps ${nameOf(running)} until it is.`}
					</p>
				)}
			</div>
			{agent !== undefined && (adding !== null || signing !== null) && <p className="group-hint">{NEVER_SEES_SIGN_IN}</p>}
			{hotline && adding !== null && provider !== undefined && (
				<div className="mt-3">
					<ConnectProvider
						key={adding}
						provider={provider}
						kinds={kindsIn(provider, adding)}
						taken={choices.map((one) => one.name)}
						onConnected={(made) => void connected(made)}
						onCancel={connectCancelled}
					/>
				</div>
			)}
			{up && !stranded && <p className="group-hint">{note}</p>}
			{!hotline && onSetup !== undefined && choices.some((one) => one.id === chosen && one.id !== SYSTEM_DEFAULT) && (
				<p className="group-hint">
					<button type="button" className="text-accent-ink hover:underline" onClick={() => onSetup(chosen)}>
						{nameOf(chosen)}'s setup
					</button>{" "}
					· instructions, settings, skills, opened in your editor. It is shared by every teammate on {nameOf(chosen)}.
				</p>
			)}
			{refusal !== null && <Refusal message={refusal} />}
		</section>
	);
}
