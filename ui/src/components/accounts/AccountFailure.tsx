import { type ReactNode, useEffect, useLayoutEffect, useState } from "react";
import type { AccountsList, AccountStatusView } from "../../generated/contract";
import { ClockIcon, WarningIcon } from "../../icons";
import { onServer, showPath } from "../../serverFiles";
import { Refusal, type Refused } from "../../ui/Refusal";
import { wire } from "../../wire";
import type { FailureDetails } from "../ErrorCard";
import { AccountPicker } from "./AccountPicker";
import { AccountSignIn } from "./AccountSignIn";
import { accountsFor, HOTLINE_AGENT, providerAccounts, suggested } from "./fits";
import { SYSTEM_DEFAULT, THIS_COMPUTER } from "./words";

const NONE: AccountsList = { agents: [], accounts: [], credentials: [] };

/** Whether the card offers another account, or the teammate has already moved off the one that failed. */
export type AccountOffer = "none" | "open" | "moved";

// Read from usedBy rather than kept by the card, so a card mounted again after a Continue still knows where the teammate went.
// Nothing when the list cannot say: a Hotline Agent teammate off the failed account's provider, or an agent the list lacks.
function currentAccount(list: AccountsList, backendId: string, providerId: string | undefined, personaId: string): string | undefined {
	if (backendId === HOTLINE_AGENT) {
		return list.credentials.find((one) => one.credential.providerId === providerId && one.usedBy.includes(personaId))?.credential.id;
	}
	if (!list.agents.some((one) => one.backendId === backendId)) return undefined;
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
	onOffer,
}: {
	error: FailureDetails;
	personaId: string;
	teammate?: string | undefined;
	backendId: string;
	latest: boolean;
	onChooseAccount?: (() => void) | undefined;
	onRetry?: (() => void) | undefined;
	onOffer?: ((offer: AccountOffer) => void) | undefined;
}) {
	const [list, setList] = useState<AccountsList | null>(null);
	const [statuses, setStatuses] = useState<Record<string, AccountStatusView>>({});
	const [probing, setProbing] = useState(0);
	const [movedTo, setMovedTo] = useState<string | null>(null);
	const [picked, setPicked] = useState<string | null>(null);
	const [declined, setDeclined] = useState(false);
	const [busy, setBusy] = useState(false);
	const [signing, setSigning] = useState(false);
	const [signedIn, setSignedIn] = useState(false);
	const [reviewed, setReviewed] = useState(false);
	const [refusal, setRefusal] = useState<Refused | null>(null);
	const kind = error.kind;
	const failedOn = error.accountId ?? SYSTEM_DEFAULT;
	const hotline = backendId === HOTLINE_AGENT;
	// A CLI agent's spent credit is its own card, as it always was.
	const limit = kind === "usage_limit" || (hotline && kind === "quota");

	useEffect(() => {
		let cancelled = false;
		void wire.command("accounts.list", {}).then(
			(answer) => {
				if (cancelled) return;
				setList(answer);
				// Each status runs the agent's CLI, so only a card that can still move the teammate asks.
				const asks = !hotline && limit && latest && currentAccount(answer, backendId, undefined, personaId) === failedOn;
				const others = asks ? answer.accounts.filter((view) => view.account.backendId === backendId && view.account.id !== failedOn) : [];
				setProbing(others.length);
				for (const view of others) {
					const id = view.account.id;
					void wire
						.command("accounts.status", { accountId: id })
						.then(
							(status) => {
								if (!cancelled) setStatuses((was) => ({ ...was, [id]: status }));
							},
							() => {},
						)
						.finally(() => {
							if (!cancelled) setProbing((left) => left - 1);
						});
				}
			},
			() => {
				if (cancelled) return;
				setList(NONE);
				// An earlier read's checks were dropped with it, and none of their answers is coming.
				setProbing(0);
			},
		);
		return () => {
			cancelled = true;
		};
	}, [hotline, limit, backendId, failedOn, latest, personaId]);

	const known = list ?? NONE;
	const fresh: AccountsList = {
		...known,
		accounts: known.accounts.map((one) => {
			const status = statuses[one.account.id];
			return status === undefined ? one : { ...one, status };
		}),
	};
	const agent = known.agents.find((one) => one.backendId === backendId);
	const failed = fresh.accounts.find((one) => one.account.backendId === backendId && one.account.id === error.accountId);
	const failedCredential = hotline ? known.credentials.find((one) => one.credential.id === error.accountId)?.credential : undefined;
	const choices = hotline ? providerAccounts(known, failedCredential?.providerId) : accountsFor(fresh, { backendId, modelId: undefined });

	const current = list === null ? undefined : (movedTo ?? currentAccount(known, backendId, failedCredential?.providerId, personaId));
	// Only an account that fits has a name to say; the card never says an id.
	const nowOn = limit && current !== failedOn ? choices.find((one) => one.id === current) : undefined;
	// Until every status is back, This computer would be suggested over a signed-in account, and Try again would run the spent one.
	const deciding = limit && latest && !declined && (list === null || probing > 0);
	// A CLI account is offered once its agent says it is signed in; This computer's own sign-in is never asked about.
	const others = choices.filter((one) => one.id !== failedOn && (hotline || one.signedIn || one.id === SYSTEM_DEFAULT));
	const selected = picked !== null && others.some((one) => one.id === picked) ? picked : (suggested(others, failedOn) ?? others[0]?.id);
	const open = limit && latest && !declined && !deciding && current === failedOn && selected !== undefined;
	const offer: AccountOffer = deciding || open ? "open" : nowOn !== undefined ? "moved" : "none";

	// Before the first paint, so Try again never shows on a card that is about to offer another account.
	useLayoutEffect(() => {
		onOffer?.(offer);
	}, [offer, onOffer]);

	const continueOn = async (accountId: string) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		// On Hotline Agent the default's row means following the default, as it does in the teammate's pane.
		const followsDefault = hotline && choices.some((one) => one.id === accountId && one.isDefault);
		try {
			await wire.command("persona.account", { personaId, accountId: accountId === SYSTEM_DEFAULT || followsDefault ? null : accountId });
			setMovedTo(accountId);
		} catch (reason) {
			// The core refuses a switch in a sentence written for the person, as the teammate's pane shows it.
			setRefusal({ message: reason instanceof Error ? reason.message : String(reason) });
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

	if (limit) {
		if (kind === "usage_limit") {
			if (hotline) {
				if (failedCredential !== undefined) title = `${failedCredential.label} hit its usage limit`;
			} else if (failedOn === SYSTEM_DEFAULT) title = `${THIS_COMPUTER} hit its usage limit`;
			else if (failed !== undefined) title = `${failed.account.name} hit its usage limit`;
		}
		if (nowOn !== undefined) {
			const session = agent === undefined ? "session" : `${agent.name} session`;
			body = `Now on ${nowOn.name}: a fresh ${session} that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.`;
		} else if (deciding && list !== null) {
			keys = (
				<>
					<span className="text-sm text-ink-3">Checking the other accounts…</span>
					<button type="button" className="control btn-quiet btn-sm" onClick={() => setDeclined(true)}>
						Try again later
					</button>
				</>
			);
		} else if (open) {
			keys = (
				<>
					<span className="text-sm text-ink-2">Continue on</span>
					<div className="w-64 min-w-0 max-w-full">
						<AccountPicker choices={others} selected={selected} label="Continue on" disabled={busy} onSelect={setPicked} />
					</div>
					<button type="button" className="control btn btn-sm btn-primary" disabled={busy} onClick={() => void continueOn(selected)}>
						Continue
					</button>
					<button type="button" className="control btn-quiet btn-sm" disabled={busy} onClick={() => setDeclined(true)}>
						Try again later
					</button>
				</>
			);
		}
	} else if (kind === "auth") {
		keys = chooseKey(true);
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
