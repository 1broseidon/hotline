import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountView, PersonaDraft } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {} }) } });
const { NewTeammateForm } = await import("../src/components/NewTeammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

const backends = [
	{ id: "hotline", name: "Hotline Agent", description: "Hotline's own agent." },
	{ id: "claude-acp", name: "Claude Code", description: "Anthropic's CLI." },
	{ id: "codex-acp", name: "Codex", description: "OpenAI's CLI." },
];
const agents = [
	{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true },
	{ backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false },
];
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
const a1 = () => account("a1", "Account 1", { status: { state: "signed_in", checkedAt: 1, email: "dev@example.com", plan: "max" }, usedBy: ["ada"] });
const a2 = () => account("a2", "Account 2");
const a3 = () => account("a3", "Account 3", { status: { state: "not_checked" } });

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let created: string[];
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

function respond(cmd: string, params: Record<string, unknown>): unknown {
	if (cmd === "backends.list") return backends;
	if (cmd === "accounts.list") return { agents, accounts };
	if (cmd === "persona.create") return { id: "p1", ...(params["draft"] as PersonaDraft) };
	if (cmd === "session.start") return null;
	if (cmd === "accounts.create") {
		if (createRefusal !== null) throw new Error(createRefusal);
		accounts = [...accounts, a3()];
		return { account: a3() };
	}
	if (cmd === "accounts.login.start") {
		change(params["accountId"], (one) => ({ ...one, attempt: { id: "l1", state: "running" } }));
		return { id: "l1" };
	}
	if (cmd === "accounts.login.poll") return { state: "running", output: "" };
	if (cmd === "accounts.login.cancel") {
		change(params["accountId"], (one) =>
			one.attempt?.state === "running" ? { ...one, attempt: { id: "l1", state: "failed", error: "Sign-in cancelled." }, status: { state: "checking" } } : one,
		);
		return null;
	}
	if (cmd === "accounts.login.input") return null;
	if (cmd === "accounts.status") {
		const checked = { state: "signed_out", checkedAt: 2 } as const;
		change(params["accountId"], (one) => ({ ...one, status: checked }));
		return checked;
	}
	throw new Error(`Not in this test: ${cmd}`);
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	accounts = [a1(), a2()];
	created = [];
	createRefusal = null;
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push([cmd, params]);
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
	await teardown();
});

async function mount() {
	await act(async () => {
		root!.render(<NewTeammateForm models={[]} onCreated={(id) => created.push(id)} />);
	});
	await settle();
	await act(async () => {
		thinksWith().click();
	});
	await settle();
}
async function unmount() {
	await act(async () => {
		root?.unmount();
		root = null;
	});
}
async function remount() {
	await unmount();
	await act(async () => {
		root = (await import("react-dom/client")).createRoot(container);
	});
	await mount();
}
async function settle() {
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const named = (cmd: string) => calls.filter(([one]) => one === cmd);
function thinksWith(): HTMLButtonElement {
	const head = [...container.querySelectorAll<HTMLButtonElement>("button.nt-fold-head")].find((node) => node.textContent?.startsWith("Thinks with"));
	if (!head) throw new Error(`No Thinks with: ${text()}`);
	return head;
}
async function agent(id: string) {
	const radio = container.querySelector<HTMLInputElement>(`input[data-backend-id="${id}"]`);
	if (!radio) throw new Error(`No agent ${id}: ${text()}`);
	await act(async () => {
		radio.click();
	});
	await settle();
}
function accountList(): HTMLElement | null {
	return container.querySelector<HTMLElement>('[role="radiogroup"][aria-labelledby="new-account"]');
}
function row(name: string): HTMLElement {
	const title = [...(accountList()?.querySelectorAll(".group-row-title") ?? [])].find((node) => node.textContent === name);
	if (!title) throw new Error(`Missing row ${name}: ${text()}`);
	return title.closest(".group-row") as HTMLElement;
}
const radio = (name: string) => row(name).querySelector("input") as HTMLInputElement;
async function pick(name: string) {
	await act(async () => {
		radio(name).click();
	});
	await settle();
}
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
async function type(input: HTMLInputElement, value: string) {
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }));
	});
}
async function submit(name: string) {
	await type(container.querySelector<HTMLInputElement>("#new-name")!, name);
	await click("Add teammate");
}
const draft = () => named("persona.create").at(-1)?.[1]["draft"] as PersonaDraft;

describe("a new teammate's agent account", () => {
	test("the Account list shows only for an agent with accounts", async () => {
		await mount();
		expect(container.querySelector("#new-account")).toBeNull();
		expect(named("accounts.list")).toHaveLength(0);

		await agent("claude-acp");
		expect(container.querySelector("#new-account")?.textContent).toBe("Account");
		expect(accountList()).not.toBeNull();
		expect(row("System default").textContent).toContain("Claude Code's own sign-in on this computer, as before");
		expect(row("Account 1").textContent).toContain("dev@example.com · Max");
		expect(row("Account 2").textContent).toContain("Not signed in · sign in to use it");
		expect(radio("System default").checked).toBe(true);
		expect(text()).toContain("Rename or remove accounts later in Settings › Agent accounts.");

		await agent("codex-acp");
		expect(container.querySelector("#new-account")).toBeNull();
	});

	test("the picked account rides in the draft; System default adds nothing", async () => {
		await mount();
		await agent("claude-acp");
		await pick("Account 1");
		expect(thinksWith().textContent).toContain("Claude Code · Account 1");
		await submit("Ada");
		expect(draft().accountId).toBe("a1");
		expect(draft().backendId).toBe("claude-acp");

		await remount();
		await agent("claude-acp");
		await submit("Mack");
		expect("accountId" in draft()).toBe(false);
		expect(thinksWith().textContent).not.toContain("·");
	});

	test("Add account starts with your setup, and Change or customize opens Start with", async () => {
		await mount();
		await agent("claude-acp");
		await click("Add account");
		expect(text()).toContain("Starts with your Claude Code setup · Change or customize");
		expect(text()).not.toContain("My Claude Code setup");
		await click("Change or customize");
		expect(text()).toContain("My Claude Code setup");
		expect(text()).toContain("Empty");
		expect(text()).toContain("Customize what is copied…");
	});

	test("a name refused as taken reads the accounts again, so the account it names is listed", async () => {
		await mount();
		await agent("claude-acp");
		accounts = [...accounts, account("r1", "Reviewer")];
		createRefusal = "There is already an account named Reviewer for that agent.";
		await click("Add account");
		await click("Add and sign in");
		expect(text()).toContain("There is already an account named Reviewer for that agent.");
		expect(row("Reviewer")).toBeDefined();
	});

	test("an inline account is selected at once, and Sign in later leaves it Not signed in", async () => {
		await mount();
		await agent("claude-acp");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await click("Add account");
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]);
		expect(radio("Account 3").checked).toBe(true);
		expect(row("Account 3").textContent).toContain("Signing in…");
		expect(text()).toContain("A Claude Code window opened for Account 3.");
		expect(button("Add teammate").disabled).toBe(true);
		const enter = new window.Event("submit", { bubbles: true, cancelable: true });
		await act(async () => {
			container.querySelector("form")!.dispatchEvent(enter);
		});
		await settle();
		expect(enter.defaultPrevented).toBe(true);
		expect(named("persona.create")).toHaveLength(0);
		expect(named("accounts.login.cancel")).toHaveLength(0);

		await click("Sign in later");
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a3", id: "l1" }]);
		expect(calls.findIndex(([cmd]) => cmd === "accounts.status")).toBeGreaterThan(calls.findIndex(([cmd]) => cmd === "accounts.login.cancel"));
		expect(radio("Account 3").checked).toBe(true);
		expect(row("Account 3").textContent).toContain("Not signed in · sign in to use it");

		expect(button("Add teammate").disabled).toBe(false);
		await click("Add teammate");
		expect(draft().accountId).toBe("a3");
		expect(created).toEqual(["p1"]);
	});

	test("Enter in the new account's email adds the account, not the teammate", async () => {
		await mount();
		await agent("claude-acp");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await click("Add account");
		const email = container.querySelector<HTMLInputElement>('input[aria-label="Email of the Claude Code account (optional)"]')!;
		await type(email, "dev@example.com");
		const enter = new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
		await act(async () => {
			email.dispatchEvent(enter);
		});
		await settle();
		expect(enter.defaultPrevented).toBe(true);
		expect(named("accounts.create")).toHaveLength(1);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a3", email: "dev@example.com" }]);
		expect(named("persona.create")).toHaveLength(0);
	});
});
