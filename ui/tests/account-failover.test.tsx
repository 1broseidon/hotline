import { expect, spyOn, test } from "bun:test";
import type { BackendChoice, ConfigChoice } from "../src/generated/contract";
import { installDom } from "./dom";

const card = (failure: Record<string, unknown>) => `${failure["title"]}: ${failure["summary"]}\n\n${JSON.stringify({ hotlineFailure: { details: "", ...failure } })}`;

const work = { id: "work", label: "Work", kind: "oauth" as const, shared: true };
const personal = { id: "personal", label: "Personal", kind: "api_key" as const, shared: true };
const listed: ConfigChoice[] = [
	{ id: "anthropic#work/claude-x", name: "Claude X", group: "Anthropic", account: work },
	{ id: "anthropic#personal/claude-x", name: "Claude X", group: "Anthropic", account: personal },
	{ id: "anthropic#personal/claude-y", name: "Claude Y", group: "Anthropic", account: personal },
];
const agent = (id: string, label?: string): BackendChoice => ({
	id,
	name: id.startsWith("claude") ? "Claude Code" : "Codex",
	description: "",
	...(label !== undefined ? { account: { id: id.split("#")[1]!, label, kind: "agent" as const, shared: true } } : {}),
});
const backends: BackendChoice[] = [
	{ id: "hotline", name: "Hotline Agent", description: "" },
	agent("claude-acp"),
	agent("claude-acp#work", "Work"),
	agent("claude-acp#side", "Side projects"),
	agent("codex-acp"),
	{ ...agent("codex-acp#spare", "Spare"), unavailable: "is not installed" },
];

async function render(text: string, answers: Record<string, unknown>) {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { ErrorCard } = await import("../src/components/ErrorCard");
	const { wire } = await import("../src/wire");
	const calls: { cmd: string; params: unknown }[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		calls.push({ cmd, params });
		return answers[cmd] ?? null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const ran = { again: 0 };
	await act(async () => root.render(<ErrorCard text={text} personaId="huck" onRunAgain={async () => { ran.again++; }} />));
	const uses = () => [...container.querySelectorAll("button")].filter((button) => button.textContent?.startsWith("Use "));
	const done = async () => {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	};
	return { act, container, calls, ran, uses, done };
}

test("a spent Hotline Agent account offers the others serving its model, never itself, and moves only on a press", async () => {
	const resetsAt = Date.now() + 60 * 60 * 1000;
	// A plain id ran on the provider's default account, the first listed.
	const text = card({ kind: "usage_limit", title: "ChatGPT usage limit reached", summary: "This ChatGPT sign-in reached its usage limit.", model: "anthropic/claude-x", resetsAt });
	const { act, container, calls, ran, uses, done } = await render(text, { "models.list": listed });
	try {
		expect(container.textContent).toContain("Resets at ");
		expect(uses().map((button) => button.textContent)).toEqual(["Use Anthropic · Personal"]);
		// The same account fails the same way: no Try again.
		expect([...container.querySelectorAll("button")].some((button) => button.textContent === "Try again")).toBe(false);
		expect(ran.again).toBe(0);
		await act(async () => uses()[0]!.click());
		expect(calls.at(-1)).toEqual({ cmd: "session.set_model", params: { personaId: "huck", modelId: "anthropic#personal/claude-x" } });
		expect(ran.again).toBe(1);
	} finally {
		await done();
	}
});

test("a quota on a named account leaves that account out", async () => {
	const text = card({ kind: "quota", title: "Provider quota exhausted", summary: "Check your balance.", model: "anthropic#personal/claude-x" });
	const { uses, done } = await render(text, { "models.list": listed });
	try {
		expect(uses().map((button) => button.textContent)).toEqual(["Use Anthropic · Work"]);
	} finally {
		await done();
	}
});

test("a harness out of usage offers its agent's other ready accounts, the one on this computer included, and moves there with its model", async () => {
	const text = card({ kind: "usage_limit", title: "Claude Code usage limit reached", summary: "You've hit your limit · resets 3pm", backend: "claude-acp#work", model: "opus" });
	const { act, calls, ran, uses, done } = await render(text, { "backends.list": backends });
	try {
		expect(calls.map((call) => call.cmd)).toEqual(["backends.list"]);
		expect(uses().map((button) => button.textContent)).toEqual(["Use Claude Code", "Use Claude Code · Side projects"]);
		await act(async () => uses()[1]!.click());
		expect(calls.at(-1)).toEqual({ cmd: "session.set_backend", params: { personaId: "huck", backendId: "claude-acp#side", modelId: "opus" } });
		expect(ran.again).toBe(1);
	} finally {
		await done();
	}
});

test("a harness whose account is gone, with no other ready, says where to add one", async () => {
	const text = card({ kind: "account_gone", title: "Account no longer connected", summary: "Pick another account.", backend: "codex-acp#gone" });
	const { uses, done } = await render(text, { "backends.list": [backends[4], backends[5]] });
	try {
		expect(uses().map((button) => button.textContent)).toEqual(["Use Codex"]);
	} finally {
		await done();
	}
	const alone = await render(text, { "backends.list": [backends[5]] });
	try {
		expect(alone.uses()).toEqual([]);
		expect(alone.container.textContent).toContain("No other account of this agent is ready. Add one in Settings › Providers.");
	} finally {
		await alone.done();
	}
});

test("an agent's account is named after its agent, and its id after the agent it runs", async () => {
	const { agentOf, backendName } = await import("../src/accounts");
	expect(agentOf("claude-acp#work")).toBe("claude-acp");
	expect(agentOf("claude-acp")).toBe("claude-acp");
	expect(agentOf("claude-acp#")).toBe("claude-acp#");
	expect(backendName(backends[2]!)).toBe("Claude Code · Work");
	expect(backendName(backends[1]!)).toBe("Claude Code");
});

test("New teammate and the room's default list each agent as it is on this computer, never an account, and an agent's account is no provider of Hotline Agent's", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { BackendPicker } = await import("../src/components/BackendPicker");
	const { wire } = await import("../src/wire");
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		if (cmd === "credential.list")
			return [
				{ id: "k", providerId: "anthropic", credentialKind: "api_key", label: "Anthropic", revoked: false, createdAt: 1, updatedAt: 1 },
				{ id: "work", providerId: "claude-acp", credentialKind: "agent", label: "Work", revoked: false, createdAt: 1, updatedAt: 1 },
			];
		if (cmd === "providers.list") return [{ id: "anthropic", name: "Anthropic", credentialKinds: ["api_key"], modelDiscovery: false }];
		return null;
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<BackendPicker backends={backends} selected="hotline" name="agent" labelledBy="agent" onSelect={() => {}} />));
		const titles = [...container.querySelectorAll(".group-row-title")].map((title) => title.textContent);
		expect(titles).toEqual(["Hotline Agent", "Claude Code", "Codex"]);
		expect([...container.querySelectorAll(".provider-chip")].map((chip) => chip.textContent)).toEqual(["Anthropic"]);
	} finally {
		await act(async () => root.unmount());
		command.mockRestore();
		await teardown();
	}
});
