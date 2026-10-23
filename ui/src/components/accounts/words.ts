import type { AccountStatusState, AccountStatusView, RosterEntry } from "../../generated/contract";

/** Nothing here names an agent; the agent's own name comes from the registry and is passed in. */

export const SYSTEM_DEFAULT = "";
export const THIS_COMPUTER = "This computer";

export const NEVER_SEES_SIGN_IN = "Hotline never sees a password or token: each agent's CLI keeps its own sign-in.";

export function ownSignIn(agentName: string): string {
	return `${agentName}'s own sign-in, as before`;
}

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

/** The agent says the plan raw ("max"), so it is capitalised. */
export function whoLine(status: AccountStatusView): string {
	const plan = status.plan === undefined ? "" : status.plan.charAt(0).toUpperCase() + status.plan.slice(1);
	return [status.email ?? "", plan].filter((part) => part !== "").join(" · ");
}

export function nextAccountName(taken: string[]): string {
	const names = new Set(taken.map((name) => name.toLowerCase()));
	let number = taken.length + 1;
	while (names.has(`account ${number}`)) number++;
	return `Account ${number}`;
}

/** The core's own rule for a blank credential name, so the field shows the name it will take. */
export function freeName(base: string, taken: string[]): string {
	const names = new Set(taken.map((name) => name.toLowerCase()));
	let name = base;
	for (let number = 2; names.has(name.toLowerCase()); number++) name = `${base} ${number}`;
	return name;
}

/** The core answers teammate ids; a teammate the roster no longer has keeps its id. */
export function namesOf(ids: string[], roster: RosterEntry[]): string[] {
	return ids.map((id) => roster.find((entry) => entry.persona.id === id)?.persona.name ?? id);
}

export function usedByWords(names: string[]): string {
	return names.length === 0 ? "Not used yet" : `Used by ${names.join(", ")}`;
}

export function namesList(names: string[]): string {
	if (names.length < 2) return names.join("");
	return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}

/**
 * The core answers ids only, so the names live here, and they are every
 * agent's: which files a kind is made of belongs to the agent's support row,
 * never to the window.
 */
export const SETUP_CATEGORIES: readonly { id: string; choice: string; item: string; noun?: string }[] = [
	{ id: "instructions", choice: "Instructions and rules", item: "Instructions" },
	{ id: "skills", choice: "Skills, agents and commands", item: "Skills", noun: "skill" },
	{ id: "plugins", choice: "Plugins", item: "Plugins", noun: "plugin" },
	{ id: "mcp", choice: "MCP servers", item: "MCP servers", noun: "server" },
	{ id: "settings", choice: "Settings", item: "Settings" },
];
