import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountView, ConfigChoice, Credential, CredentialAccount, PersonaDraft, Provider } from "../src/generated/contract";
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
	{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true, providerId: "anthropic" },
	{ backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false },
];
const providers: Provider[] = [
	{ id: "anthropic", name: "Anthropic", credentialKinds: ["api_key"], modelDiscovery: false },
	{ id: "openai", name: "OpenAI", credentialKinds: ["api_key"], modelDiscovery: false },
];
const models: ConfigChoice[] = [
	{ id: "anthropic/claude-x", name: "Claude X" },
	{ id: "openai/gpt-x", name: "GPT X" },
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
function key(id: string, label: string, providerId = "anthropic", isDefault = false): CredentialAccount {
	return {
		credential: { id, providerId, credentialKind: "api_key", label, revoked: false, createdAt: 1, updatedAt: 1 },
		section: "api_key",
		usedBy: [],
		default: isDefault,
	};
}

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let credentials: CredentialAccount[];
let created: string[];
let createRefusal: string | null;
let createGate: Promise<void>;
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
	if (cmd === "providers.list") return providers;
	if (cmd === "accounts.list") return { agents, accounts, credentials };
	if (cmd === "persona.create") return createGate.then(() => ({ id: "p1", ...(params["draft"] as PersonaDraft) }));
	if (cmd === "session.start") return null;
	if (cmd === "credential.create") {
		const made: Credential = {
			id: "C",
			providerId: params["providerId"] as string,
			credentialKind: "api_key",
			label: params["label"] as string,
			revoked: false,
			createdAt: 2,
			updatedAt: 2,
		};
		credentials = [...credentials, { credential: made, section: "api_key", usedBy: [], default: false }];
		return made;
	}
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
	credentials = [key("A", "Key A", "anthropic", true), key("B", "Key B"), key("O", "OpenAI key", "openai", true)];
	created = [];
	createRefusal = null;
	createGate = Promise.resolve();
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
		root!.render(<NewTeammateForm models={models} onCreated={(id) => created.push(id)} />);
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
const picker = () => container.querySelector<HTMLButtonElement>('button[aria-label="Account"]');
const labels = () => [...container.querySelectorAll("p.label")].map((node) => node.textContent);
const options = () => [...document.querySelectorAll<HTMLElement>('[role="option"]')];
const optionNames = () => options().map((one) => one.querySelector(".menu-item-text")?.firstChild?.textContent);
async function openPicker() {
	const shut = picker();
	if (!shut) throw new Error(`No Account field: ${text()}`);
	await act(async () => {
		shut.click();
	});
	await settle();
}
async function closePicker() {
	await act(async () => {
		document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
	});
	await settle();
}
async function choose(name: string) {
	await openPicker();
	const option = options().find((one) => one.querySelector(".menu-item-text")?.firstChild?.textContent === name);
	if (!option) throw new Error(`No account ${name}: ${optionNames().join(", ")}`);
	await act(async () => {
		option.click();
	});
	await settle();
}
async function add(label: string) {
	await openPicker();
	const row = [...document.querySelectorAll<HTMLButtonElement>(".account-picker-add")].find((node) => node.textContent === label);
	if (!row) throw new Error(`No ${label}`);
	await act(async () => {
		row.click();
	});
	await settle();
}
async function model(name: string) {
	const shut = container.querySelector<HTMLButtonElement>('button[aria-label="Model"]')!;
	await act(async () => {
		shut.click();
	});
	const item = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
		(one) => one.querySelector(".menu-item-text")?.firstChild?.textContent === name,
	);
	if (!item) throw new Error(`No model ${name}`);
	await act(async () => {
		item.click();
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
async function enter(input: HTMLInputElement): Promise<boolean> {
	const press = new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
	await act(async () => {
		input.dispatchEvent(press);
	});
	await settle();
	return press.defaultPrevented;
}
async function submit(name: string) {
	await type(container.querySelector<HTMLInputElement>("#new-name")!, name);
	await click("Add teammate");
}
const draft = () => named("persona.create").at(-1)?.[1]["draft"] as PersonaDraft;
const NEVER_SEES = "Hotline never sees a password or token: each agent's CLI keeps its own sign-in.";

describe("a new Claude Code teammate's account", () => {
	test("This computer alone is nothing to pick, so there is no Account field", async () => {
		accounts = [];
		await mount();
		await agent("claude-acp");
		expect(named("accounts.list").length).toBeGreaterThan(0);
		expect(container.querySelector("#new-account")).toBeNull();
		expect(picker()).toBeNull();
	});

	test("one subscription shows the field, starting on This computer", async () => {
		accounts = [a1()];
		await mount();
		await agent("claude-acp");
		expect(container.querySelector("#new-account")?.textContent).toBe("Account");
		expect(picker()!.textContent).toBe("This computer · Claude Code's own sign-in, as before");
		expect(text()).toContain("Rename or remove accounts later in Settings › Providers.");
		await openPicker();
		expect(optionNames()).toEqual(["Account 1", "This computer"]);
		expect(options()[0]!.textContent).toContain("dev@example.com · Max");
		expect([...document.querySelectorAll(".account-picker-add")].map((node) => node.textContent)).toEqual(["Add subscription"]);
		await closePicker();

		await agent("codex-acp");
		expect(container.querySelector("#new-account")).toBeNull();
	});

	test("the picked account rides in the draft; This computer adds nothing", async () => {
		await mount();
		await agent("claude-acp");
		await choose("Account 1");
		expect(picker()!.textContent).toBe("Account 1 · dev@example.com · Max");
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

	test("an account picked for Claude Code does not ride along to Hotline Agent", async () => {
		await mount();
		await agent("claude-acp");
		await choose("Account 1");
		await agent("hotline");
		await model("Claude X");
		expect(picker()!.textContent).toBe("Key A · API key · pay per use · Default");
		await submit("Ada");
		expect(draft().backendId).toBe("hotline");
		expect("accountId" in draft()).toBe(false);
	});

	test("the Account field cannot change while the teammate is being made", async () => {
		let release = () => {};
		createGate = new Promise((resolve) => {
			release = resolve;
		});
		await mount();
		await agent("claude-acp");
		expect(picker()!.disabled).toBe(false);
		await submit("Ada");
		expect(button("Setting up…").disabled).toBe(true);
		expect(picker()!.disabled).toBe(true);
		await act(async () => {
			release();
		});
		await settle();
		expect(created).toEqual(["p1"]);
	});

	test("Add subscription starts with your setup, says Hotline never sees the sign-in, and Change or customize opens Start with", async () => {
		await mount();
		await agent("claude-acp");
		expect(text()).not.toContain(NEVER_SEES);
		await add("Add subscription");
		expect(text()).toContain("Starts with your Claude Code setup · Change or customize");
		expect(text()).toContain(NEVER_SEES);
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
		await add("Add subscription");
		await click("Add and sign in");
		expect(text()).toContain("There is already an account named Reviewer for that agent.");
		await openPicker();
		expect(optionNames()).toContain("Reviewer");
	});

	test("an inline account is selected at once, and Sign in later leaves it Not signed in", async () => {
		await mount();
		await agent("claude-acp");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await add("Add subscription");
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]);
		expect(picker()!.textContent).toBe("Account 3 · Signing in…");
		expect(text()).toContain("A Claude Code window opened for Account 3.");
		expect(text()).toContain(NEVER_SEES);
		expect(button("Add teammate").disabled).toBe(true);
		const enterOnForm = new window.Event("submit", { bubbles: true, cancelable: true });
		await act(async () => {
			container.querySelector("form")!.dispatchEvent(enterOnForm);
		});
		await settle();
		expect(enterOnForm.defaultPrevented).toBe(true);
		expect(named("persona.create")).toHaveLength(0);
		expect(named("accounts.login.cancel")).toHaveLength(0);

		await click("Sign in later");
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a3", id: "l1" }]);
		expect(calls.findIndex(([cmd]) => cmd === "accounts.status")).toBeGreaterThan(calls.findIndex(([cmd]) => cmd === "accounts.login.cancel"));
		expect(picker()!.textContent).toBe("Account 3 · Not signed in");

		expect(button("Add teammate").disabled).toBe(false);
		await click("Add teammate");
		expect(draft().accountId).toBe("a3");
		expect(created).toEqual(["p1"]);
	});

	test("Enter in the new account's email adds the account, not the teammate", async () => {
		await mount();
		await agent("claude-acp");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await add("Add subscription");
		const email = container.querySelector<HTMLInputElement>('input[aria-label="Email of the Claude Code account (optional)"]')!;
		await type(email, "dev@example.com");
		expect(await enter(email)).toBe(true);
		expect(named("accounts.create")).toHaveLength(1);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a3", email: "dev@example.com" }]);
		expect(named("persona.create")).toHaveLength(0);
	});
});

describe("a new Hotline Agent teammate's account", () => {
	test("two keys of the model's provider show the field under the model, on the default; picking B sends B", async () => {
		await mount();
		expect(picker()).toBeNull();
		await model("Claude X");
		expect(labels().slice(-2)).toEqual(["Model", "Account"]);
		expect(picker()!.textContent).toBe("Key A · API key · pay per use · Default");
		await openPicker();
		expect(optionNames()).toEqual(["Key A", "Key B"]);
		await closePicker();
		await submit("Ada");
		expect(draft().modelId).toBe("anthropic/claude-x");
		expect("accountId" in draft()).toBe(false);

		await remount();
		await model("Claude X");
		await choose("Key B");
		expect(thinksWith().textContent).toContain("Hotline Agent · Claude X · Key B");
		await choose("Key A");
		await submit("Mack");
		expect("accountId" in draft()).toBe(false);

		await remount();
		await model("Claude X");
		await choose("Key B");
		await submit("Rae");
		expect(draft().accountId).toBe("B");
		expect(draft().modelId).toBe("anthropic/claude-x");
	});

	test("a model of another provider hides the field and drops the choice, and coming back starts on the default", async () => {
		await mount();
		await model("Claude X");
		await choose("Key B");
		await model("GPT X");
		expect(picker()).toBeNull();
		expect(thinksWith().textContent).not.toContain("Key B");
		await model("Claude X");
		expect(picker()!.textContent).toContain("Key A");
		await model("GPT X");
		await submit("Ada");
		expect(draft().modelId).toBe("openai/gpt-x");
		expect("accountId" in draft()).toBe(false);
	});

	test("Add API key opens the key form in place and selects the new key", async () => {
		await mount();
		await model("Claude X");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await add("Add API key");
		expect(container.querySelectorAll("form form")).toHaveLength(0);
		expect(text()).not.toContain(NEVER_SEES);
		const name = container.querySelector<HTMLInputElement>('input[placeholder="Anthropic"]')!;
		expect(name).not.toBeNull();
		await type(container.querySelector<HTMLInputElement>("#key-secret")!, "sk-three");
		await click("Save key");
		expect(calls).toContainEqual(["credential.create", { providerId: "anthropic", label: "Anthropic", secret: "sk-three" }]);
		expect(named("persona.create")).toHaveLength(0);
		expect(container.querySelector("#key-secret")).toBeNull();
		expect(picker()!.textContent).toBe("Anthropic · API key · pay per use");

		await click("Add teammate");
		expect(draft().accountId).toBe("C");
	});

	test("cancelling a new key selects nothing", async () => {
		await mount();
		await model("Claude X");
		await add("Add API key");
		await click("Cancel");
		expect(container.querySelector("#key-secret")).toBeNull();
		expect(named("credential.create")).toHaveLength(0);
		expect(picker()!.textContent).toBe("Key A · API key · pay per use · Default");
		await submit("Ada");
		expect("accountId" in draft()).toBe(false);
	});

	test("Enter in the new key's Name saves the key once, not the teammate", async () => {
		await mount();
		await model("Claude X");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await add("Add API key");
		await type(container.querySelector<HTMLInputElement>("#key-secret")!, "sk-three");
		const name = container.querySelector<HTMLInputElement>('input[placeholder="Anthropic"]')!;
		await type(name, "Spare");
		expect(await enter(name)).toBe(true);
		expect(named("credential.create")).toEqual([["credential.create", { providerId: "anthropic", label: "Spare", secret: "sk-three" }]]);
		expect(named("persona.create")).toHaveLength(0);
		expect(picker()!.textContent).toBe("Spare · API key · pay per use");
	});

	test("Enter in the new key saves the key, not the teammate", async () => {
		await mount();
		await model("Claude X");
		await type(container.querySelector<HTMLInputElement>("#new-name")!, "Ada");
		await add("Add API key");
		const secret = container.querySelector<HTMLInputElement>("#key-secret")!;
		await type(secret, "sk-three");
		expect(await enter(secret)).toBe(true);
		expect(named("credential.create")).toHaveLength(1);
		expect(named("persona.create")).toHaveLength(0);
	});
});
