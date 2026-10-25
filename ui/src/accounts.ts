import type { BackendChoice } from "./generated/contract";

/**
 * What an account is called where accounts are listed: its provider's name
 * when the provider has one account, and when it has several, the name the
 * person gave it, after the provider's ("Anthropic · Work"). A name that
 * already starts with the provider's ("Anthropic 2") stands alone.
 */
export function accountName(provider: string, label: string, shared: boolean): string {
	if (!shared) return provider;
	return label.toLowerCase().startsWith(provider.toLowerCase()) ? label : `${provider} · ${label}`;
}

/** A second account's first name: the provider's, numbered. */
export function nextAccountLabel(provider: string, held: number): string {
	return held === 0 ? provider : `${provider} ${held + 1}`;
}

/** `provider#account/model` without its account: the model every account of the provider serves. */
export function plainModel(id: string): string {
	const slash = id.indexOf("/");
	if (slash < 0) return id;
	const provider = id.slice(0, slash);
	const hash = provider.indexOf("#");
	return hash < 0 ? id : `${provider.slice(0, hash)}${id.slice(slash)}`;
}

/** `claude-acp#account` without its account: the agent every account of it runs. */
export function agentOf(backendId: string): string {
	const hash = backendId.indexOf("#");
	return hash <= 0 || hash === backendId.length - 1 ? backendId : backendId.slice(0, hash);
}

/**
 * What an agent is called where agents are listed: its name, and for one of
 * the person's accounts of it the account after it ("Claude Code · Work"),
 * the way a provider's accounts are named.
 */
export function backendName(backend: Pick<BackendChoice, "name" | "account">): string {
	return backend.account === undefined ? backend.name : accountName(backend.name, backend.account.label, backend.account.shared);
}
