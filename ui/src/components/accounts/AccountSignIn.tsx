import { useMemo } from "react";
import type { AccountsList, AccountView } from "../../generated/contract";
import { platform } from "../../native";
import { onServer } from "../../serverFiles";
import { wire } from "../../wire";
import { type SignInJob, SignInTerminal } from "../AgentSignIn";

export function accountJob(accountId: string, email?: string): SignInJob {
	return {
		start: () => wire.command("accounts.login.start", email === undefined ? { accountId } : { accountId, email }),
		poll: (id) => wire.command("accounts.login.poll", { accountId, id }),
		input: async (id, input) => {
			await wire.command("accounts.login.input", { accountId, id, input });
		},
		cancel: async (id) => {
			await wire.command("accounts.login.cancel", { accountId, id });
		},
	};
}

/**
 * The core answers a cancel before its attempt ends, then checks the account
 * again. Run it from an effect, so it starts after the closed sign-in's
 * cleanup has sent the cancel.
 */
export async function settledAccounts(accountId: string, alive: () => boolean): Promise<AccountsList | null> {
	const viewIn = (list: AccountsList) => list.accounts.find((one) => one.account.id === accountId);
	let list = await wire.command("accounts.list", {});
	while (alive() && viewIn(list)?.attempt?.state === "running") {
		await new Promise((resolve) => setTimeout(resolve, 200));
		if (!alive()) return null;
		list = await wire.command("accounts.list", {});
	}
	if (alive() && viewIn(list)?.status.state === "checking") {
		await wire.command("accounts.status", { accountId }).catch(() => {});
		if (alive()) list = await wire.command("accounts.list", {});
	}
	return alive() ? list : null;
}

/** Mounted only where `canSignInHere`; unmounting cancels the sign-in. */
export function AccountSignIn({
	account,
	agentName,
	email,
	cancelLabel = "Cancel",
	onEnd,
}: {
	account: AccountView;
	agentName: string;
	email?: string | undefined;
	cancelLabel?: string;
	onEnd(signedIn: boolean): void;
}) {
	const accountId = account.account.id;
	const job = useMemo(() => accountJob(accountId, email), [accountId, email]);
	const windowNote =
		!onServer() && platform() === "windows"
			? `A ${agentName} window opened for ${account.account.name}. Finish signing in there; Hotline checks when it closes.`
			: undefined;
	return (
		<div className="flex flex-col gap-2">
			<SignInTerminal job={job} windowNote={windowNote} cancelLabel={cancelLabel} onDone={() => onEnd(true)} onClose={() => onEnd(false)} />
			<p className="hint">
				The browser uses whatever sign-in is already open there: use Switch account on the page, or open the link from the {agentName} window
				in the right browser profile.
			</p>
		</div>
	);
}
