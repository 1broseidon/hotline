import { describe, expect, test } from "bun:test";
import type { AccountSection, AccountsList, AccountStatusView, AccountView, CredentialAccount, CredentialKind } from "../src/generated/contract";
import { accountsFor, detailLine, inUse, suggested } from "../src/components/accounts/fits";

function account(id: string, name: string, status: AccountStatusView): AccountView {
	return { account: { id, backendId: "claude-acp", name, createdAt: 1 }, status, usedBy: [], canSignInHere: true };
}

function credential(id: string, providerId: string, label: string, kind: CredentialKind, section: AccountSection, extra: { revoked?: boolean; default?: boolean } = {}): CredentialAccount {
	return {
		credential: { id, providerId, credentialKind: kind, label, revoked: extra.revoked ?? false, createdAt: 1, updatedAt: 1 },
		section,
		usedBy: [],
		default: extra.default ?? false,
	};
}

const signedIn: AccountStatusView = { state: "signed_in", checkedAt: 1, email: "dev@example.com", plan: "max" };
const signedOut: AccountStatusView = { state: "signed_out", checkedAt: 1 };

function list(accounts: AccountView[]): AccountsList {
	return {
		agents: [
			{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true, providerId: "anthropic" },
			{ backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false },
		],
		accounts,
		credentials: [
			credential("A", "anthropic", "Prod key", "api_key", "api_key", { default: true }),
			credential("gone", "anthropic", "Old key", "api_key", "api_key", { revoked: true }),
			credential("L", "openai-codex", "ChatGPT Pro", "oauth", "subscription", { default: true }),
			credential("B", "anthropic", "Personal key", "api_key", "api_key"),
			credential("X", "xai", "SuperGrok", "oauth", "subscription", { default: true }),
			credential("K1", "xai", "xAI key", "api_key", "api_key"),
			credential("K2", "xai", "xAI key 2", "api_key", "api_key"),
		],
	};
}

const twoAccounts = list([account("w", "Work", signedIn), account("p", "Personal", signedIn)]);
const claude = { backendId: "claude-acp", modelId: undefined };
const ids = (choices: { id: string }[]) => choices.map((one) => one.id);

describe("the accounts that fit a teammate", () => {
	test("Claude Code lists its accounts, then This computer", () => {
		const choices = accountsFor(twoAccounts, claude);
		expect(ids(choices)).toEqual(["w", "p", ""]);
		expect(choices[0]).toMatchObject({ name: "Work", section: "subscription", detail: "dev@example.com · Max", payPerUse: false, signedIn: true });
		expect(choices[2]).toMatchObject({ name: "This computer", section: "subscription", detail: "Claude Code's own sign-in, as before", payPerUse: false });
	});

	test("Hotline Agent lists the live credentials of its model's provider, in stream order", () => {
		const choices = accountsFor(twoAccounts, { backendId: "hotline", modelId: "anthropic/x" });
		expect(ids(choices)).toEqual(["A", "B"]);
		expect(choices[0]).toMatchObject({ name: "Prod key", section: "api_key", detail: "API key", payPerUse: true, isDefault: true });
		expect(choices[1]).toMatchObject({ name: "Personal key", isDefault: false });
	});

	test("a ChatGPT model lists only the ChatGPT sign-in", () => {
		const choices = accountsFor(twoAccounts, { backendId: "hotline", modelId: "openai-codex/y" });
		expect(ids(choices)).toEqual(["L"]);
		expect(choices[0]).toMatchObject({ name: "ChatGPT Pro", section: "subscription", detail: "Subscription", payPerUse: false });
	});

	test("an agent without accounts, or Hotline Agent without a model, has nothing to pick", () => {
		expect(accountsFor(twoAccounts, { backendId: "codex-acp", modelId: undefined })).toEqual([]);
		expect(accountsFor(twoAccounts, { backendId: "hotline", modelId: undefined })).toEqual([]);
	});

	test("a model id with no provider before a slash has nothing to pick", () => {
		expect(accountsFor(twoAccounts, { backendId: "hotline", modelId: "anthropic" })).toEqual([]);
	});

	test("a provider with no credentials has nothing to pick", () => {
		expect(accountsFor(twoAccounts, { backendId: "hotline", modelId: "openai/gpt-5" })).toEqual([]);
	});

	test("Claude Code with no accounts still lists This computer", () => {
		expect(ids(accountsFor(list([]), claude))).toEqual([""]);
	});
});

describe("an account's line", () => {
	test("marks the default only beside another live account of its provider", () => {
		const anthropic = accountsFor(twoAccounts, { backendId: "hotline", modelId: "anthropic/x" });
		expect(anthropic.map(detailLine)).toEqual(["API key · pay per use · Default", "API key · pay per use"]);

		const alone = list([]);
		alone.credentials = alone.credentials.filter((one) => one.credential.id !== "B");
		expect(accountsFor(alone, { backendId: "hotline", modelId: "anthropic/x" }).map(detailLine)).toEqual(["API key · pay per use"]);
		expect(accountsFor(twoAccounts, { backendId: "hotline", modelId: "openai-codex/y" }).map(detailLine)).toEqual(["Subscription"]);
	});
});

describe("the account in use", () => {
	test("is the provider's default when nothing runs yet", () => {
		const choices = accountsFor(twoAccounts, { backendId: "hotline", modelId: "anthropic/x" });
		expect(inUse(choices, undefined)).toBe("A");
		expect(inUse(choices, "B")).toBe("B");
		expect(inUse(choices, "L")).toBe("A");
	});

	test("is This computer for a CLI teammate running on nothing it lists", () => {
		const choices = accountsFor(twoAccounts, claude);
		expect(inUse(choices, undefined)).toBe("");
		expect(inUse(choices, "p")).toBe("p");
	});
});

describe("the suggested account after a failure", () => {
	test("is the other signed-in subscription", () => {
		expect(suggested(accountsFor(twoAccounts, claude), "w")).toBe("p");
	});

	test("is This computer for a CLI teammate with no other signed-in subscription", () => {
		const oneSignedIn = list([account("w", "Work", signedIn), account("p", "Personal", signedOut)]);
		expect(suggested(accountsFor(oneSignedIn, claude), "w")).toBe("");
	});

	test("is the first key when the provider's only subscription failed", () => {
		expect(suggested(accountsFor(twoAccounts, { backendId: "hotline", modelId: "xai/grok" }), "X")).toBe("K1");
	});
});
