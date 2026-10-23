import { type ReactNode, useEffect, useState } from "react";
import type { AccountsList, AccountStatusView } from "../../generated/contract";
import { ClockIcon, WarningIcon } from "../../icons";
import { onServer, showPath } from "../../serverFiles";
import { Refusal, refused, type Refused } from "../../ui/Refusal";
import { wire } from "../../wire";
import type { FailureDetails } from "../ErrorCard";
import { AccountSignIn } from "./AccountSignIn";
import { SYSTEM_DEFAULT, THIS_COMPUTER } from "./words";

const NONE: AccountsList = { agents: [], accounts: [], credentials: [] };

// Read from usedBy rather than kept by the card, so a card mounted again after a Continue still knows where the teammate went.
function currentAccount(list: AccountsList, backendId: string, personaId: string): string {
	return list.accounts.find((one) => one.account.backendId === backendId && one.usedBy.includes(personaId))?.account.id ?? SYSTEM_DEFAULT;
}

export function AccountFailure({
	error,
	personaId,
	teammate,
	backendId,
	latest,
	onChooseAccount,
	onRetry,
}: {
	error: FailureDetails;
	personaId: string;
	teammate?: string | undefined;
	backendId: string;
	latest: boolean;
	onChooseAccount?: (() => void) | undefined;
	onRetry?: (() => void) | undefined;
}) {
	const [list, setList] = useState<AccountsList | null>(null);
	const [statuses, setStatuses] = useState<Record<string, AccountStatusView>>({});
	const [movedTo, setMovedTo] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [signing, setSigning] = useState(false);
	const [signedIn, setSignedIn] = useState(false);
	const [reviewed, setReviewed] = useState(false);
	const [refusal, setRefusal] = useState<Refused | null>(null);
	const kind = error.kind;
	const failedOn = error.accountId ?? SYSTEM_DEFAULT;

	useEffect(() => {
		let cancelled = false;
		void wire.command("accounts.list", {}).then(
			(answer) => {
				if (cancelled) return;
				setList(answer);
				// Each status runs the agent's CLI, so only a card that can still move the teammate asks.
				if (kind !== "usage_limit" || !latest || currentAccount(answer, backendId, personaId) !== failedOn) return;
				for (const view of answer.accounts) {
					const id = view.account.id;
					if (view.account.backendId !== backendId || id === failedOn) continue;
					void wire.command("accounts.status", { accountId: id }).then(
						(status) => {
							if (!cancelled) setStatuses((was) => ({ ...was, [id]: status }));
						},
						() => {},
					);
				}
			},
			() => {
				if (!cancelled) setList(NONE);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [kind, backendId, failedOn, latest, personaId]);

	const known = list ?? NONE;
	const agent = known.agents.find((one) => one.backendId === backendId);
	const accounts = known.accounts
		.filter((one) => one.account.backendId === backendId)
		.map((one) => {
			const status = statuses[one.account.id];
			return status === undefined ? one : { ...one, status };
		});
	const failed = accounts.find((one) => one.account.id === error.accountId);
	const nameOf = (id: string) => (id === SYSTEM_DEFAULT ? THIS_COMPUTER : (accounts.find((one) => one.account.id === id)?.account.name ?? id));

	const continueOn = async (accountId: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("persona.account", { personaId, accountId: accountId === SYSTEM_DEFAULT ? null : accountId });
			setMovedTo(accountId);
		} catch (reason) {
			setRefusal(refused("Couldn't change the account", reason));
		} finally {
			setBusy(false);
		}
	};

	const chooseKey = (primary: boolean) =>
		onChooseAccount !== undefined && (
			<button type="button" className={primary ? "control btn btn-sm btn-primary" : "control btn btn-sm"} onClick={onChooseAccount}>
				Choose account
			</button>
		);

	let title = error.title;
	let body = error.summary;
	let keys: ReactNode = null;
	let below: ReactNode = null;

	if (kind === "usage_limit") {
		const current = movedTo ?? currentAccount(known, backendId, personaId);
		const switched = list !== null && current !== failedOn;
		if (failedOn === SYSTEM_DEFAULT) title = `${THIS_COMPUTER} hit its usage limit`;
		else if (failed !== undefined) title = `${failed.account.name} hit its usage limit`;
		if (switched) {
			const session = agent === undefined ? "session" : `${agent.name} session`;
			body = `Now on ${nameOf(current)}: a fresh ${session} that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.`;
		} else if (list !== null) {
			const others = accounts.filter((one) => one.account.id !== failedOn && one.status.state === "signed_in").map((one) => one.account.id);
			const choices = failedOn === SYSTEM_DEFAULT ? others : [...others, SYSTEM_DEFAULT];
			keys = choices.length === 0 ? null : choices.map((id, index) => (
				<button
					key={id === SYSTEM_DEFAULT ? "system-default" : id}
					type="button"
					className={index === 0 ? "control btn btn-sm btn-primary" : "control btn btn-sm"}
					disabled={busy}
					onClick={() => void continueOn(id)}
				>
					Continue on {nameOf(id)}
				</button>
			));
		}
	} else if (kind === "agent_auth") {
		if (failed !== undefined && agent !== undefined) {
			const name = failed.account.name;
			title = `${name} isn't signed in`;
			body = `${teammate ?? "This teammate"} runs on ${name}, and ${agent.name} says nobody is signed in there. Your message is kept; send it again once the account is ready.`;
			if (signedIn) {
				below = (
					<div className="mt-3 text-sm" role="status">
						<p>Signed in. The teammate is ready for another message.</p>
						{onRetry !== undefined && !reviewed && (
							<>
								<p className="mt-1 text-ink-3">The failed turn may have already done some work. Review before sending it again.</p>
								<button
									type="button"
									className="control btn mt-2"
									onClick={() => {
										onRetry();
										setReviewed(true);
									}}
								>
									Review failed message
								</button>
							</>
						)}
					</div>
				);
			} else if (signing) {
				below = (
					<div className="mt-3">
						<AccountSignIn
							account={failed}
							agentName={agent.name}
							onEnd={(done) => {
								setSigning(false);
								setSignedIn(done);
							}}
						/>
					</div>
				);
			}
			keys = (
				<>
					{signedIn || signing ? null : failed.canSignInHere ? (
						<button type="button" className="control btn btn-sm btn-primary" onClick={() => setSigning(true)}>
							Sign in {name}
						</button>
					) : (
						<span className="text-sm text-ink-3">Sign in from the desk itself</span>
					)}
					{chooseKey(false)}
				</>
			);
		} else {
			keys = chooseKey(true);
		}
	} else {
		if (error.code === "unknown_account" && agent !== undefined) title = `Pick a ${agent.name} account`;
		else if (error.code === "folder_unavailable" && failed !== undefined) title = `Can't open ${failed.account.name}'s folder`;
		const folder = failed?.folder;
		const reveal = folder !== undefined && !onServer();
		keys = (
			<>
				{reveal && (
					<button type="button" className="control btn btn-sm" onClick={() => showPath(folder)}>
						Show folder
					</button>
				)}
				{chooseKey(!reveal)}
			</>
		);
	}

	return (
		<>
			<div className={kind === "usage_limit" ? "flex items-center gap-2 text-warn" : "flex items-center gap-2 text-danger"}>
				{kind === "usage_limit" ? <ClockIcon className="shrink-0" /> : <WarningIcon className="shrink-0" />}
				<span className="font-medium">{title}</span>
			</div>
			<p className="mt-1 text-sm text-ink-2">{body}</p>
			{keys !== null && <div className="mt-2 flex flex-wrap items-center gap-2">{keys}</div>}
			{below}
			{refusal !== null && <Refusal {...refusal} />}
		</>
	);
}
