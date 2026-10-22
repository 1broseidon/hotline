import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountStatusState, AccountView, RosterEntry } from "../src/generated/contract";
import { installDom } from "./dom";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const { AgentAccountsSection } = await import("../src/components/settings/AgentAccounts");
const { accountJob } = await import("../src/components/accounts/AccountSignIn");
const { nextAccountName } = await import("../src/components/accounts/words");
Reflect.deleteProperty(globalThis, "localStorage");

const epoch = 1_800_000_000_000;
const agents = [
	{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true },
	{ backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false },
];
function account(id: string, name: string, extra: Partial<AccountView> = {}, loginAt?: number): AccountView {
	return {
		account: { id, backendId: "claude-acp", name, createdAt: epoch, ...(loginAt === undefined ? {} : { loginAt }) },
		status: { state: "signed_out", checkedAt: epoch },
		usedBy: [],
		folder: `/data/agent-accounts/${id}`,
		canSignInHere: true,
		...extra,
	};
}
const a1 = () => account("a1", "Account 1", { status: { state: "signed_in", checkedAt: epoch, email: "dev@example.com", plan: "max" }, usedBy: ["ada"] }, epoch);
const a2 = () => account("a2", "Account 2");
const a3 = () => account("a3", "Account 3", { status: { state: "not_checked" } });

function entry(id: string, name: string, agentAccountId?: string): RosterEntry {
	return {
		persona: {
			id, name, goal: "", backendId: "claude-acp", cwd: "/work",
			mcpPolicy: { mode: "all", serverIds: [] }, skillPolicy: { mode: "all", names: [] },
			backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0,
			...(agentAccountId === undefined ? {} : { agentAccountId }),
		},
		session: {
			personaId: id, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
			capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		},
		waiting: false,
		drawing: false,
	} as RosterEntry;
}
const roster = [entry("ada", "Ada", "a1"), entry("mack", "Mack")];

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let made: AccountView;
let poll: { state: "running" | "succeeded" | "failed"; output: string; error?: string };
let checked: AccountView["status"];
let cancelLands: boolean;
let createRefusal: string | null;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function change(id: unknown, edit: (view: AccountView) => AccountView) {
	accounts = accounts.map((one) => (one.account.id === id ? edit(one) : one));
}
function cancelled(view: AccountView): AccountView {
	if (view.attempt?.state !== "running") return view;
	return { ...view, attempt: { id: view.attempt.id, state: "failed", error: "Sign-in cancelled." }, status: { state: "checking" } };
}

function respond(cmd: string, params: Record<string, unknown>): unknown {
	if (cmd === "accounts.list") return { agents, accounts };
	if (cmd === "accounts.create") {
		if (createRefusal !== null) throw new Error(createRefusal);
		accounts = [...accounts, made];
		return { account: made };
	}
	if (cmd === "accounts.login.start") {
		change(params["accountId"], (one) => ({ ...one, attempt: { id: "l1", state: "running" } }));
		return { id: "l1" };
	}
	if (cmd === "accounts.login.poll") {
		change(params["accountId"], (one) => ({ ...one, attempt: { id: "l1", state: poll.state, ...(poll.error === undefined ? {} : { error: poll.error }) } }));
		return poll;
	}
	if (cmd === "accounts.login.cancel") {
		if (cancelLands) change(params["accountId"], cancelled);
		return null;
	}
	if (cmd === "accounts.login.input") return null;
	if (cmd === "accounts.status") {
		change(params["accountId"], (one) => ({ ...one, status: checked }));
		return checked;
	}
	if (cmd === "accounts.rename") {
		accounts = accounts.map((one) => (one.account.id === params["accountId"] ? { ...one, account: { ...one.account, name: String(params["name"]) } } : one));
		return accounts.find((one) => one.account.id === params["accountId"]);
	}
	if (cmd === "accounts.remove") {
		accounts = accounts.filter((one) => one.account.id !== params["accountId"]);
		return null;
	}
	if (cmd === "accounts.setup.list") return { categories: [] };
	throw new Error(`Unexpected command ${cmd}`);
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	accounts = [a1(), a2()];
	made = a3();
	poll = { state: "running", output: "" };
	checked = { state: "signed_in", checkedAt: Date.now(), email: "two@example.com", plan: "pro" };
	cancelLands = true;
	createRefusal = null;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push([cmd, params]);
		return respond(cmd, params);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	// React reads the DOM when it loads, so it loads once the DOM is here.
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
	await teardown();
});

async function mount() {
	await act(async () => {
		root!.render(<AgentAccountsSection roster={roster} />);
	});
	await settle();
}
async function unmount() {
	await act(async () => {
		root?.unmount();
		root = null;
	});
}
async function settle() {
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const band = () => container.querySelector(".band h2")?.textContent;
const named = (cmd: string) => calls.filter(([one]) => one === cmd);
function row(name: string): HTMLElement {
	const title = [...container.querySelectorAll(".group-row-title")].find((node) => node.textContent === name);
	if (!title) throw new Error(`Missing row ${name}: ${text()}`);
	return title.closest(".group-row") as HTMLElement;
}
function button(label: string, within: ParentNode = container) {
	const found = [...within.querySelectorAll("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
const hasButton = (label: string) => [...container.querySelectorAll("button")].some((node) => node.textContent === label);
async function click(label: string, within?: ParentNode) {
	const found = button(label, within);
	await act(async () => {
		found.click();
	});
	await settle();
}
async function openAgent() {
	await act(async () => {
		row("Claude Code").click();
	});
	await settle();
}
async function openAccount(name: string) {
	await act(async () => {
		row(name).querySelector("button")!.click();
	});
	await settle();
}
function field(label: string): HTMLInputElement {
	const found = [...container.querySelectorAll("label")].find((node) => node.textContent?.startsWith(label));
	const input = found?.htmlFor ? (document.getElementById(found.htmlFor) as HTMLInputElement | null) : (container.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement | null);
	if (!input) throw new Error(`Missing field ${label}: ${text()}`);
	return input;
}
async function type(input: HTMLInputElement, value: string) {
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}
async function tick(title: string) {
	const found = [...container.querySelectorAll("label")].find((node) => node.querySelector("input") && node.textContent?.startsWith(title));
	if (!found) throw new Error(`Missing choice ${title}: ${text()}`);
	await act(async () => {
		found.querySelector("input")!.click();
	});
}

describe("Settings › Agent accounts", () => {
	test("an agent with accounts opens its page; the others use their own sign-in", async () => {
		await mount();
		expect(named("accounts.list")).toHaveLength(1);
		expect(band()).toBe("Agent accounts");
		expect(row("Claude Code").textContent).toContain("2 accounts · 1 not signed in");
		expect(row("Codex").closest("button")).toBeNull();
		expect(text()).toContain("Uses its own sign-in");
		expect(text()).toContain("Hotline never sees a password or token: each agent's CLI keeps its own sign-in.");
		await openAgent();
		expect(band()).toBe("Claude Code");
	});

	test("the agent page shows System default and each account's line", async () => {
		await mount();
		await openAgent();
		expect(text()).toContain("Claude Code's own sign-in on this computer, as before · Used by Mack");
		expect(text()).toContain("Built in");
		expect(row("Account 1").textContent).toContain("Signed in · dev@example.com · Used by Ada");
		expect(row("Account 1").textContent).not.toContain("Max");
		expect(row("Account 2").textContent).toContain("Not signed in · Not used yet");
		expect(text()).toContain("Claude Code keeps each account's sign-in, history and settings in its own folder.");
	});

	test("each status state reads as its words", async () => {
		const words: [AccountStatusState, string][] = [
			["not_checked", "Not checked"],
			["checking", "Checking…"],
			["signed_in", "Signed in"],
			["signed_out", "Not signed in"],
			["unavailable", "Status unavailable"],
		];
		accounts = words.map(([state], index) => account(`s${index}`, `Account ${index + 1}`, { status: { state } }));
		await mount();
		await openAgent();
		for (const [index, [state, said]] of words.entries()) {
			const line = row(`Account ${index + 1}`);
			expect(line.textContent).toContain(said);
			expect(line.querySelector(".agent-account-dot")?.getAttribute("data-state") ?? null).toBe(state === "not_checked" ? null : state);
			expect([...line.querySelectorAll("button")].some((node) => node.textContent === "Check")).toBe(state === "not_checked" || state === "unavailable");
		}
		expect(row("Account 1").querySelector(".agent-account-dot")).toBeNull();
		await click("Check", row("Account 1"));
		expect(calls).toContainEqual(["accounts.status", { accountId: "s0" }]);
		expect(row("Account 1").textContent).toContain("Signed in · two@example.com");
	});

	test("a blank name adds Account 3 and a Windows sign-in shows only Cancel", async () => {
		await mount();
		await openAgent();
		await click("Add account");
		expect(field("Name").placeholder).toBe("Account 3");
		expect([...container.querySelectorAll(".group-title")].map((node) => node.textContent)).toContain("What a new account starts with");
		expect(text()).toContain("Chosen once, here. After that the account is edited on its own in its Setup, where it can also copy from your setup again.");
		const choice = container.querySelector('[role="radiogroup"]')!;
		const label = document.getElementById(choice.getAttribute("aria-labelledby") ?? "");
		expect(label?.textContent).toBe("Start with");
		expect(label?.closest('[aria-hidden="true"]') ?? null).toBeNull();
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a3" }]);
		expect(row("Account 3").textContent).toContain("Signing in…");
		expect(text()).toContain("A Claude Code window opened for Account 3. Finish signing in there; Hotline checks when it closes.");
		expect(container.querySelector("[data-private-terminal]")).toBeNull();
		const panel = container.querySelector('section[aria-label="Harness sign-in"]')!;
		expect([...panel.querySelectorAll("button")].map((node) => node.textContent)).toEqual(["Cancel"]);
		checked = { state: "signed_out", checkedAt: Date.now() };
		await click("Cancel", panel);
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a3", id: "l1" }]);
		expect(calls.findIndex(([cmd]) => cmd === "accounts.status")).toBeGreaterThan(calls.findIndex(([cmd]) => cmd === "accounts.login.cancel"));
		expect(text()).not.toContain("A Claude Code window opened");
		expect(row("Account 3").textContent).toContain("Sign-in did not finish: Sign-in cancelled.");
		expect(row("Account 3").textContent).not.toContain("Signing in…");
		expect(button("Try again", row("Account 3"))).toBeDefined();
	});

	test("a name refused as taken reads the accounts again, so the account it names is listed", async () => {
		await mount();
		await openAgent();
		accounts = [...accounts, account("r1", "Reviewer")];
		createRefusal = "There is already an account named Reviewer for that agent.";
		await click("Add account");
		await type(field("Name"), "Reviewer");
		await click("Add and sign in");
		expect(text()).toContain("There is already an account named Reviewer for that agent.");
		expect(row("Reviewer")).toBeDefined();
	});

	test("Back mid sign-in leaves the row as the cancelled attempt ended", async () => {
		checked = { state: "signed_out", checkedAt: Date.now() };
		await mount();
		await openAgent();
		await openAccount("Account 2");
		await click("Sign in");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a2" }]);
		await click("Back to the team");
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a2", id: "l1" }]);
		expect(row("Account 2").textContent).toContain("Sign-in did not finish: Sign-in cancelled.");
		expect(row("Account 2").textContent).not.toContain("Signing in…");
		expect(button("Try again", row("Account 2"))).toBeDefined();
	});

	test("a list that still says signing in after a cancel is read again", async () => {
		checked = { state: "signed_out", checkedAt: Date.now() };
		cancelLands = false;
		await mount();
		await openAgent();
		await openAccount("Account 2");
		await click("Sign in");
		await click("Back to the team");
		expect(row("Account 2").textContent).not.toContain("Sign-in did not finish");
		expect(named("accounts.status")).toHaveLength(0);
		const listed = named("accounts.list").length;
		change("a2", cancelled);
		for (let round = 0; round < 100 && named("accounts.list").length === listed; round++) {
			await act(async () => {
				await new Promise((resolve) => setTimeout(resolve, 20));
			});
		}
		await settle();
		expect(named("accounts.list").length).toBeGreaterThan(listed);
		expect(row("Account 2").textContent).toContain("Sign-in did not finish: Sign-in cancelled.");
	});

	test("an attempt running elsewhere reads Signing in… on the account page, with neither Sign in nor Check", async () => {
		accounts = [a1(), { ...a2(), attempt: { id: "l0", state: "running" } }];
		await mount();
		await openAgent();
		await openAccount("Account 2");
		expect(container.querySelector(".nt-card")?.textContent).toContain("Signing in…");
		expect(hasButton("Sign in")).toBe(false);
		expect(hasButton("Check")).toBe(false);
	});

	test("an owner seat adds an account and is told to sign it in from the desk", async () => {
		made = { ...a3(), canSignInHere: false };
		await mount();
		await openAgent();
		await click("Add account");
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]);
		expect(named("accounts.login.start")).toHaveLength(0);
		expect(container.querySelector('section[aria-label="Harness sign-in"]')).toBeNull();
		expect(text()).toContain("Sign in from the desk itself");
		expect(row("Account 3").textContent).not.toContain("Signing in…");
	});

	test("Customize what is copied sends the ticked; Empty sends none", async () => {
		await mount();
		await openAgent();
		await click("Add account");
		await click("Customize what is copied…");
		await tick("Settings");
		await click("Add and sign in");
		expect(named("accounts.create").at(-1)?.[1]).toEqual({ backendId: "claude-acp", name: "Account 3", startWith: "system_default", categories: ["instructions", "skills", "plugins", "mcp"] });

		await unmount();
		accounts = [a1(), a2()];
		await act(async () => {
			const { createRoot } = await import("react-dom/client");
			root = createRoot(container);
		});
		await mount();
		await openAgent();
		await click("Add account");
		await click("Customize what is copied…");
		await tick("Settings");
		await tick("Empty");
		expect(hasButton("Customize what is copied…") || hasButton("Hide what is copied")).toBe(false);
		await click("Add and sign in");
		expect(named("accounts.create").at(-1)?.[1]).toEqual({ backendId: "claude-acp", name: "Account 3", startWith: "empty" });
	});

	test("rename saves the trimmed name", async () => {
		await mount();
		await openAgent();
		await openAccount("Account 1");
		await click("Rename");
		await type(field("Account name"), "  Reviewer seat ");
		await click("Save");
		expect(calls).toContainEqual(["accounts.rename", { accountId: "a1", name: "Reviewer seat" }]);
		expect(band()).toBe("Reviewer seat");
		const renames = named("accounts.rename").length;
		await click("Rename");
		await type(field("Account name"), "   ");
		await click("Save");
		expect(named("accounts.rename")).toHaveLength(renames);
		expect(band()).toBe("Reviewer seat");
	});

	test("remove is refused in place while Ada uses it, and frees an unused account", async () => {
		await mount();
		await openAgent();
		await openAccount("Account 1");
		await click("Remove account");
		expect(text()).toContain("Account 1 can't be removed while Ada use it.");
		expect(text()).toContain("Give them another account in their settings first; Hotline never moves a teammate to another account for you.");
		await click("OK");
		expect(named("accounts.remove")).toHaveLength(0);
		expect(text()).not.toContain("can't be removed");

		await click("Back to the team");
		await openAccount("Account 2");
		await click("Remove account");
		expect(text()).toContain("Remove Account 2?");
		expect(text()).toContain("Hotline forgets Account 2. Its folder and the sign-in in it stay at /data/agent-accounts/a2; sign out there with Claude Code's own sign-out if you no longer need it.");
		await click("Remove");
		expect(calls).toContainEqual(["accounts.remove", { accountId: "a2" }]);
		expect(band()).toBe("Claude Code");
		expect(text()).not.toContain("Account 2");
	});

	test("Check asks the agent; an owner seat signs in from the desk", async () => {
		await mount();
		await openAgent();
		await openAccount("Account 2");
		expect(text()).toContain("Its teammates ask you to sign in at their next message");
		await click("Check");
		expect(calls).toContainEqual(["accounts.status", { accountId: "a2" }]);
		expect(text()).toContain("two@example.com · Pro");
		expect(text()).toContain("Checked with Claude Code just now");

		await unmount();
		accounts = [a1(), { ...a2(), canSignInHere: false }];
		await act(async () => {
			const { createRoot } = await import("react-dom/client");
			root = createRoot(container);
		});
		await mount();
		await openAgent();
		await openAccount("Account 2");
		expect(text()).toContain("Sign in from the desk itself");
		expect(hasButton("Sign in")).toBe(false);
	});

	test("a second sign-in says its teammates start fresh; a first one does not", async () => {
		poll = { state: "succeeded", output: "" };
		await mount();
		await openAgent();
		await openAccount("Account 1");
		expect(text()).toContain("dev@example.com · Max");
		await click("Sign in again");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a1" }]);
		expect(text()).toContain("Account 1 has a new sign-in. Its teammates start fresh once their work ends.");
		await click("Back to the team");
		expect(row("Account 1").textContent).toContain("Account 1 has a new sign-in. Its teammates start fresh once their work ends.");

		await openAccount("Account 2");
		await click("Sign in");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a2" }]);
		await click("Back to the team");
		expect(row("Account 2").textContent).not.toContain("has a new sign-in");
	});

	test("a sign-in that did not finish says why on its row and offers Try again", async () => {
		accounts = [
			{ ...a1(), attempt: { id: "l0", state: "failed", error: "Sign-in cancelled." } },
			{ ...a2(), attempt: { id: "l0", state: "failed", error: "Claude Code closed before it was signed in." } },
		];
		await mount();
		await openAgent();
		expect(row("Account 1").textContent).toContain("Signed in · Sign-in did not finish: Sign-in cancelled.");
		expect(row("Account 1").querySelector(".agent-account-dot")?.getAttribute("data-state")).toBe("signed_in");
		expect(row("Account 2").textContent).toContain("Sign-in did not finish: Claude Code closed before it was signed in.");
		expect(row("Account 2").textContent).not.toContain("Signed in ·");
		expect(row("Account 2").querySelector(".agent-account-dot")?.getAttribute("data-state")).toBe("failed");
		await click("Try again", row("Account 2"));
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a2" }]);
		expect(text()).toContain("A Claude Code window opened for Account 2.");
	});

	test("an account's sign-in job speaks accounts.login.*", async () => {
		const job = accountJob("a1", "dev@example.com");
		await job.start();
		await job.poll("l1");
		await job.input("l1", "x");
		await job.cancel("l1");
		expect(calls).toEqual([
			["accounts.login.start", { accountId: "a1", email: "dev@example.com" }],
			["accounts.login.poll", { accountId: "a1", id: "l1" }],
			["accounts.login.input", { accountId: "a1", id: "l1", input: "x" }],
			["accounts.login.cancel", { accountId: "a1", id: "l1" }],
		]);
		await accountJob("a2").start();
		expect(calls.at(-1)).toEqual(["accounts.login.start", { accountId: "a2" }]);
		expect(Object.keys(calls.at(-1)![1])).toEqual(["accountId"]);
	});

	test("opening lands on the account's page with its agent behind Back", async () => {
		await act(async () => {
			root!.render(<AgentAccountsSection roster={roster} opening="a1" />);
		});
		await settle();
		expect(band()).toBe("Account 1");
		await click("Back to the team");
		expect(band()).toBe("Claude Code");
	});

	test("the next blank name skips a taken one", () => {
		expect(nextAccountName(["Account 1", "account 3"])).toBe("Account 4");
		expect(nextAccountName([])).toBe("Account 1");
	});
});
