import { afterEach, beforeEach, describe, expect, setSystemTime, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountStatusView, AccountView, CredentialAccount, CredentialKind } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {} }) } });
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const { ErrorCard, resetWords } = await import("../src/components/ErrorCard");

const agents = [{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true }];
function account(id: string, name: string, extra: Partial<AccountView> = {}): AccountView {
	return {
		account: { id, backendId: "claude-acp", name, createdAt: 1 },
		status: { state: "signed_out", checkedAt: 1 },
		usedBy: [],
		folder: `/data/agent-accounts/${id}`,
		canSignInHere: true,
		...extra,
	};
}
const signedIn = (id: string, name: string, usedBy: string[] = []) =>
	account(id, name, { status: { state: "signed_in", checkedAt: 1, email: `${id}@example.com`, plan: "max" }, usedBy });
const a1 = (usedBy = ["ada"]) => account("a1", "Account 1", { status: { state: "signed_in", checkedAt: 1, email: "dev@example.com", plan: "max" }, usedBy });
const a2 = () => account("a2", "Account 2", { status: { state: "signed_in", checkedAt: 1, email: "two@example.com", plan: "pro" } });
const a3 = () => account("a3", "Account 3");
function credential(id: string, label: string, kind: CredentialKind, extra: Partial<CredentialAccount> = {}, providerId = "xai"): CredentialAccount {
	return {
		credential: { id, providerId, credentialKind: kind, label, revoked: false, createdAt: 1, updatedAt: 1 },
		section: kind === "oauth" ? "subscription" : "api_key",
		usedBy: [],
		default: false,
		...extra,
	};
}
const local = { id: "local", name: "This computer", kind: "local" as const, origin: "", token: "" };
const remote = { id: "server-1", name: "Studio", kind: "remote" as const, origin: "", token: "" };

function notice(failure: Record<string, unknown>): string {
	const full = { title: "Turn failed", summary: "It did not finish.", details: "diagnostic", phase: "acp", ...failure };
	return `${full.title}: ${full.summary}\n\n${JSON.stringify({ hotlineFailure: full })}`;
}
const limit = (extra: Record<string, unknown> = {}) =>
	notice({ kind: "usage_limit", title: "Claude Code usage limit reached", summary: "You've hit your limit · resets 5pm", ...extra });
const quota = (extra: Record<string, unknown> = {}) =>
	notice({ kind: "quota", title: "Provider quota exhausted", summary: "Check your provider balance or usage limit before continuing.", phase: "turn", ...extra });
const chatgptLimit = (extra: Record<string, unknown> = {}) =>
	notice({
		kind: "usage_limit",
		title: "ChatGPT usage limit reached",
		summary: "This ChatGPT sign-in reached its usage limit.",
		status: 429,
		code: "usage_limit_reached",
		phase: "turn",
		...extra,
	});
const signIn = { harnessName: "Claude Code", methods: [{ id: "claude-ai", name: "Claude account" }, { id: "console", name: "Anthropic Console" }] };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let credentials: CredentialAccount[];
let fresh: Map<string, AccountStatusView>;
let statusGate: Promise<void> | null;
let accountRefusal: string | null;
let poll: { state: "running" | "succeeded" | "failed"; output: string };
let chosen: number;
let retried: number;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function respond(cmd: string, params: Record<string, unknown>): unknown {
	if (cmd === "accounts.list") return { agents, accounts, credentials };
	if (cmd === "accounts.status") return fresh.get(params["accountId"] as string) ?? accounts.find((one) => one.account.id === params["accountId"])?.status;
	if (cmd === "persona.account") return { running: params["accountId"] ?? undefined };
	if (cmd === "accounts.login.start") return { id: "l1" };
	if (cmd === "accounts.login.poll") return poll;
	if (cmd === "accounts.login.cancel") return null;
	throw new Error(`Unexpected command ${cmd}`);
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	replaceDesks([local]);
	calls = [];
	accounts = [a1(), a2(), a3()];
	credentials = [];
	fresh = new Map();
	statusGate = null;
	accountRefusal = null;
	poll = { state: "running", output: "" };
	chosen = 0;
	retried = 0;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push([cmd, params]);
		if (cmd === "accounts.status" && statusGate !== null) await statusGate;
		if (cmd === "persona.account" && accountRefusal !== null) throw new Error(accountRefusal);
		return respond(cmd, params);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await unmount();
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	setSystemTime();
	await teardown();
});

type Mounting = { latest?: boolean; backendId?: string; at?: number };
async function mount(text: string, { latest = true, backendId = "claude-acp", at }: Mounting = {}) {
	await act(async () => {
		root!.render(
			<ErrorCard
				text={text}
				personaId="ada"
				teammate="Ada"
				backendId={backendId}
				onChooseAccount={() => chosen++}
				onRetry={() => retried++}
				{...(latest ? { onRunAgain: async () => null } : {})}
				{...(at !== undefined ? { at } : {})}
			/>,
		);
	});
	await settle();
}
async function unmount() {
	await act(async () => {
		root?.unmount();
		root = null;
	});
}
async function remount(text: string, mounting: Mounting = {}) {
	await unmount();
	await act(async () => {
		root = (await import("react-dom/client")).createRoot(container);
	});
	await mount(text, mounting);
}
async function settle() {
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const buttons = () => [...container.querySelectorAll("button")].map((node) => node.textContent);
function button(label: string): HTMLButtonElement {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
async function click(label: string) {
	const found = button(label);
	await act(async () => {
		found.click();
	});
	await settle();
}
const picker = () => container.querySelector<HTMLButtonElement>('[aria-label="Continue on"]');
const listbox = () => document.querySelector<HTMLElement>('[role="listbox"]');
const options = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')];
const nameOf = (option: HTMLElement) => option.querySelector(".menu-item-text")?.firstChild?.textContent;
const optionNames = () => options().map(nameOf);
async function openPicker() {
	await act(async () => picker()!.click());
}
async function pick(name: string) {
	const option = options().find((one) => nameOf(one) === name);
	if (!option) throw new Error(`Missing option ${name}: ${optionNames().join(", ")}`);
	await act(async () => option.click());
}

describe("a usage limit's offer", () => {
	test("Claude Code offers every other signed-in account in one picker, starting on the first subscription", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal"), signedIn("agency", "Agency"), account("idle", "Idle")];
		await mount(limit({ accountId: "work" }));
		expect(text()).toContain("Work hit its usage limit");
		expect(text()).toContain("You've hit your limit · resets 5pm");
		expect(calls).not.toContainEqual(["accounts.status", { accountId: "work" }]);
		expect(calls).toContainEqual(["accounts.status", { accountId: "personal" }]);
		expect(text()).toContain("Continue on");
		expect(picker()!.textContent).toContain("Personal");
		expect(buttons()).toContain("Continue");
		expect(buttons()).toContain("Try again later");
		expect(buttons()).not.toContain("Try again");

		await openPicker();
		expect(optionNames()).toEqual(["Personal", "Agency", "This computer"]);
		await pick("Agency");
		expect(picker()!.textContent).toContain("Agency");
		await openPicker();
		await pick("Personal");

		await click("Continue");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "personal" }]);
		expect(text()).toContain(
			"Now on Personal: a fresh Claude Code session that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.",
		);
		expect(buttons()).toContain("Try again");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Try again later");
	});

	test("a status that comes back signed out leaves the picker, and only fresh signed-in accounts are offered", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal"), signedIn("agency", "Agency")];
		fresh.set("personal", { state: "signed_out", checkedAt: 2 });
		await mount(limit({ accountId: "work" }));
		expect(picker()!.textContent).toContain("Agency");
		await openPicker();
		expect(optionNames()).toEqual(["Agency", "This computer"]);
	});

	test("Try again later closes the offer and sends nothing, and Try again comes back", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal")];
		await mount(limit({ accountId: "work" }));
		expect(picker()).not.toBeNull();
		expect(buttons()).not.toContain("Try again");
		await click("Try again later");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Continue");
		expect(buttons()).toContain("Try again");
		expect(text()).toContain("Work hit its usage limit");
		expect(calls.filter(([cmd]) => cmd === "persona.account")).toEqual([]);
	});

	test("thirty accounts get a search, and opening the picker never grows the card", async () => {
		const many = Array.from({ length: 29 }, (_, at) => signedIn(`p${at}`, `Plan ${at}`));
		accounts = [signedIn("work", "Work", ["ada"]), ...many];
		await mount(limit({ accountId: "work" }));
		const card = container.querySelector("section")!;
		const before = [...card.querySelectorAll("*")];
		await openPicker();
		const search = document.querySelector<HTMLInputElement>('input[type="search"]');
		expect(search).not.toBeNull();
		expect(search!.placeholder).toBe("Search 30 accounts");
		// happy-dom lays nothing out, so no height can be read: the menu floats on the body, and the card keeps every element it had.
		const menu = listbox()!.closest<HTMLElement>(".account-picker-menu")!;
		expect(menu.parentElement).toBe(document.body);
		expect(card.contains(menu)).toBe(false);
		const after = [...card.querySelectorAll("*")];
		expect(after).toHaveLength(before.length);
		for (const [at, element] of after.entries()) expect(element).toBe(before[at]!);
	});

	test("Hotline Agent on xAI suggests the subscription after a spent key, and a key says it bills per use", async () => {
		credentials = [
			credential("key-a", "Key A", "api_key", { usedBy: ["ada"], default: true }),
			credential("sub-s", "SuperGrok", "oauth"),
			credential("key-b", "Key B", "api_key"),
		];
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		expect(text()).toContain("Provider quota exhausted");
		expect(calls.filter(([cmd]) => cmd === "accounts.status")).toEqual([]);
		expect(picker()!.textContent).toContain("SuperGrok");
		expect(buttons()).not.toContain("Try again");

		await openPicker();
		expect(optionNames()).toEqual(["SuperGrok", "Key B"]);
		const keys = [...document.querySelectorAll<HTMLElement>('[role="group"]')].find((group) => group.textContent?.startsWith("API keys · pay per use"));
		expect(keys).toBeDefined();
		expect([...keys!.querySelectorAll<HTMLElement>('[role="option"]')].map(nameOf)).toEqual(["Key B"]);
		await pick("Key B");
		expect(picker()!.textContent).toContain("Key B · API key · pay per use");

		await click("Continue");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "key-b" }]);
		expect(text()).toContain(
			"Now on Key B: a fresh session that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.",
		);
		expect(buttons()).toContain("Try again");
	});

	test("Hotline Agent continues on its provider's default by following it, never pinning it", async () => {
		credentials = [
			credential("sub-s", "SuperGrok", "oauth", { default: true }),
			credential("key-a", "Key A", "api_key", { usedBy: ["ada"] }),
		];
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		expect(picker()!.textContent).toContain("SuperGrok");
		await click("Continue");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: null }]);
		expect(text()).toContain("Now on SuperGrok: a fresh session");
	});

	test("a ChatGPT limit names the sign-in that hit it and offers the provider's other one", async () => {
		credentials = [
			credential("plus", "Plus", "oauth", { usedBy: ["ada"], default: true }, "openai-codex"),
			credential("pro", "Pro", "oauth", {}, "openai-codex"),
			credential("grok", "SuperGrok", "oauth"),
		];
		await mount(chatgptLimit({ accountId: "plus" }), { backendId: "hotline" });
		expect(text()).toContain("Plus hit its usage limit");
		await openPicker();
		expect(optionNames()).toEqual(["Pro"]);
	});

	test("a spent quota with no other account is the card it was: no picker and no Try again", async () => {
		credentials = [credential("key-a", "Key A", "api_key", { usedBy: ["ada"], default: true })];
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		expect(text()).toContain("Provider quota exhausted");
		expect(text()).toContain("Check your provider balance or usage limit before continuing.");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Continue");
		expect(buttons()).not.toContain("Try again");
	});

	test("a usage limit on This computer with no account is the card it was, with Try again", async () => {
		accounts = [];
		await mount(limit());
		expect(text()).toContain("This computer hit its usage limit");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Continue");
		expect(buttons()).toContain("Try again");
	});

	test("a Hotline Agent usage limit with no account named offers no picker and keeps its own title", async () => {
		credentials = [credential("plus", "Plus", "oauth", { usedBy: ["ada"], default: true }, "openai-codex")];
		await mount(chatgptLimit(), { backendId: "hotline" });
		expect(text()).toContain("ChatGPT usage limit reached");
		expect(text()).not.toContain("This computer");
		expect(picker()).toBeNull();
		expect(buttons()).toContain("Try again");
	});

	test("only the latest failure offers another account", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal")];
		await mount(limit({ accountId: "work" }), { latest: false });
		expect(text()).toContain("Work hit its usage limit");
		expect(picker()).toBeNull();
	});

	test("nothing is offered, and Try again stays hidden, until every other account's status is back", async () => {
		const stale = { state: "not_checked" as const };
		accounts = [signedIn("work", "Work", ["ada"]), account("personal", "Personal", { status: stale }), account("agency", "Agency", { status: stale })];
		fresh.set("personal", { state: "signed_in", checkedAt: 2, email: "personal@example.com" });
		let release = () => {};
		statusGate = new Promise((resolve) => {
			release = resolve;
		});
		await mount(limit({ accountId: "work" }));
		expect(calls.filter(([cmd]) => cmd === "accounts.status")).toHaveLength(2);
		expect(buttons()).not.toContain("Try again");
		expect(buttons()).not.toContain("Continue");
		expect(picker()).toBeNull();
		expect(text()).toContain("Checking the other accounts…");

		release();
		await settle();
		expect(text()).not.toContain("Checking the other accounts…");
		expect(picker()!.textContent).toContain("Personal");
		expect(buttons()).toContain("Continue");
		expect(buttons()).not.toContain("Try again");
	});

	test("Try again later while the other accounts are checked closes the offer", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), account("personal", "Personal", { status: { state: "not_checked" } })];
		fresh.set("personal", { state: "signed_in", checkedAt: 2, email: "personal@example.com" });
		let release = () => {};
		statusGate = new Promise((resolve) => {
			release = resolve;
		});
		await mount(limit({ accountId: "work" }));
		expect(text()).toContain("Checking the other accounts…");
		await click("Try again later");
		expect(text()).not.toContain("Checking the other accounts…");
		expect(buttons()).toContain("Try again");

		release();
		await settle();
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Continue");
		expect(calls.filter(([cmd]) => cmd === "persona.account")).toEqual([]);
	});

	test("a list that cannot be read leaves no card checking the other accounts", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal"), signedIn("agency", "Agency")];
		statusGate = new Promise(() => {});
		await mount(limit({ accountId: "work" }));
		expect(text()).toContain("Checking the other accounts…");

		// Read again for another account, and this time the list does not come back.
		const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
			calls.push([cmd, params]);
			if (cmd === "accounts.list") throw new Error("The desk did not answer.");
			return respond(cmd, params);
		}) as typeof wire.command);
		restores.push(() => command.mockRestore());
		await mount(limit({ accountId: "personal" }));
		expect(text()).not.toContain("Checking the other accounts…");
		expect(buttons()).toContain("Try again");
	});

	test("a status that fails to answer still settles the offer", async () => {
		accounts = [signedIn("work", "Work", ["ada"]), signedIn("personal", "Personal")];
		const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
			calls.push([cmd, params]);
			if (cmd === "accounts.status") throw new Error("Claude Code did not answer.");
			return respond(cmd, params);
		}) as typeof wire.command);
		restores.push(() => command.mockRestore());
		await mount(limit({ accountId: "work" }));
		expect(text()).not.toContain("Checking the other accounts…");
		expect(picker()!.textContent).toContain("Personal");
	});

	test("on This computer the signed-in accounts are offered, and This computer is not", async () => {
		accounts = [a1([]), a2(), a3()];
		await mount(limit());
		expect(text()).toContain("This computer hit its usage limit");
		expect(picker()!.textContent).toContain("Account 1");
		await openPicker();
		expect(optionNames()).toEqual(["Account 1", "Account 2"]);
		await pick("Account 2");
		await click("Continue");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a2" }]);
		expect(text()).toContain("Now on Account 2: a fresh Claude Code session");
	});

	test("a CLI agent's spent credit is the card it always was", async () => {
		await mount(quota({ accountId: "a1" }));
		expect(text()).toContain("Provider quota exhausted");
		expect(text()).toContain("Check your provider balance or usage limit before continuing.");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Choose account");
		expect(buttons()).not.toContain("Try again");
		expect(calls).toEqual([]);
	});

	test("a Hotline Agent quota put off with Try again later never offers Try again", async () => {
		credentials = [credential("key-a", "Key A", "api_key", { usedBy: ["ada"], default: true }), credential("key-b", "Key B", "api_key")];
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		expect(buttons()).not.toContain("Try again");
		await click("Try again later");
		expect(picker()).toBeNull();
		expect(buttons()).not.toContain("Try again");
		expect(text()).toContain("Provider quota exhausted");
	});

	test("a refused switch says the core's sentence and keeps the offer", async () => {
		credentials = [credential("key-a", "Key A", "api_key", { usedBy: ["ada"], default: true }), credential("key-b", "Key B", "api_key")];
		accountRefusal = "Key B is signed out. Sign it in again in Settings › Providers.";
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		await click("Continue");
		expect(text()).toContain("Key B is signed out. Sign it in again in Settings › Providers.");
		expect(picker()).not.toBeNull();
		expect(button("Continue").disabled).toBe(false);
		expect(buttons()).not.toContain("Try again");

		accountRefusal = null;
		await click("Continue");
		expect(text()).toContain("Now on Key B");
		expect(text()).not.toContain("Key B is signed out.");
	});

	test("a teammate now on an account that does not fit is never named by its id", async () => {
		const gone = credential("gone", "Old key", "api_key", { usedBy: ["ada"] });
		credentials = [credential("key-a", "Key A", "api_key", { default: true }), { ...gone, credential: { ...gone.credential, revoked: true } }, credential("key-b", "Key B", "api_key")];
		await mount(quota({ accountId: "key-a" }), { backendId: "hotline" });
		expect(text()).not.toContain("Now on");
		expect(text()).not.toContain("gone");
		expect(text()).toContain("Check your provider balance or usage limit before continuing.");
		expect(picker()).toBeNull();
	});

	test("a signed-out chosen credential offers Choose account", async () => {
		credentials = [credential("key-a", "Key A", "api_key", { usedBy: ["ada"] }), credential("key-b", "Key B", "api_key", { default: true })];
		await mount(
			notice({
				kind: "auth",
				title: "Provider sign-in required",
				summary: "The account this teammate is set to is signed out or gone. Choose another account.",
				accountId: "key-a",
				phase: "turn",
			}),
			{ backendId: "hotline" },
		);
		expect(text()).toContain("Provider sign-in required");
		expect(text()).toContain("The account this teammate is set to is signed out or gone. Choose another account.");
		expect(buttons()).toContain("Choose account");
		await click("Choose account");
		expect(chosen).toBe(1);
	});
});

describe("a limit's reset time", () => {
	const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
	const shortDate = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
	// Local dates on both sides, so the zone the tests run in never moves a day.
	const now = new Date(2026, 9, 23, 14, 0).getTime();
	const at = (day: number, hour: number, minute: number) => new Date(2026, 9, day, hour, minute);

	test("is the local clock time today, tomorrow with its day word, and a short date after", () => {
		expect(resetWords(at(23, 17, 40).getTime(), now)).toBe(`Resets at ${clock.format(at(23, 17, 40))}.`);
		expect(resetWords(at(23, 23, 59).getTime(), now)).toBe(`Resets at ${clock.format(at(23, 23, 59))}.`);
		expect(resetWords(at(24, 0, 10).getTime(), now)).toBe(`Resets tomorrow at ${clock.format(at(24, 0, 10))}.`);
		expect(resetWords(at(24, 9, 10).getTime(), now)).toBe(`Resets tomorrow at ${clock.format(at(24, 9, 10))}.`);
		expect(resetWords(at(25, 0, 0).getTime(), now)).toBe(`Resets ${shortDate.format(at(25, 0, 0))} at ${clock.format(at(25, 0, 0))}.`);
		expect(resetWords(at(30, 9, 10).getTime(), now)).toBe(`Resets ${shortDate.format(at(30, 9, 10))} at ${clock.format(at(30, 9, 10))}.`);
	});

	test("a limit's card says when it resets from its notice's time, and a short wait keeps its seconds", async () => {
		setSystemTime(new Date(now));
		credentials = [credential("plus", "Plus", "oauth", { usedBy: ["ada"], default: true }, "openai-codex")];
		const written = now - 3600_000;
		await mount(chatgptLimit({ retryAfterSeconds: 5 * 3600 }), { backendId: "hotline", at: written });
		expect(text()).toContain(`This ChatGPT sign-in reached its usage limit. Resets at ${clock.format(at(23, 18, 0))}.`);
		expect(text()).not.toContain("Retry after");

		await remount(chatgptLimit({ retryAfterSeconds: 20 * 3600 }), { backendId: "hotline", at: written });
		expect(text()).toContain(`Resets tomorrow at ${clock.format(at(24, 9, 0))}.`);

		await remount(chatgptLimit({ retryAfterSeconds: 3600 }), { backendId: "hotline", at: now - 6 * 3600_000 });
		expect(text()).toContain("This ChatGPT sign-in reached its usage limit.");
		expect(text()).not.toContain("Resets");

		await remount(notice({ kind: "rate_limit", title: "Provider rate limit", summary: "The provider is temporarily limiting requests.", retryAfterSeconds: 20 }));
		expect(text()).toContain("Retry after 20s");
		expect(text()).not.toContain("Resets");
	});
});

describe("an account's failure card", () => {
	test("only the latest usage limit, with the teammate still on the failed account, asks for each status", async () => {
		const probes = () => calls.filter(([cmd]) => cmd === "accounts.status");
		await mount(limit({ accountId: "a1" }), { latest: false });
		expect(text()).toContain("Account 1 hit its usage limit");
		expect(probes()).toEqual([]);

		accounts = [a1([]), a2(), a3()];
		await remount(limit({ accountId: "a1" }));
		expect(text()).toContain("Now on This computer");
		expect(probes()).toEqual([]);

		accounts = [a1(), a2(), a3()];
		await remount(limit({ accountId: "a1" }));
		expect(probes()).toEqual([
			["accounts.status", { accountId: "a2" }],
			["accounts.status", { accountId: "a3" }],
		]);
	});

	test("a card with no window to read it from is the provider's own", async () => {
		const { renderToStaticMarkup } = await import("react-dom/server");
		const html = renderToStaticMarkup(<ErrorCard text={limit({ accountId: "a1" })} personaId="ada" onRunAgain={async () => null} />);
		expect(html).toContain("Claude Code usage limit reached");
		expect(html).toContain("Try again");
		expect(html).not.toContain("Continue on");
	});

	test("an account-bound sign-in failure offers Sign in and Choose account, not the harness methods", async () => {
		const failure = notice({ kind: "agent_auth", title: "Claude Code sign-in required", summary: "Sign in again", accountId: "a3", signIn });
		await mount(failure);
		expect(text()).toContain("Account 3 isn't signed in");
		expect(text()).toContain(
			"Ada runs on Account 3, and Claude Code says nobody is signed in there. Your message is kept; send it again once the account is ready.",
		);
		expect(buttons()).toContain("Sign in Account 3");
		expect(buttons()).toContain("Choose account");
		expect(text()).not.toContain("Sign-in method");
		expect(text()).not.toContain("Anthropic Console");
		await click("Choose account");
		expect(chosen).toBe(1);

		poll = { state: "succeeded", output: "" };
		await click("Sign in Account 3");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a3" }]);
		expect(calls.some(([cmd]) => cmd.startsWith("agent.auth."))).toBe(false);
		expect(text()).toContain("Signed in. The teammate is ready for another message.");
		await click("Review failed message");
		expect(retried).toBe(1);

		accounts = [a1(), a2(), { ...a3(), canSignInHere: false }];
		await remount(failure);
		expect(buttons()).not.toContain("Sign in Account 3");
		expect(text()).toContain("Sign in from the desk itself");
		expect(buttons()).toContain("Choose account");
	});

	test("an unavailable account offers Show folder only for a listed folder on the Desk", async () => {
		const folder = notice({
			kind: "account_unavailable",
			code: "folder_unavailable",
			accountId: "a1",
			title: "Ada can't start",
			summary: "Can't open Account 1's folder (/data/agent-accounts/a1): permission denied. Fix the folder or pick another account.",
		});
		await mount(folder);
		expect(text()).toContain("Can't open Account 1's folder");
		expect(text()).toContain("permission denied. Fix the folder or pick another account.");
		expect(buttons()).toEqual(["Show folder", "Choose account"]);

		replaceDesks([remote]);
		await remount(folder);
		expect(buttons()).toEqual(["Choose account"]);

		replaceDesks([local]);
		await remount(
			notice({
				kind: "account_unavailable",
				code: "unknown_account",
				accountId: "gone",
				title: "Ada can't start",
				summary: "Pick a Claude Code account for Ada: the one it was set to is not on this desk.",
			}),
		);
		expect(text()).toContain("Pick a Claude Code account");
		expect(text()).toContain("Pick a Claude Code account for Ada: the one it was set to is not on this desk.");
		expect(buttons()).toEqual(["Choose account"]);
		await click("Choose account");
		expect(chosen).toBe(1);
	});

	test("a card with no account calls no wire command", async () => {
		await mount(notice({ kind: "agent_auth", title: "Claude Code sign-in required", summary: "Sign in again", signIn }));
		expect(text()).toContain("Sign-in method");
		await remount(notice({ kind: "quota", title: "Quota spent", summary: "No credit left" }));
		await remount(notice({ kind: "auth", title: "Provider sign-in required", summary: "Check its credentials", accountId: "a1" }));
		await remount("Plain diagnostic text");
		expect(calls).toEqual([]);
	});
});
