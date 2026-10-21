import { useEffect, useState } from "react";
import type { AccountsList, AccountView, Persona, SessionInfo } from "../../generated/contract";
import { PlusIcon } from "../../icons";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { AccountChoices } from "../accounts/AccountChoices";
import { AccountSignIn } from "../accounts/AccountSignIn";
import { AddAccountForm } from "../accounts/AddAccountForm";
import { SYSTEM_DEFAULT } from "../accounts/words";

/** An account this pane added and is signing in now, with the email its sign-in page fills in. */
type Signing = { view: AccountView; email: string | undefined };

/**
 * The "{agent} account" section of a teammate's pane, for an agent with
 * accounts: which account it runs on, picked here, with the account being
 * added and signed in in place. Nothing switches an account on its own,
 * and a switch waits: for the teammate's work to end (Keep or Stop and
 * switch now in the meantime), and for the account to be signed in. An
 * account added here becomes the teammate's only once its sign-in is
 * confirmed.
 *
 * The checked row is the teammate's record, which `persona.account`
 * writes at once. The session says which account it runs on, the one it
 * moves to (`wanted`, sent only while that differs, so absent falls back
 * to the record) and why a move is held. The section is not drawn when the
 * desk lists no accounts for this agent, or refuses to list them.
 */
export function AccountRow({
	persona,
	session,
	disabled,
	onSetup,
}: {
	persona: Persona;
	session: SessionInfo;
	disabled: boolean;
	/** Opens an account's setup in Settings › Agent accounts. */
	onSetup?: ((accountId: string) => void) | undefined;
}) {
	const [list, setList] = useState<AccountsList | null>(null);
	const [reads, setReads] = useState(0);
	const [adding, setAdding] = useState(false);
	const [signing, setSigning] = useState<Signing | null>(null);
	/* An account added from a seat that cannot sign in: the section says where to. */
	const [fromDesk, setFromDesk] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	// biome-ignore lint/correctness/useExhaustiveDependencies: each read is asked for by bumping `reads`.
	useEffect(() => {
		let cancelled = false;
		void wire.command("accounts.list", {}).then(
			(answer) => {
				if (!cancelled) setList(answer);
			},
			() => {
				if (!cancelled) setList(null);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [reads]);
	const reload = () => setReads((was) => was + 1);

	const agent = list?.agents.find((one) => one.backendId === persona.backendId && one.supportsAccounts);
	if (list === null || agent === undefined) return null;
	const accounts = list.accounts.filter((one) => one.account.backendId === agent.backendId);
	const nameOf = (id: string) => (id === SYSTEM_DEFAULT ? "System default" : (accounts.find((one) => one.account.id === id)?.account.name ?? id));

	const chosen = persona.agentAccountId ?? SYSTEM_DEFAULT;
	const running = session.account?.running ?? SYSTEM_DEFAULT;
	const wanted = session.account?.wanted ?? chosen;
	const held = session.account?.held;
	const up = session.state === "ready" || session.state === "thinking";
	const pending = up && held === undefined && wanted !== running;

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
	const pick = (accountId: string) => {
		if (accountId !== chosen) void assign(accountId);
	};
	// The same as Stop, then a start, which comes up on the account chosen (spec §10.4).
	const stopAndSwitch = () =>
		void send(async () => {
			await wire.command("session.stop", { personaId: persona.id });
			await wire.command("session.start", { personaId: persona.id });
		});

	const added = (made: AccountView, email: string | undefined) => {
		setAdding(false);
		setList((was) => was && { ...was, accounts: [...was.accounts.filter((one) => one.account.id !== made.account.id), made] });
		if (made.canSignInHere) setSigning({ view: made, email });
		else setFromDesk(made.account.id);
	};
	const signInEnded = (signedIn: boolean) => {
		const ended = signing;
		setSigning(null);
		if (signedIn && ended !== null) void assign(ended.view.account.id);
		reload();
	};

	const title = `${agent.name} account`;
	const note = session.contextRestored
		? `On ${nameOf(running)} · Restored: ${agent.name} reopened its own session for this account.`
		: `On ${nameOf(running)} · Fresh: a new ${agent.name} session for this account, handed this conversation. Another account's session is never reopened here.`;
	return (
		<section>
			<h3 className="label">{title}</h3>
			<div className="grouped" role="radiogroup" aria-label={title}>
				<AccountChoices
					agentName={agent.name}
					accounts={accounts}
					selected={chosen}
					inUse={up && wanted === running ? running : undefined}
					signing={signing?.view.account.id}
					name={`account-${persona.id}`}
					disabled={disabled || busy}
					onSelect={pick}
				/>
				{signing !== null && (
					<div className="group-row flex-col items-stretch">
						<AccountSignIn account={signing.view} agentName={agent.name} email={signing.email} onEnd={signInEnded} />
					</div>
				)}
				{fromDesk !== null && signing === null && <p className="group-row text-sm text-ink-3">Sign in from the desk itself</p>}
				{adding ? (
					<AddAccountForm
						compact
						backendId={agent.backendId}
						agentName={agent.name}
						takesEmail={agent.takesEmail}
						taken={accounts.map((one) => one.account.name)}
						onAdded={added}
						onCancel={() => setAdding(false)}
					/>
				) : (
					signing === null && (
						<button
							type="button"
							className="group-row group-row-add"
							disabled={disabled}
							onClick={() => {
								setFromDesk(null);
								setAdding(true);
							}}
						>
							<PlusIcon />
							Add account
						</button>
					)
				)}
				{pending && (
					<div className="group-row flex-col items-stretch gap-1">
						<p>Switches to {nameOf(wanted)} when its work ends.</p>
						<p className="text-sm text-ink-3">
							What is running now stays on {nameOf(running)}. Stop and switch now stops all of {persona.name}'s work.
						</p>
						<div className="flex justify-end gap-2">
							<button type="button" className="control btn-quiet" disabled={disabled || busy} onClick={() => void assign(running)}>
								Keep {nameOf(running)}
							</button>
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
			{up && <p className="group-hint">{note}</p>}
			{persona.agentAccountId !== undefined && onSetup !== undefined && (
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
