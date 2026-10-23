import type { AccountSection, AccountsList, AccountView, CredentialAccount, CredentialKind, Provider } from "../../generated/contract";
import { ownSignIn, statusWords, SYSTEM_DEFAULT, THIS_COMPUTER, whoLine } from "./words";

const HOTLINE_AGENT = "hotline";

/**
 * One account a teammate may run on. `signedIn` is false for This computer,
 * whose sign-in the window never asks about.
 */
export type AccountChoice = {
	id: string;
	name: string;
	section: AccountSection;
	detail: string;
	payPerUse: boolean;
	isDefault: boolean;
	signedIn: boolean;
};

function agentAccountChoice(view: AccountView): AccountChoice {
	const signedIn = view.status.state === "signed_in";
	const who = signedIn ? whoLine(view.status) : "";
	return {
		id: view.account.id,
		name: view.account.name,
		section: "subscription",
		detail: who !== "" ? who : statusWords(view.status.state),
		payPerUse: false,
		isDefault: false,
		signedIn,
	};
}

function credentialDetail(one: CredentialAccount): string {
	if (one.section === "server") return one.credential.baseUrl ?? "Server";
	if (one.section === "subscription") return "Subscription";
	return one.credential.credentialKind === "oauth" ? "Signed in" : "API key";
}

function credentialChoice(one: CredentialAccount): AccountChoice {
	return {
		id: one.credential.id,
		name: one.credential.label,
		section: one.section,
		detail: credentialDetail(one),
		payPerUse: one.section === "api_key",
		isDefault: one.default,
		signedIn: true,
	};
}

/** Where no section heading says so, a key's own line says it bills per use. */
export function detailLine(choice: AccountChoice): string {
	return choice.payPerUse ? `${choice.detail} · pay per use` : choice.detail;
}

export function providerOf(modelId: string | undefined): string | undefined {
	if (modelId === undefined || !modelId.includes("/")) return undefined;
	return modelId.slice(0, modelId.indexOf("/"));
}

/** The core's own rule (`Vault::section_of`): an OpenRouter sign-in mints a key billed per use. */
function sectionOf(providerId: string, kind: CredentialKind): AccountSection {
	if (kind === "local") return "server";
	if (kind === "oauth" && providerId !== "openrouter") return "subscription";
	return "api_key";
}

/** The ways into a provider that make an account of that section. */
export function kindsIn(provider: Provider | undefined, section: AccountSection): CredentialKind[] {
	if (provider === undefined) return [];
	return provider.credentialKinds.filter((kind) => sectionOf(provider.id, kind) === section);
}

/** The window's copy of the core's rule for which accounts fit a teammate. */
export function accountsFor(list: AccountsList, teammate: { backendId: string; modelId: string | undefined }): AccountChoice[] {
	if (teammate.backendId === HOTLINE_AGENT) return providerAccounts(list, providerOf(teammate.modelId));
	const agent = list.agents.find((one) => one.backendId === teammate.backendId && one.supportsAccounts);
	if (agent === undefined) return [];
	const thisComputer: AccountChoice = {
		id: SYSTEM_DEFAULT,
		name: THIS_COMPUTER,
		section: "subscription",
		detail: ownSignIn(agent.name),
		payPerUse: false,
		isDefault: false,
		signedIn: false,
	};
	const accounts = list.accounts.filter((one) => one.account.backendId === agent.backendId).map(agentAccountChoice);
	return [...accounts, thisComputer];
}

/** What Hotline Agent picks among on a model of `providerId`: that provider's live credentials. */
export function providerAccounts(list: AccountsList, providerId: string | undefined): AccountChoice[] {
	if (providerId === undefined) return [];
	return list.credentials.filter((one) => one.credential.providerId === providerId && !one.credential.revoked).map(credentialChoice);
}

/** Absent a running account, Hotline Agent runs on its provider's default and a CLI agent on This computer. */
export function inUse(choices: AccountChoice[], running: string | undefined): string {
	if (running !== undefined && choices.some((one) => one.id === running)) return running;
	return choices.find((one) => one.isDefault)?.id ?? SYSTEM_DEFAULT;
}

export function suggested(choices: AccountChoice[], failed: string): string | undefined {
	const others = choices.filter((one) => one.id !== failed);
	const subscription = others.find((one) => one.section === "subscription" && one.signedIn);
	if (subscription !== undefined) return subscription.id;
	if (others.some((one) => one.id === SYSTEM_DEFAULT)) return SYSTEM_DEFAULT;
	return others.find((one) => one.section === "api_key")?.id;
}
