import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { AccountAgent, AccountStatusState, AccountView, CredentialAccount, Provider, RosterEntry } from "../src/generated/contract";
import { installDom } from "./dom";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");
const { ProvidersSection } = await import("../src/components/settings/Providers");
const { accountJob } = await import("../src/components/accounts/AccountSignIn");
const { nextAccountName } = await import("../src/components/accounts/words");
const { ConnectProvider } = await import("../src/components/ConnectProvider");
Reflect.deleteProperty(globalThis, "localStorage");

const epoch = 1_800_000_000_000;
const day = 86_400_000;
const claudeCode: AccountAgent = { backendId: "claude-acp", name: "Claude Code", supportsAccounts: true, takesEmail: true, providerId: "anthropic" };
const codex: AccountAgent = { backendId: "codex-acp", name: "Codex", supportsAccounts: false, takesEmail: false };
const providers: Provider[] = [
	{ id: "anthropic", name: "Anthropic", credentialKinds: ["api_key"], modelDiscovery: true },
	{ id: "github-copilot", name: "GitHub Copilot", credentialKinds: ["oauth"], modelDiscovery: true },
	{ id: "ollama", name: "Ollama Local", credentialKinds: ["local"], modelDiscovery: true },
	{ id: "openai", name: "OpenAI", credentialKinds: ["api_key"], modelDiscovery: true },
	{ id: "openrouter", name: "OpenRouter", credentialKinds: ["oauth", "api_key"], modelDiscovery: true },
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

function credential(id: string, label: string, extra: Partial<CredentialAccount> = {}, createdAt = epoch): CredentialAccount {
	return {
		credential: { id, providerId: "anthropic", credentialKind: "api_key", label, revoked: false, createdAt, updatedAt: createdAt },
		section: "api_key",
		usedBy: [],
		default: false,
		...extra,
	};
}
function login(id: string, label: string, revoked = false): CredentialAccount {
	const made = credential(id, label, { section: "subscription" });
	return { ...made, credential: { ...made.credential, providerId: "github-copilot", credentialKind: "oauth", revoked } };
}
function server(id: string, label: string, baseUrl: string): CredentialAccount {
	const made = credential(id, label, { section: "server" });
	return { ...made, credential: { ...made.credential, providerId: "ollama", credentialKind: "local", baseUrl } };
}
const keys = () => [credential("k1", "Prod key", { default: true, usedBy: ["rae"] }), credential("k2", "Personal key")];

function entry(id: string, name: string, accountId?: string, backendId = "claude-acp"): RosterEntry {
	return {
		persona: {
			id, name, goal: "", backendId, cwd: "/work",
			mcpPolicy: { mode: "all", serverIds: [] }, skillPolicy: { mode: "all", names: [] },
			backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 0, updatedAt: 0,
			...(backendId === "hotline" ? { modelId: "anthropic/claude-sonnet" } : {}),
			...(accountId === undefined ? {} : { accountId }),
		},
		session: {
			personaId: id, state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
			capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
		},
		waiting: false,
		drawing: false,
	} as RosterEntry;
}
const roster = [entry("ada", "Ada", "a1"), entry("mack", "Mack"), entry("rae", "Rae", undefined, "hotline")];

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = [string, Record<string, unknown>];
let calls: Call[];
let agents: AccountAgent[];
let accounts: AccountView[];
let credentials: CredentialAccount[];
let made: AccountView;
let poll: { state: "running" | "succeeded" | "failed"; output: string; error?: string };
let checked: AccountView["status"];
let cancelLands: boolean;
let createRefusal: string | null;
let deleteRefusal: string | null;
let updateRefusal: string | null;
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
	if (cmd === "accounts.list") return { agents, accounts, credentials };
	if (cmd === "providers.list") return providers;
	if (cmd === "capabilities.options") throw new Error("Not on this desk.");
	if (cmd === "models.catalog" || cmd === "credential.refresh_models") return [];
	if (cmd === "credential.create") {
		const created = credential(`k${credentials.length + 1}`, String(params["label"]));
		credentials = [...credentials, created];
		return created.credential;
	}
	if (cmd === "credential.update") {
		if (updateRefusal !== null) throw new Error(updateRefusal);
		const { id, ...edit } = params;
		credentials = credentials.map((one) => (one.credential.id === id ? { ...one, credential: { ...one.credential, ...edit } } : one));
		return credentials.find((one) => one.credential.id === id)?.credential;
	}
	if (cmd === "credential.connect_local") {
		const connected = server(`o${credentials.length + 1}`, String(params["label"]), String(params["baseUrl"]));
		credentials = [...credentials, connected];
		return connected.credential;
	}
	if (cmd === "credential.delete") {
		if (deleteRefusal !== null) throw new Error(deleteRefusal);
		credentials = credentials.filter((one) => one.credential.id !== params["id"]);
		return null;
	}
	if (cmd === "credential.login") return { loginId: "c-login", userCode: "ABCD-1234", verificationUri: "https://example.com/device" };
	if (cmd === "credential.login_status") return { state: "pending" };
	if (cmd === "credential.login_cancel") return null;
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
	agents = [claudeCode, codex];
	accounts = [a1(), a2()];
	credentials = keys();
	made = a3();
	poll = { state: "running", output: "" };
	checked = { state: "signed_in", checkedAt: Date.now(), email: "two@example.com", plan: "pro" };
	cancelLands = true;
	createRefusal = null;
	deleteRefusal = null;
	updateRefusal = null;
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

async function mount(opening?: string) {
	await act(async () => {
		root!.render(<ProvidersSection enabledModels={{}} roster={roster} {...(opening === undefined ? {} : { opening })} />);
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
		const { createRoot } = await import("react-dom/client");
		root = createRoot(container);
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
const band = () => container.querySelector(".band h2")?.textContent;
const named = (cmd: string) => calls.filter(([one]) => one === cmd);
const titles = () => [...container.querySelectorAll(".group-title")].map((node) => node.textContent);
const shortDay = (at: number) => new Date(at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
function rows(name: string): HTMLElement[] {
	return [...container.querySelectorAll(".group-row-title")].filter((node) => node.textContent === name).map((node) => node.closest(".group-row") as HTMLElement);
}
function row(name: string): HTMLElement {
	const found = rows(name)[0];
	if (!found) throw new Error(`Missing row ${name}: ${text()}`);
	return found;
}
function button(label: string, within: ParentNode = container) {
	const found = [...within.querySelectorAll("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found as HTMLButtonElement;
}
const hasButton = (label: string, within: ParentNode = container) => [...within.querySelectorAll("button")].some((node) => node.textContent === label);
async function click(label: string, within?: ParentNode) {
	const found = button(label, within);
	await act(async () => {
		found.click();
	});
	await settle();
}
async function press(element: HTMLElement) {
	await act(async () => {
		element.click();
	});
	await settle();
}
const openProvider = (name: string) => press(row(name));
async function openAccount(name: string) {
	await press(row(name).querySelector("button")!);
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
async function key(element: HTMLElement, name: string) {
	await act(async () => {
		element.dispatchEvent(new window.KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true }));
	});
	await settle();
}
function asked(question: string): HTMLElement {
	const found = [...container.querySelectorAll("p")].find((node) => node.textContent === question);
	if (!found) throw new Error(`Missing question ${question}: ${text()}`);
	return found.closest(".group-row") as HTMLElement;
}
const hasLabel = (name: string) => [...container.querySelectorAll("label")].some((node) => node.textContent === name);
async function tick(title: string) {
	const found = [...container.querySelectorAll("label")].find((node) => node.querySelector("input") && node.textContent?.startsWith(title));
	if (!found) throw new Error(`Missing choice ${title}: ${text()}`);
	await act(async () => {
		found.querySelector("input")!.click();
	});
}

describe("Settings › Providers", () => {
	test("a provider row counts its subscriptions and keys", async () => {
		credentials = [...keys(), server("o1", "Ollama Local", "http://localhost:11434")];
		await mount();
		expect(named("accounts.list")).toHaveLength(1);
		expect(band()).toBe("Providers");
		expect(row("Anthropic").textContent).toContain("2 subscriptions · 2 API keys · All models");
		expect(row("Ollama Local").textContent).toContain("1 server · All models");
		expect(text()).not.toContain("Codex");
		await openProvider("Anthropic");
		expect(band()).toBe("Anthropic");
		expect(container.querySelector(".nt-card")?.textContent).toContain("2 subscriptions · 2 API keys");
	});

	test("a provider whose accounts are all signed out says so", async () => {
		agents = [codex];
		accounts = [];
		credentials = [login("c1", "GitHub Copilot", true)];
		await mount();
		expect(row("GitHub Copilot").textContent).toContain("Signed out");
		await click("Add provider");
		expect(rows("GitHub Copilot")).toHaveLength(1);
	});

	test("the provider page lists every way in, with who uses each and the default", async () => {
		credentials = [...keys(), server("o1", "Ollama Local", "http://localhost:11434")];
		await mount();
		await openProvider("Anthropic");
		expect(titles()).toEqual(["Subscriptions", "API keys", "Models in the picker"]);
		expect(row("This computer").textContent).toContain("Built in");
		expect(row("Account 1").textContent).toContain("Used by Ada");
		expect(row("Prod key").textContent).toContain("API key · pay per use · Default · Used by Rae");
		expect(row("Personal key").textContent).toContain("API key · pay per use · Not used yet");
		expect(row("Personal key").textContent).not.toContain("Default");
		expect(hasButton("Add subscription")).toBe(true);
		expect(hasButton("Add API key")).toBe(true);
		expect(text()).toContain("A teammate on Anthropic picks one of these when it has more than one to choose from.");

		await click("Back to the team");
		await openProvider("Ollama Local");
		expect(titles()).toEqual(["Servers", "Models in the picker"]);
		expect(row("Ollama Local").textContent).toContain("http://localhost:11434 · Not used yet");
		expect(row("Ollama Local").textContent).not.toContain("Default");
		expect(hasButton("Change server", row("Ollama Local"))).toBe(true);
		expect(hasButton("Disconnect", row("Ollama Local"))).toBe(true);
		expect(hasButton("Add server")).toBe(true);
	});

	test("adding a key with no name takes the next free one", async () => {
		credentials = [credential("k1", "Anthropic", { default: true }), credential("k2", "Anthropic", {}, epoch + 3 * day)];
		await mount();
		await openProvider("Anthropic");
		await click("Add API key");
		expect(field("Name").placeholder).toBe("Anthropic 2");
		expect(field("API key").type).toBe("password");
		await type(field("API key"), "sk-three");
		await click("Save key");
		expect(calls).toContainEqual(["credential.create", { providerId: "anthropic", label: "Anthropic 2", secret: "sk-three" }]);
		expect(rows("Anthropic 2")).toHaveLength(1);
		expect(hasButton("Save key")).toBe(false);
	});

	test("two old keys with one name are told apart by date", async () => {
		credentials = [credential("k1", "Anthropic", { default: true }), credential("k2", "Anthropic", {}, epoch + 3 * day), credential("k3", "Spare")];
		await mount();
		await openProvider("Anthropic");
		const [first, second] = rows("Anthropic").map((one) => one.textContent ?? "");
		expect(first).toContain(`Added ${shortDay(epoch)}`);
		expect(second).toContain(`Added ${shortDay(epoch + 3 * day)}`);
		expect(row("Spare").textContent).not.toContain("Added");
	});

	test("change key updates the same credential", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Change key", row("Prod key"));
		expect(hasLabel("Name")).toBe(false);
		expect(field("API key").type).toBe("password");
		await type(field("API key"), "sk-new");
		await click("Save key");
		expect(calls).toContainEqual(["credential.update", { id: "k1", secret: "sk-new" }]);
		expect(named("credential.create")).toHaveLength(0);
		expect(named("credential.delete")).toHaveLength(0);
		expect(row("Prod key").textContent).toContain("Default");
	});

	test("a row renames its account in place", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Rename", row("Personal key"));
		await type(field("Account name"), "  Spare key ");
		await click("Save");
		expect(calls).toContainEqual(["credential.update", { id: "k2", label: "Spare key" }]);
		expect(row("Spare key")).toBeDefined();

		await click("Rename", row("Account 2"));
		await type(field("Account name"), "Reviewer seat");
		await click("Save");
		expect(calls).toContainEqual(["accounts.rename", { accountId: "a2", name: "Reviewer seat" }]);
		expect(row("Reviewer seat")).toBeDefined();
		expect(band()).toBe("Anthropic");
	});

	test("remove refused names its teammates", async () => {
		deleteRefusal = "Prod key can't be removed while Rae uses it.";
		await mount();
		await openProvider("Anthropic");
		await click("Remove key", row("Prod key"));
		await click("Remove key", asked("Remove Prod key?"));
		expect(calls).toContainEqual(["credential.delete", { id: "k1" }]);
		expect(text()).toContain("Prod key can't be removed while Rae uses it.");
		expect(row("Prod key")).toBeDefined();
		expect(text()).not.toContain("Remove Prod key?");
		// Said where the remove was pressed, not past the models.
		const refusal = container.querySelector(".refusal")!;
		const models = [...container.querySelectorAll(".group-title")].find((node) => node.textContent === "Models in the picker")!;
		expect(refusal.compareDocumentPosition(models) & window.Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
	});

	test("remove key asks once before anything is lost", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Remove key", row("Personal key"));
		expect(text()).toContain("Remove Personal key?");
		expect(text()).toContain("Hotline forgets this key. It still works at Anthropic until you revoke it there.");
		expect(hasButton("Rename", row("Personal key"))).toBe(false);
		expect(named("credential.delete")).toHaveLength(0);
		await click("Cancel");
		expect(named("credential.delete")).toHaveLength(0);
		expect(text()).not.toContain("Remove Personal key?");

		await click("Remove key", row("Personal key"));
		await click("Remove key", asked("Remove Personal key?"));
		expect(calls).toContainEqual(["credential.delete", { id: "k2" }]);
		expect(rows("Personal key")).toHaveLength(0);
	});

	test("a remove's question takes the focus to Cancel, and Escape puts it away", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Remove key", row("Personal key"));
		const cancel = button("Cancel", asked("Remove Personal key?"));
		expect(document.activeElement === cancel).toBe(true);
		await key(cancel, "Escape");
		expect(text()).not.toContain("Remove Personal key?");
		expect(hasButton("Rename", row("Personal key"))).toBe(true);
		expect(named("credential.delete")).toHaveLength(0);

		await openAccount("Account 2");
		await click("Remove account");
		const cancelRemove = button("Cancel", asked("Remove Account 2?"));
		expect(document.activeElement === cancelRemove).toBe(true);
		await key(cancelRemove, "Escape");
		expect(text()).not.toContain("Remove Account 2?");
		expect(band()).toBe("Account 2");
		expect(named("accounts.remove")).toHaveLength(0);
	});

	test("a refusal goes once a later rename succeeds", async () => {
		deleteRefusal = "Prod key can't be removed while Rae uses it.";
		await mount();
		await openProvider("Anthropic");
		await click("Remove key", row("Prod key"));
		await click("Remove key", asked("Remove Prod key?"));
		expect(text()).toContain("Prod key can't be removed while Rae uses it.");

		await click("Rename", row("Personal key"));
		await type(field("Account name"), "Spare key");
		await click("Save");
		expect(row("Spare key")).toBeDefined();
		expect(text()).not.toContain("can't be removed");
	});

	test("a remove's question goes when its account is signed out meanwhile", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Remove key", row("Personal key"));
		expect(text()).toContain("Remove Personal key?");

		// Signed out elsewhere; the page learns it when it next reads the list.
		credentials = credentials.map((one) => (one.credential.id === "k2" ? { ...one, credential: { ...one.credential, revoked: true } } : one));
		await click("Rename", row("Prod key"));
		await type(field("Account name"), "Main key");
		await click("Save");
		expect(row("Personal key").textContent).toContain("Signed out");
		expect(text()).not.toContain("Remove Personal key?");
		expect([...row("Personal key").querySelectorAll("button")].map((node) => node.textContent)).toEqual(["Remove"]);
	});

	test("change server keeps the server and starts from its address", async () => {
		agents = [codex];
		accounts = [];
		credentials = [server("o1", "Ollama Local", "http://192.168.1.5:11434")];
		await mount();
		await openProvider("Ollama Local");
		await click("Change server", row("Ollama Local"));
		expect(hasLabel("Name")).toBe(false);
		expect(field("Server URL").value).toBe("http://192.168.1.5:11434");
		await type(field("Server URL"), "http://192.168.1.9:11434");
		await click("Connect");
		expect(calls).toContainEqual(["credential.update", { id: "o1", baseUrl: "http://192.168.1.9:11434" }]);
		expect(named("credential.connect_local")).toHaveLength(0);
		expect(row("Ollama Local").textContent).toContain("http://192.168.1.9:11434");

		await click("Disconnect", row("Ollama Local"));
		expect(text()).toContain("Disconnect Ollama Local?");
		expect(named("credential.delete")).toHaveLength(0);
	});

	test("Enter in a new server's Name or its Server URL connects it, once", async () => {
		agents = [codex];
		accounts = [];
		credentials = [server("o1", "Ollama Local", "http://192.168.1.5:11434")];
		await mount();
		await openProvider("Ollama Local");
		await click("Add server");
		await type(field("Name"), "Lab");
		await key(field("Name"), "Enter");
		expect(named("credential.connect_local")).toEqual([["credential.connect_local", { baseUrl: "http://localhost:11434", label: "Lab" }]]);
		expect(rows("Lab")).toHaveLength(1);

		await click("Add server");
		await key(field("Server URL"), "Enter");
		expect(named("credential.connect_local")).toHaveLength(2);
		expect(named("credential.connect_local")[1]).toEqual(["credential.connect_local", { baseUrl: "http://localhost:11434", label: "Ollama Local 2" }]);
	});

	test("a refused rename keeps the old name and says why on the row", async () => {
		updateRefusal = "Anthropic already has an account named Personal key.";
		await mount();
		await openProvider("Anthropic");
		await click("Rename", row("Prod key"));
		await type(field("Account name"), "Personal key");
		await click("Save");
		expect(calls).toContainEqual(["credential.update", { id: "k1", label: "Personal key" }]);
		const input = field("Account name");
		expect(input.value).toBe("Personal key");
		expect(input.closest(".group-row")?.textContent).toContain("Anthropic already has an account named Personal key.");
		await key(input, "Escape");
		expect(row("Prod key").textContent).toContain("Default");
		expect(rows("Personal key")).toHaveLength(1);
	});

	test("an OpenRouter sign-in is a key billed per use, signed in again or signed out", async () => {
		agents = [codex];
		accounts = [];
		const made = login("r1", "OpenRouter");
		credentials = [{ ...made, section: "api_key", credential: { ...made.credential, providerId: "openrouter" } }];
		await mount();
		await openProvider("OpenRouter");
		expect(titles()).toEqual(["API keys", "Models in the picker"]);
		expect(row("OpenRouter").textContent).toContain("Signed in with OpenRouter · pay per use · Not used yet");
		expect([...row("OpenRouter").querySelectorAll("button")].map((node) => node.textContent)).toEqual(["Rename", "Sign in again", "Sign out"]);
		await click("Add API key");
		expect(hasButton("Sign in")).toBe(true);
		expect(hasButton("API key")).toBe(true);
	});

	test("a provider with only Claude Code accounts has no models to pick", async () => {
		credentials = [];
		await mount();
		await openProvider("Anthropic");
		expect(titles()).toEqual(["Subscriptions", "API keys"]);
		expect(text()).toContain("Hotline never sees a password or token: each agent's CLI keeps its own sign-in.");
	});

	test("a sign-in again keeps its account, and a signed-out one offers only Remove", async () => {
		agents = [codex];
		accounts = [];
		credentials = [login("c1", "GitHub Copilot"), login("c2", "Work Copilot", true)];
		await mount();
		await openProvider("GitHub Copilot");
		expect(row("GitHub Copilot").textContent).toContain("Signed in · Not used yet");
		expect(row("Work Copilot").textContent).toContain("Signed out");
		expect([...row("Work Copilot").querySelectorAll("button")].map((node) => node.textContent)).toEqual(["Remove"]);
		await click("Sign in again", row("GitHub Copilot"));
		expect(calls).toContainEqual(["credential.login", { providerId: "github-copilot", id: "c1" }]);
		expect(text()).toContain("ABCD-1234");
		await click("Cancel");
		expect(calls).toContainEqual(["credential.login_cancel", { loginId: "c-login" }]);

		await click("Remove", row("Work Copilot"));
		expect(calls).toContainEqual(["credential.delete", { id: "c2" }]);
		expect(rows("Work Copilot")).toHaveLength(0);
	});

	test("a custom connection is a row and a page of its own, with no sections", async () => {
		agents = [codex];
		accounts = [];
		const made = credential("x1", "LM Studio");
		credentials = [
			{
				...made,
				credential: { ...made.credential, providerId: "custom-x1", baseUrl: "http://localhost:1234/v1", custom: { api: "chat_completions", models: ["qwen"] } },
			},
		];
		await mount();
		expect(row("LM Studio").textContent).toContain("Chat Completions · All models");
		await openProvider("LM Studio");
		expect(band()).toBe("LM Studio");
		expect(container.querySelector(".nt-card")?.textContent).toContain("Chat Completions · http://localhost:1234/v1");
		expect(titles()).toEqual(["Models in the picker"]);
		await click("Remove connection");
		expect(text()).toContain("Remove LM Studio?");
		expect(named("credential.delete")).toHaveLength(0);
		await click("Remove connection", asked("Remove LM Studio?"));
		expect(calls).toContainEqual(["credential.delete", { id: "x1" }]);
		expect(band()).toBe("Providers");
	});

	test("adding anthropic asks which way in when Claude Code takes accounts", async () => {
		accounts = [];
		credentials = [];
		made = account("a9", "Account 1", { status: { state: "not_checked" } });
		await mount();
		await click("Add provider");
		await press(row("Anthropic"));
		expect(hasButton("Claude subscription")).toBe(true);
		expect(hasButton("API key")).toBe(true);
		await click("Claude subscription");
		expect(field("Name").placeholder).toBe("Account 1");
		expect(text()).toContain("Hotline never sees a password or token: each agent's CLI keeps its own sign-in.");
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 1", startWith: "system_default" }]);
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a9" }]);
		expect(band()).toBe("Anthropic");
		expect(row("Account 1").textContent).toContain("Signing in…");

		agents = [codex];
		await remount();
		await click("Add provider");
		await press(row("Anthropic"));
		expect(hasButton("Claude subscription")).toBe(false);
		expect(field("API key")).toBeDefined();
		expect(field("Name").placeholder).toBe("Anthropic");
	});
});

describe("Settings › Providers › Anthropic: Claude Code accounts", () => {
	test("the page shows This computer and each account's line", async () => {
		await mount();
		await openProvider("Anthropic");
		expect(text()).toContain("Claude Code's own sign-in, as before · Used by Mack");
		expect(row("This computer").textContent).toContain("Built in");
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
		await openProvider("Anthropic");
		for (const [index, [state, said]] of words.entries()) {
			const line = row(`Account ${index + 1}`);
			expect(line.textContent).toContain(said);
			expect(line.querySelector(".agent-account-dot")?.getAttribute("data-state") ?? null).toBe(state === "not_checked" ? null : state);
			expect(hasButton("Check", line)).toBe(state === "not_checked" || state === "unavailable");
		}
		expect(row("Account 1").querySelector(".agent-account-dot")).toBeNull();
		await click("Check", row("Account 1"));
		expect(calls).toContainEqual(["accounts.status", { accountId: "s0" }]);
		expect(row("Account 1").textContent).toContain("Signed in · two@example.com");
	});

	test("a blank name adds Account 3 and a Windows sign-in shows only Cancel", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Add subscription");
		expect(field("Name").placeholder).toBe("Account 3");
		expect(titles()).toContain("What a new account starts with");
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

	test("a Windows sign-in started under StrictMode's double mount starts once and is never cancelled by it", async () => {
		const { StrictMode } = await import("react");
		await act(async () => {
			root!.render(
				<StrictMode>
					<ProvidersSection enabledModels={{}} roster={roster} />
				</StrictMode>,
			);
		});
		await settle();
		await openProvider("Anthropic");
		await click("Add subscription");
		await click("Add and sign in");
		await settle();
		expect(calls.filter(([cmd]) => cmd === "accounts.login.start")).toEqual([["accounts.login.start", { accountId: "a3" }]]);
		expect(calls.some(([cmd]) => cmd === "accounts.login.cancel")).toBe(false);
		expect(text()).toContain("A Claude Code window opened for Account 3. Finish signing in there; Hotline checks when it closes.");
	});

	test("a name refused as taken reads the accounts again, so the account it names is listed", async () => {
		await mount();
		await openProvider("Anthropic");
		accounts = [...accounts, account("r1", "Reviewer")];
		createRefusal = "There is already an account named Reviewer for that agent.";
		await click("Add subscription");
		await type(field("Name"), "Reviewer");
		await click("Add and sign in");
		expect(text()).toContain("There is already an account named Reviewer for that agent.");
		expect(row("Reviewer")).toBeDefined();
	});

	test("Back mid sign-in leaves the row as the cancelled attempt ended", async () => {
		checked = { state: "signed_out", checkedAt: Date.now() };
		await mount();
		await openProvider("Anthropic");
		await openAccount("Account 2");
		await click("Sign in");
		expect(calls).toContainEqual(["accounts.login.start", { accountId: "a2" }]);
		await click("Back to the team");
		expect(band()).toBe("Anthropic");
		expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a2", id: "l1" }]);
		expect(row("Account 2").textContent).toContain("Sign-in did not finish: Sign-in cancelled.");
		expect(row("Account 2").textContent).not.toContain("Signing in…");
		expect(button("Try again", row("Account 2"))).toBeDefined();
	});

	test("a list that still says signing in after a cancel is read again", async () => {
		checked = { state: "signed_out", checkedAt: Date.now() };
		cancelLands = false;
		await mount();
		await openProvider("Anthropic");
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
		await openProvider("Anthropic");
		await openAccount("Account 2");
		expect(container.querySelector(".nt-card")?.textContent).toContain("Signing in…");
		expect(hasButton("Sign in")).toBe(false);
		expect(hasButton("Check")).toBe(false);
	});

	test("an owner seat adds an account and is told to sign it in from the desk", async () => {
		made = { ...a3(), canSignInHere: false };
		await mount();
		await openProvider("Anthropic");
		await click("Add subscription");
		await click("Add and sign in");
		expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]);
		expect(named("accounts.login.start")).toHaveLength(0);
		expect(container.querySelector('section[aria-label="Harness sign-in"]')).toBeNull();
		expect(text()).toContain("Sign in from the desk itself");
		expect(row("Account 3").textContent).not.toContain("Signing in…");
	});

	test("Customize what is copied sends the ticked; Empty sends none", async () => {
		await mount();
		await openProvider("Anthropic");
		await click("Add subscription");
		await click("Customize what is copied…");
		await tick("Settings");
		await click("Add and sign in");
		expect(named("accounts.create").at(-1)?.[1]).toEqual({ backendId: "claude-acp", name: "Account 3", startWith: "system_default", categories: ["instructions", "skills", "plugins", "mcp"] });

		accounts = [a1(), a2()];
		await remount();
		await openProvider("Anthropic");
		await click("Add subscription");
		await click("Customize what is copied…");
		await tick("Settings");
		await tick("Empty");
		expect(hasButton("Customize what is copied…") || hasButton("Hide what is copied")).toBe(false);
		await click("Add and sign in");
		expect(named("accounts.create").at(-1)?.[1]).toEqual({ backendId: "claude-acp", name: "Account 3", startWith: "empty" });
	});

	test("rename saves the trimmed name", async () => {
		await mount();
		await openProvider("Anthropic");
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
		await openProvider("Anthropic");
		await openAccount("Account 1");
		await click("Remove account");
		expect(text()).toContain("Account 1 can't be removed while Ada uses it.");
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
		expect(band()).toBe("Anthropic");
		expect(text()).not.toContain("Account 2");
	});

	test("an account two teammates use names both when its removal is refused", async () => {
		accounts = [{ ...a1(), usedBy: ["ada", "mack"] }, a2()];
		await mount();
		await openProvider("Anthropic");
		await openAccount("Account 1");
		await click("Remove account");
		expect(text()).toContain("Account 1 can't be removed while Ada and Mack use it.");
	});

	test("Check asks the agent; an owner seat signs in from the desk", async () => {
		await mount();
		await openProvider("Anthropic");
		await openAccount("Account 2");
		expect(text()).toContain("Its teammates ask you to sign in at their next message");
		await click("Check");
		expect(calls).toContainEqual(["accounts.status", { accountId: "a2" }]);
		expect(text()).toContain("two@example.com · Pro");
		expect(text()).toContain("Checked with Claude Code just now");

		accounts = [a1(), { ...a2(), canSignInHere: false }];
		await remount();
		await openProvider("Anthropic");
		await openAccount("Account 2");
		expect(text()).toContain("Sign in from the desk itself");
		expect(hasButton("Sign in")).toBe(false);
	});

	test("a second sign-in says its teammates start fresh; a first one does not", async () => {
		poll = { state: "succeeded", output: "" };
		await mount();
		await openProvider("Anthropic");
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
		await openProvider("Anthropic");
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

	test("opening lands on the account's page with its provider behind Back", async () => {
		await mount("a1");
		expect(band()).toBe("Account 1");
		await click("Back to the team");
		expect(band()).toBe("Anthropic");
	});

	test("the next blank name skips a taken one", () => {
		expect(nextAccountName(["Account 1", "account 3"])).toBe("Account 4");
		expect(nextAccountName([])).toBe("Account 1");
	});
});

describe("Connecting a provider where no name is asked, as the welcome does", () => {
	const copilot = providers.find((one) => one.id === "github-copilot")!;
	const anthropic = providers.find((one) => one.id === "anthropic")!;
	async function connect(provider: Provider) {
		await act(async () => {
			root!.render(<ConnectProvider provider={provider} onConnected={() => {}} onCancel={() => {}} />);
		});
		await settle();
	}

	test("a sign-in starts on the one click that chose it, with no name field", async () => {
		await connect(copilot);
		expect(calls).toContainEqual(["credential.login", { providerId: "github-copilot" }]);
		expect(hasLabel("Name")).toBe(false);
		expect(text()).toContain("ABCD-1234");
	});

	test("a key is named for its provider, with no name field", async () => {
		await connect(anthropic);
		expect(hasLabel("Name")).toBe(false);
		await type(field("API key"), "sk-first");
		await click("Save key");
		expect(calls).toContainEqual(["credential.create", { providerId: "anthropic", label: "Anthropic", secret: "sk-first" }]);
	});
});
