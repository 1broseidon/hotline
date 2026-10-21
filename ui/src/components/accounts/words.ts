import type { AccountStatusState, AccountStatusView } from "../../generated/contract";

/**
 * The words an agent account is said in, wherever it shows: Settings, the
 * new-teammate form and the teammate's pane. Nothing here names an agent;
 * the agent's own name comes from the registry and is passed in.
 */

const STATUS_WORDS: Record<AccountStatusState, string> = {
	not_checked: "Not checked",
	checking: "Checking…",
	signed_in: "Signed in",
	signed_out: "Not signed in",
	unavailable: "Status unavailable",
};

export function statusWords(state: AccountStatusState): string {
	return STATUS_WORDS[state];
}

/** Who the account is signed in as: its email and plan. The agent says the plan raw ("max"), so it is capitalised. */
export function whoLine(status: AccountStatusView): string {
	const plan = status.plan === undefined ? "" : status.plan.charAt(0).toUpperCase() + status.plan.slice(1);
	return [status.email ?? "", plan].filter((part) => part !== "").join(" · ");
}

/** What a blank name becomes: the first "Account N", counting on from how many there are, that no account has yet. */
export function nextAccountName(taken: string[]): string {
	const names = new Set(taken.map((name) => name.toLowerCase()));
	let number = taken.length + 1;
	while (names.has(`account ${number}`)) number++;
	return `Account ${number}`;
}

/** Names in a sentence: "Ada", "Ada and Mack", "Ada, Mack and Rosa". */
export function namesList(names: string[]): string {
	if (names.length < 2) return names.join("");
	return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}

/**
 * The kinds of setup an account folder keeps, in the order a choice of
 * them lists them: `choice` is the name a checkbox says, `item` the name of
 * its row in an account's setup, `noun` what its count counts, and `detail`
 * what it holds. The core answers ids only, so the names live here.
 */
export const SETUP_CATEGORIES: readonly { id: string; choice: string; item: string; noun?: string; detail: string }[] = [
	{ id: "instructions", choice: "Instructions and rules", item: "Instructions", detail: "CLAUDE.md, rules/" },
	{ id: "skills", choice: "Skills, agents and commands", item: "Skills", noun: "skill", detail: "skills/, agents/, commands/" },
	{ id: "plugins", choice: "Plugins", item: "Plugins", noun: "plugin", detail: "plugins/ and their settings" },
	{ id: "mcp", choice: "MCP servers", item: "MCP servers", noun: "server", detail: "user-scope servers" },
	{ id: "settings", choice: "Settings", item: "Settings", detail: "permissions, hooks, model, statusline — without auth keys" },
];
