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
