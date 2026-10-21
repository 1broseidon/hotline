import type { AccountView } from "../../generated/contract";
import { statusWords, SYSTEM_DEFAULT, whoLine } from "./words";

export function AccountChoices({
	agentName,
	accounts,
	selected,
	inUse,
	signing,
	name,
	disabled,
	onSelect,
}: {
	agentName: string;
	accounts: AccountView[];
	selected: string;
	inUse?: string | undefined;
	signing?: string | undefined;
	name: string;
	disabled: boolean;
	onSelect(accountId: string): void;
}) {
	const choice = (id: string, title: string, detail: string, warn = false) => (
		<label key={id === SYSTEM_DEFAULT ? "system-default" : id} className="group-row group-row-choice">
			<input type="radio" className="radio" name={name} checked={selected === id} disabled={disabled} onChange={() => onSelect(id)} />
			<span className="group-row-text">
				<span className="group-row-title">{title}</span>
				<span className={warn ? "group-row-detail text-warn" : "group-row-detail"} style={{ whiteSpace: "normal" }}>
					{detail}
				</span>
			</span>
			{inUse === id && <span className="shrink-0 text-sm text-ink-3">In use</span>}
		</label>
	);
	return (
		<>
			{choice(SYSTEM_DEFAULT, "System default", `${agentName}'s own sign-in on this computer, as before`)}
			{accounts.map((view) => {
				const status = view.status;
				if (signing === view.account.id) return choice(view.account.id, view.account.name, "Signing in…");
				if (status.state === "signed_in") return choice(view.account.id, view.account.name, whoLine(status) || "Signed in");
				if (status.state === "signed_out") return choice(view.account.id, view.account.name, "Not signed in · it will not start until it is", true);
				return choice(view.account.id, view.account.name, statusWords(status.state));
			})}
		</>
	);
}
