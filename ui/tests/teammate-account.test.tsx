import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountView, Credential, CredentialAccount, Persona, Provider, SessionAccount, SessionInfo } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { Wire, wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

const agents = [
	{ backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true, providerId: "anthropic" },
	{ backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false },
];
const providers: Provider[] = [
	{ id: "anthropic", name: "Anthropic", credentialKinds: ["api_key"], modelDiscovery: false },
	{ id: "openai", name: "OpenAI", credentialKinds: ["api_key"], modelDiscovery: false },
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
const a2 = () => account("a2", "Account 2", { status: { state: "signed_in", checkedAt: 1, email: "reviewer@example.com", plan: "pro" } });
const a3 = () => account("a3", "Account 3");
const a4 = () => account("a4", "Account 4", { status: { state: "not_checked" } });
function key(id: string, label: string, providerId = "anthropic", isDefault = false): CredentialAccount {
	return {
		credential: { id, providerId, credentialKind: "api_key", label, revoked: false, createdAt: 1, updatedAt: 1 },
		section: "api_key",
		usedBy: [],
		default: isDefault,
	};
}

const ada: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "claude-acp", cwd: "/work/ada", accountId: "a1",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
};
function sessionOf(state: SessionInfo["state"], account?: SessionAccount, contextRestored = false, currentModelId?: string): SessionInfo {
	return {
		personaId: "ada", state, contextRestored, models: [], modes: [], configs: [], slashCommands: [],
		capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		...(account === undefined ? {} : { account }),
		...(currentModelId === undefined ? {} : { currentModelId }),
	};
}
const onA1 = sessionOf("ready", { running: "a1", wanted: "a1" });
const { accountId: _onA1, ...onDefault } = ada;
const onClaudeX: Persona = { ...onDefault, backendId: "hotline", modelId: "anthropic/claude-x" };

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let credentials: CredentialAccount[];
let poll: { state: "running" | "succeeded" | "failed"; output: string; error?: string };
let setupOpened: string[];
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
	if (cmd === "accounts.list") return { agents, accounts, credentials };
	if (cmd === "providers.list") return providers;
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
	if (cmd === "persona.account") return { running: "a1", ...(params["accountId"] ? { wanted: params["accountId"] } : {}) };
	if (cmd === "session.stop" || cmd === "session.start") return null;
	if (cmd === "accounts.create") {
		if (createRefusal !== null) throw new Error(createRefusal);
		if (!accounts.some((one) => one.account.id === "a4")) accounts = [...accounts, a4()];
		return { account: a4() };
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
		change(params["accountId"], (one) =>
			one.attempt?.state === "running" ? { ...one, attempt: { id: "l1", state: "failed", error: "Sign-in cancelled." }, status: { state: "checking" } } : one,
		);
		return null;
	}
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
	accounts = [a1(), a2(), a3()];
	credentials = [key("A", "Key A", "anthropic", true), key("B", "Key B"), key("O", "OpenAI key", "openai", true)];
	poll = { state: "running", output: "" };
	setupOpened = [];
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

async function mount(who: Persona, info: SessionInfo) {
	await act(async () => {
		root!.render(
			<Teammate
				persona={who}
				session={info}
				jobs={[]}
				roster={[]}
				focusSchedules={false}
				onClose={() => {}}
				onDeleted={() => {}}
				onOpenThread={() => {}}
				onAccountSetup={(id) => setupOpened.push(id)}
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
async function remount(who: Persona, info: SessionInfo) {
	await unmount();
	await act(async () => {
		root = (await import("react-dom/client")).createRoot(container);
	});
	await mount(who, info);
}
async function settle() {
	for (let round = 0; round < 6; round++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	}
}
const text = () => container.textContent ?? "";
const named = (prefix: string) => calls.filter(([cmd]) => cmd.startsWith(prefix));
const CLAUDE_CODE = "Claude Code account";
const ANTHROPIC = "Anthropic account";
const OPENAI = "OpenAI account";
function section(title = CLAUDE_CODE): HTMLElement {
	const group = container.querySelector<HTMLElement>(`[role="radiogroup"][aria-label="${title}"]`);
	if (!group) throw new Error(`No ${title} list: ${text()}`);
	return group;
}
const titles = (title = CLAUDE_CODE) => [...section(title).querySelectorAll(".group-row-title")].map((node) => node.textContent);
function row(name: string, title = CLAUDE_CODE): HTMLElement {
	const found = [...section(title).querySelectorAll(".group-row-title")].find((node) => node.textContent === name);
	if (!found) throw new Error(`Missing row ${name}: ${text()}`);
	return found.closest(".group-row") as HTMLElement;
}
const checked = (name: string, title = CLAUDE_CODE) => (row(name, title).querySelector("input") as HTMLInputElement).checked;
const lastAccount = () => named("persona.account").at(-1)?.[1];
const revoked = (one: CredentialAccount): CredentialAccount => ({ ...one, credential: { ...one.credential, revoked: true } });
async function pick(name: string, title = CLAUDE_CODE) {
	const radio = row(name, title).querySelector("input")!;
	await act(async () => {
		radio.click();
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
const NEVER_SEES = "Hotline never sees a password or token: each agent's CLI keeps its own sign-in.";

describe("a teammate's account section", () => {
	test("no section when fewer than two accounts fit: an agent without accounts, This computer alone, one key", async () => {
		await mount({ ...ada, backendId: "codex-acp" }, sessionOf("idle"));
		expect(calls.map(([cmd]) => cmd)).toContain("accounts.list");
		expect(text()).not.toContain("Codex account");
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();

		accounts = [];
		await remount(onDefault, sessionOf("ready"));
		expect(text()).not.toContain(CLAUDE_CODE);
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();

		await remount({ ...onClaudeX, modelId: "openai/gpt-x" }, sessionOf("ready"));
		expect(text()).not.toContain(OPENAI);
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
	});

	test("set to an account the core ignores, it starts on the default, so one account that fits is still nothing to pick", async () => {
		accounts = [];
		credentials = [...credentials, revoked(key("R", "Old key"))];
		// On Hotline Agent: another provider's live key, then another provider's signed-out key.
		await mount({ ...onClaudeX, modelId: "openai/gpt-x", accountId: "B" }, sessionOf("ready"));
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
		await remount({ ...onClaudeX, modelId: "openai/gpt-x", accountId: "R" }, sessionOf("ready"));
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
		// On a CLI agent: a key, signed out or live.
		await remount({ ...ada, accountId: "R" }, sessionOf("ready"));
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
		await remount({ ...ada, accountId: "B" }, sessionOf("ready"));
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
	});

	test("set to an account the core refuses, the one account that fits is offered unpicked", async () => {
		accounts = [];
		await mount({ ...ada, accountId: "gone" }, sessionOf("idle"));
		expect(titles()).toEqual(["This computer"]);
		expect(checked("This computer")).toBe(false);
		expect(text()).not.toContain("'s setup");
		await pick("This computer");
		expect(lastAccount()).toEqual({ personaId: "ada", accountId: null });

		await remount({ ...onClaudeX, modelId: "openai/gpt-x", accountId: "gone" }, sessionOf("idle"));
		expect(checked("OpenAI key", OPENAI)).toBe(false);
		await pick("OpenAI key", OPENAI);
		expect(lastAccount()).toEqual({ personaId: "ada", accountId: null });

		// A signed-out key of the model's own provider fails every turn.
		credentials = [...credentials, revoked(key("R", "Old OpenAI key", "openai"))];
		await remount({ ...onClaudeX, modelId: "openai/gpt-x", accountId: "R" }, sessionOf("idle"));
		expect(titles(OPENAI)).toEqual(["OpenAI key"]);
		expect(checked("OpenAI key", OPENAI)).toBe(false);
	});
});

describe("a Hotline Agent teammate's account", () => {
	test("its provider's accounts are radios, In use on the default, and picking B sends persona.account", async () => {
		await mount(onClaudeX, sessionOf("ready"));
		expect([...container.querySelectorAll("h3")].map((node) => node.textContent)).toContain(ANTHROPIC);
		expect(titles(ANTHROPIC)).toEqual(["Key A", "Key B"]);
		expect(row("Key A", ANTHROPIC).textContent).toContain("API key · pay per use");
		expect(row("Key A", ANTHROPIC).textContent).toContain("In use");
		expect(row("Key B", ANTHROPIC).textContent).not.toContain("In use");
		expect(checked("Key A", ANTHROPIC)).toBe(true);
		expect(text()).toContain("On Key A · Fresh: a new session on this account, handed this conversation.");
		expect(text()).not.toContain("'s setup");
		expect(text()).not.toContain("Add subscription");
		expect(button("Add API key")).toBeDefined();

		await pick("Key B", ANTHROPIC);
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "B" }]);
		expect(named("persona.account")).toHaveLength(1);
	});

	test("a pending switch offers Keep and Stop and switch now", async () => {
		await mount({ ...onClaudeX, accountId: "B" }, sessionOf("thinking", { wanted: "B" }));
		expect(text()).toContain("Switches to Key B when its work ends.");
		expect(text()).not.toContain("In use");
		expect(checked("Key B", ANTHROPIC)).toBe(true);

		await click("Keep Key A");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: null }]);
		await click("Stop and switch now");
		expect(named("session.").map(([cmd]) => cmd)).toEqual(["session.stop", "session.start"]);
		expect(named("session.").map(([, params]) => params)).toEqual([{ personaId: "ada" }, { personaId: "ada" }]);

		await remount({ ...onClaudeX, accountId: "B" }, sessionOf("ready", { running: "B", wanted: "B" }));
		expect(text()).not.toContain("Switches to");
		expect(row("Key B", ANTHROPIC).textContent).toContain("In use");
		expect(text()).toContain("On Key B · Fresh: a new session on this account, handed this conversation.");
	});

	test("picking the default's row after a detour follows the default again, and never pins it", async () => {
		await mount(onClaudeX, sessionOf("thinking"));
		await pick("Key B", ANTHROPIC);
		expect(lastAccount()).toEqual({ personaId: "ada", accountId: "B" });

		// What the roster and the session then say: set to B, switching once the work ends.
		await mount({ ...onClaudeX, accountId: "B" }, sessionOf("thinking", { wanted: "B" }));
		expect(text()).toContain("Switches to Key B when its work ends.");
		await pick("Key A", ANTHROPIC);
		expect(lastAccount()).toEqual({ personaId: "ada", accountId: null });
		expect(named("persona.account")).toHaveLength(2);

		// Back on no choice, nothing is pending: never a switch from Key A to Key A.
		await mount(onClaudeX, sessionOf("thinking"));
		expect(text()).not.toContain("Switches to");
		expect(text()).not.toContain("stays on");
		expect(checked("Key A", ANTHROPIC)).toBe(true);
		expect(row("Key A", ANTHROPIC).textContent).toContain("In use");
	});

	test("the accounts are those of the model the session runs now", async () => {
		credentials = [...credentials, key("P", "Personal OpenAI", "openai")];
		await mount(onClaudeX, sessionOf("ready", undefined, false, "openai/gpt-x"));
		expect(titles(OPENAI)).toEqual(["OpenAI key", "Personal OpenAI"]);
		expect(text()).not.toContain(ANTHROPIC);
	});

	test("set to a key of the provider its model left, the new provider's default is checked and in use", async () => {
		credentials = [...credentials, key("P", "Personal OpenAI", "openai")];
		await mount({ ...onClaudeX, accountId: "B" }, sessionOf("ready", { running: "B", wanted: "B" }, false, "openai/gpt-x"));
		expect(titles(OPENAI)).toEqual(["OpenAI key", "Personal OpenAI"]);
		expect(checked("OpenAI key", OPENAI)).toBe(true);
		expect(checked("Personal OpenAI", OPENAI)).toBe(false);
		expect(row("OpenAI key", OPENAI).textContent).toContain("In use");
		expect(text()).not.toContain("Switches to");
		expect(text()).toContain("On OpenAI key · Fresh");
	});

	test("a switch that lands on the account already running is never shown as pending", async () => {
		// The phone moved the model to another provider while a switch to B waited: both sides are now OpenAI's default.
		credentials = [...credentials, key("P", "Personal OpenAI", "openai")];
		await mount({ ...onClaudeX, accountId: "B" }, sessionOf("thinking", { running: "A", wanted: "B" }, false, "openai/gpt-x"));
		expect(text()).not.toContain("Switches to");
		expect(text()).not.toContain("stays on");
		expect(checked("OpenAI key", OPENAI)).toBe(true);

		// A wire caller pinned the default while the session runs on it unpinned.
		await remount({ ...onClaudeX, accountId: "A" }, sessionOf("thinking", { wanted: "A" }));
		expect(text()).not.toContain("Switches to");
		expect(text()).not.toContain("stays on");
		expect(row("Key A", ANTHROPIC).textContent).toContain("In use");
	});

	test("set to a signed-out key of its model's provider, no account says it is in use", async () => {
		credentials = [...credentials, revoked(key("R", "Old OpenAI key", "openai"))];
		await mount({ ...onClaudeX, modelId: "openai/gpt-x", accountId: "R" }, sessionOf("ready", { running: "R", wanted: "R" }));
		expect(titles(OPENAI)).toEqual(["OpenAI key"]);
		expect(checked("OpenAI key", OPENAI)).toBe(false);
		expect(text()).not.toContain("In use");
		expect(text()).not.toContain("On OpenAI key");
	});

	test("Keep is offered only when the running account fits the model", async () => {
		credentials = [...credentials, key("P", "Personal OpenAI", "openai")];
		await mount({ ...onClaudeX, accountId: "P" }, sessionOf("thinking", { running: "A", wanted: "P" }, false, "openai/gpt-x"));
		expect(text()).toContain("Switches to Personal OpenAI when its work ends.");
		expect(text()).toContain("What is running now stays on OpenAI key.");
		expect([...container.querySelectorAll("button")].some((node) => node.textContent?.startsWith("Keep"))).toBe(false);
		expect(button("Stop and switch now")).toBeDefined();
	});

	test("with no model of its own and no session, the section is for the room's preferred model", async () => {
		const room = [
			{ kind: "setting", id: "defaultModelId", value: "openai/gpt-x" },
			{ kind: "setting", id: "lastModelId", value: "anthropic/claude-x" },
		];
		const subscribe = spyOn(Wire.prototype, "subscribe").mockImplementation(((target: unknown, handlers: { snapshot(items: unknown[]): void }) => {
			if (target === "room") handlers.snapshot(room);
			return () => {};
		}) as typeof Wire.prototype.subscribe);
		restores.push(() => subscribe.mockRestore());
		credentials = [...credentials, key("P", "Personal OpenAI", "openai")];
		await mount({ ...onDefault, backendId: "hotline" }, sessionOf("idle"));
		expect(titles(OPENAI)).toEqual(["OpenAI key", "Personal OpenAI"]);
		expect(checked("OpenAI key", OPENAI)).toBe(true);
	});

	test("Add API key adds a key in place and moves the teammate onto it", async () => {
		await mount(onClaudeX, sessionOf("ready"));
		await click("Add API key");
		expect(text()).not.toContain(NEVER_SEES);
		await type(container.querySelector<HTMLInputElement>("#key-secret")!, "sk-three");
		await click("Save key");
		expect(calls).toContainEqual(["credential.create", { providerId: "anthropic", label: "Anthropic", secret: "sk-three" }]);
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "C" }]);
		expect(container.querySelector("#key-secret")).toBeNull();
		expect(row("Anthropic", ANTHROPIC)).toBeDefined();
	});
});

describe("a Claude Code teammate's account", () => {
	test("picking an account sends persona.account; This computer sends null", async () => {
		await mount(ada, onA1);
		expect([...container.querySelectorAll("h3")].map((node) => node.textContent)).toContain("Claude Code account");
		expect(row("This computer").textContent).toContain("Claude Code's own sign-in, as before");
		expect(row("Account 1").textContent).toContain("dev@example.com · Max");
		expect(row("Account 3").textContent).toContain("Not signed in · sign in to use it");
		expect(row("Account 3").querySelector(".text-warn")).not.toBeNull();
		expect((row("Account 1").querySelector("input") as HTMLInputElement).checked).toBe(true);

		await pick("Account 2");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a2" }]);
		await pick("This computer");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: null }]);
		expect(named("persona.account")).toHaveLength(2);
	});

	test("In use marks the running account, and the note says Restored or Fresh", async () => {
		await mount(ada, sessionOf("ready", { running: "a1", wanted: "a1" }, true));
		expect(row("Account 1").textContent).toContain("In use");
		expect(row("Account 2").textContent).not.toContain("In use");
		expect(text()).toContain("On Account 1 · Restored: Claude Code reopened its own session for this account.");

		await remount(ada, onA1);
		expect(text()).toContain(
			"On Account 1 · Fresh: a new Claude Code session for this account, handed this conversation. Another account's session is never reopened here.",
		);

		await remount(onDefault, sessionOf("ready"));
		expect(row("This computer").textContent).toContain("In use");
		expect(text()).toContain("On This computer · Fresh:");

		await remount(ada, sessionOf("idle"));
		expect(text()).not.toContain("In use");
		expect(text()).not.toContain("On Account 1 ·");
	});

	test("a pending switch offers Keep and Stop and switch now", async () => {
		await mount({ ...ada, accountId: "a2" }, sessionOf("thinking", { running: "a1", wanted: "a2" }));
		expect(text()).toContain("Switches to Account 2 when its work ends.");
		expect(text()).toContain("What is running now stays on Account 1. Stop and switch now stops all of Ada's work.");
		expect(text()).not.toContain("In use");
		expect((row("Account 2").querySelector("input") as HTMLInputElement).checked).toBe(true);

		await click("Keep Account 1");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a1" }]);

		await click("Stop and switch now");
		expect(named("session.").map(([cmd]) => cmd)).toEqual(["session.stop", "session.start"]);
		expect(named("session.").map(([, params]) => params)).toEqual([{ personaId: "ada" }, { personaId: "ada" }]);

		await remount(ada, sessionOf("thinking", { running: "a1", wanted: "a2" }));
		expect(text()).toContain("Switches to Account 2 when its work ends.");
		await remount(onDefault, sessionOf("thinking", { running: "a1" }));
		expect(text()).toContain("Switches to This computer when its work ends.");
		await remount(ada, sessionOf("thinking", { running: "a1", wanted: "a1" }));
		expect(text()).not.toContain("Switches to");
		expect(row("Account 1").textContent).toContain("In use");
	});

	test("a held switch keeps the current account", async () => {
		await mount({ ...ada, accountId: "a3" }, sessionOf("ready", { running: "a1", wanted: "a3", held: "not_signed_in" }));
		expect(text()).toContain("Account 3 is not signed in. Ada keeps Account 1 until it is.");
		expect(text()).not.toContain("Switches to");

		await remount({ ...ada, accountId: "a3" }, sessionOf("ready", { running: "a1", wanted: "a3", held: "signing_in" }));
		expect(text()).toContain("Account 3 is still signing in. Ada keeps Account 1 until it is.");
		expect(text()).not.toContain("Switches to");
	});

	test("a name refused as taken reads the accounts again, so the account it names is listed", async () => {
		await mount(ada, onA1);
		accounts = [...accounts, account("r1", "Reviewer")];
		createRefusal = "There is already an account named Reviewer for that agent.";
		await click("Add subscription");
		await click("Add and sign in");
		expect(text()).toContain("There is already an account named Reviewer for that agent.");
		expect(row("Reviewer")).toBeDefined();
	});

	test("an account added here is assigned only after its sign-in succeeds", async () => {
		await mount(ada, onA1);
		expect(text()).not.toContain(NEVER_SEES);
		await click("Add subscription");
		expect(text()).toContain("Starts with your Claude Code setup · Change or customize");
		expect(text()).toContain(NEVER_SEES);
		const name = container.querySelector<HTMLInputElement>('input[aria-label="New account name"]')!;
		expect(name.placeholder).toBe("Account 4");
		const email = container.querySelector<HTMLInputElement>('input[aria-label="Email of the Claude Code account (optional)"]')!;
		expect(email.placeholder).toBe("Email (optional)");

		poll = { state: "failed", output: "", error: "Claude Code closed before it was signed in." };
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 4", startWith: "system_default" }]);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a4" }]);
		expect(text()).toContain("Claude Code closed before it was signed in.");
		expect(text()).not.toContain("A Claude Code window opened");
		expect(named("persona.account")).toHaveLength(0);
		const listed = named("accounts.list").length;
		await click("Close");
		expect(named("persona.account")).toHaveLength(0);
		expect(named("accounts.list").length).toBeGreaterThan(listed);

		poll = { state: "succeeded", output: "" };
		await click("Add subscription");
		await click("Add and sign in");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a4" }]);
	});

	test("a cancelled sign-in leaves the new account as the core checked it after", async () => {
		await mount(ada, onA1);
		await click("Add subscription");
		await click("Add and sign in");
		expect(row("Account 4").textContent).toContain("Signing in…");
		const panel = container.querySelector('section[aria-label="Harness sign-in"]')!;
		const cancel = [...panel.querySelectorAll("button")].find((node) => node.textContent === "Cancel")!;
		await act(async () => {
			cancel.click();
		});
		await settle();
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a4", id: "l1" }]);
		expect(calls.findIndex(([cmd]) => cmd === "accounts.status")).toBeGreaterThan(calls.findIndex(([cmd]) => cmd === "accounts.login.cancel"));
		expect(row("Account 4").textContent).toContain("Not signed in · sign in to use it");
		expect(named("persona.account")).toHaveLength(0);
	});

	test("the setup link opens the account's setup", async () => {
		await mount(ada, onA1);
		expect(text()).toContain(
			"Account 1's setup · instructions, settings, skills, opened in your editor. It is shared by every teammate on Account 1.",
		);
		await click("Account 1's setup");
		expect(setupOpened).toEqual(["a1"]);

		await remount({ ...ada, accountId: "a9" }, onA1);
		expect(text()).not.toContain("'s setup");
	});
});
