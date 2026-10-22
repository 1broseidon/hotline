import { useMemo } from "react";
import type { AccountView } from "../../generated/contract";
import { platform } from "../../native";
import { onServer } from "../../serverFiles";
import { wire } from "../../wire";
import { type SignInJob, SignInTerminal } from "../AgentSignIn";

/** An account's sign-in, as the terminal drives it. The email goes only when there is one. */
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
 * Signing an agent account in, in place under the row that asked. A
 * Windows desk opens the agent's own console window on its screen, so this
 * window says so and keeps only Cancel; elsewhere the agent's sign-in runs
 * in the terminal here. Hotline checks by itself when the sign-in ends, and
 * `onEnd` says whether it did: true only when the agent confirmed it.
 * Mounted only where `canSignInHere`; unmounting cancels the sign-in.
 */
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
