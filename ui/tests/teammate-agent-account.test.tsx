import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountView, Persona, SessionAccount, SessionInfo } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

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
const a2 = () => account("a2", "Account 2", { status: { state: "signed_in", checkedAt: 1, email: "reviewer@example.com", plan: "pro" } });
const a3 = () => account("a3", "Account 3");
const a4 = () => account("a4", "Account 4");

const ada: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "claude-acp", cwd: "/work/ada", agentAccountId: "a1",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
};
function sessionOf(state: SessionInfo["state"], account?: SessionAccount, contextRestored = false): SessionInfo {
	return {
		personaId: "ada", state, contextRestored, models: [], modes: [], configs: [], slashCommands: [],
		capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		...(account === undefined ? {} : { account }),
	};
}
const onA1 = sessionOf("ready", { running: "a1", wanted: "a1" });
const { agentAccountId: _onA1, ...onDefault } = ada;

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let accounts: AccountView[];
let poll: { state: "running" | "succeeded" | "failed"; output: string; error?: string };
let setupOpened: string[];
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

function respond(cmd: string, params: Record<string, unknown>): unknown {
	if (cmd === "accounts.list") return { agents, accounts };
	if (cmd === "persona.account") return { running: "a1", ...(params["accountId"] ? { wanted: params["accountId"] } : {}) };
	if (cmd === "session.stop" || cmd === "session.start") return null;
	if (cmd === "accounts.create") {
		if (!accounts.some((one) => one.account.id === "a4")) accounts = [...accounts, a4()];
		return { account: a4() };
	}
	if (cmd === "accounts.login.start") return { id: "l1" };
	if (cmd === "accounts.login.poll") return poll;
	if (cmd === "accounts.login.cancel") return null;
	// The rest of the pane reads a refusal as nothing to show.
	throw new Error(`Not in this test: ${cmd}`);
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ ResizeObserver: NoResize }));
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { platform: "windows" };
	// A desk with no endpoint: the room's wire never opens, so the pane reads no settings.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	accounts = [a1(), a2(), a3()];
	poll = { state: "running", output: "" };
	setupOpened = [];
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
function section(): HTMLElement {
	const group = container.querySelector<HTMLElement>('[role="radiogroup"][aria-label="Claude Code account"]');
	if (!group) throw new Error(`No account list: ${text()}`);
	return group;
}
function row(name: string): HTMLElement {
	const title = [...section().querySelectorAll(".group-row-title")].find((node) => node.textContent === name);
	if (!title) throw new Error(`Missing row ${name}: ${text()}`);
	return title.closest(".group-row") as HTMLElement;
}
async function pick(name: string) {
	const radio = row(name).querySelector("input")!;
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

describe("a teammate's agent account", () => {
	test("no account section for Hotline Agent or an agent without accounts", async () => {
		await mount({ ...ada, backendId: "hotline" }, sessionOf("idle"));
		expect(calls.map(([cmd]) => cmd)).not.toContain("accounts.list");
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();

		await remount({ ...ada, backendId: "codex-acp" }, sessionOf("idle"));
		expect(calls.map(([cmd]) => cmd)).toContain("accounts.list");
		expect(text()).not.toContain("Codex account");
		expect(container.querySelector('[role="radiogroup"]')).toBeNull();
	});

	test("picking an account sends persona.account; System default sends null", async () => {
		await mount(ada, onA1);
		expect([...container.querySelectorAll("h3")].map((node) => node.textContent)).toContain("Claude Code account");
		expect(row("System default").textContent).toContain("Claude Code's own sign-in on this computer, as before");
		expect(row("Account 1").textContent).toContain("dev@example.com · Max");
		expect(row("Account 3").textContent).toContain("Not signed in · it will not start until it is");
		expect(row("Account 3").querySelector(".text-warn")).not.toBeNull();
		expect((row("Account 1").querySelector("input") as HTMLInputElement).checked).toBe(true);

		await pick("Account 2");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a2" }]);
		await pick("System default");
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

		// A session on System default names no account, and its row is the one in use.
		await remount(onDefault, sessionOf("ready"));
		expect(row("System default").textContent).toContain("In use");
		expect(text()).toContain("On System default · Fresh:");

		// A teammate at rest has nothing in use and no note.
		await remount(ada, sessionOf("idle"));
		expect(text()).not.toContain("In use");
		expect(text()).not.toContain("On Account 1 ·");
	});

	test("a pending switch offers Keep and Stop and switch now", async () => {
		await mount({ ...ada, agentAccountId: "a2" }, sessionOf("thinking", { running: "a1", wanted: "a2" }));
		expect(text()).toContain("Switches to Account 2 when its work ends.");
		expect(text()).toContain("What is running now stays on Account 1. Stop and switch now stops all of Ada's work.");
		expect(text()).not.toContain("In use");
		expect((row("Account 2").querySelector("input") as HTMLInputElement).checked).toBe(true);

		await click("Keep Account 1");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a1" }]);

		await click("Stop and switch now");
		expect(named("session.").map(([cmd]) => cmd)).toEqual(["session.stop", "session.start"]);
		expect(named("session.").map(([, params]) => params)).toEqual([{ personaId: "ada" }, { personaId: "ada" }]);
	});

	test("a held switch keeps the current account", async () => {
		await mount({ ...ada, agentAccountId: "a3" }, sessionOf("ready", { running: "a1", wanted: "a3", held: "not_signed_in" }));
		expect(text()).toContain("Account 3 is not signed in. Ada keeps Account 1 until it is.");
		expect(text()).not.toContain("Switches to");

		await remount({ ...ada, agentAccountId: "a3" }, sessionOf("ready", { running: "a1", wanted: "a3", held: "signing_in" }));
		expect(text()).toContain("Account 3 is still signing in. Ada keeps Account 1 until it is.");
		expect(text()).not.toContain("Switches to");
	});

	test("an account added here is assigned only after its sign-in succeeds", async () => {
		await mount(ada, onA1);
		await click("Add account");
		expect(text()).toContain("Starts with your Claude Code setup · Change or customize");
		const name = container.querySelector<HTMLInputElement>('input[aria-label="New account name"]')!;
		expect(name.placeholder).toBe("Account 4");
		const email = container.querySelector<HTMLInputElement>('input[aria-label="Email of the Claude Code account (optional)"]')!;
		expect(email.placeholder).toBe("Email (optional)");

		poll = { state: "failed", output: "", error: "Claude Code closed before it was signed in." };
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 4", startWith: "system_default" }]);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a4" }]);
		expect(text()).toContain("Claude Code closed before it was signed in.");
		expect(named("persona.account")).toHaveLength(0);
		const listed = named("accounts.list").length;
		await click("Close");
		expect(named("persona.account")).toHaveLength(0);
		expect(named("accounts.list").length).toBeGreaterThan(listed);

		poll = { state: "succeeded", output: "" };
		await click("Add account");
		await click("Add and sign in");
		expect(calls).toContainEqual(["persona.account", { personaId: "ada", accountId: "a4" }]);
	});

	test("the setup link opens the account's setup", async () => {
		await mount(ada, onA1);
		expect(text()).toContain(
			"Account 1's setup · instructions, settings, skills, opened in your editor. It is shared by every teammate on Account 1.",
		);
		await click("Account 1's setup");
		expect(setupOpened).toEqual(["a1"]);
	});
});
