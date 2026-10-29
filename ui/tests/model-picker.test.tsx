import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { ConfigChoice } from "../src/generated/contract";
import { installDom } from "./dom";

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

const account = (id: string, label: string, shared: boolean, kind: "api_key" | "oauth" = "api_key") => ({ id, label, kind, shared });
const choice = (id: string, name: string, group: string, held: ReturnType<typeof account>): ConfigChoice => ({ id, name, group, account: held });

const work = account("work", "Work", true, "oauth");
const personal = account("personal", "Personal", true);
const openai = account("oai", "OpenAI", false);
const choices: ConfigChoice[] = [
	choice("anthropic#work/claude-haiku", "Claude Haiku", "Anthropic", work),
	choice("anthropic#work/claude-opus", "Claude Opus", "Anthropic", work),
	choice("anthropic#personal/claude-haiku", "Claude Haiku", "Anthropic", personal),
	choice("anthropic#personal/claude-opus", "Claude Opus", "Anthropic", personal),
	choice("openai/gpt-6", "GPT-6", "OpenAI", openai),
];
const efforts: ConfigChoice[] = ["low", "medium", "high", "max"].map((id) => ({ id, name: id[0]!.toUpperCase() + id.slice(1) }));

test("one row per account: a provider with two names each, a provider with one is its name", async () => {
	const { accountRows } = await import("../src/components/ModelPicker");
	const rows = accountRows(choices);
	expect(rows.map((row) => [row.name, row.detail, row.choices.length])).toEqual([
		["Anthropic · Work", "subscription", 2],
		["Anthropic · Personal", "API key", 2],
		["OpenAI", "API key", 1],
	]);
});

async function picker(props: Partial<Parameters<typeof import("../src/components/ModelPicker").ModelPicker>[0]> = {}) {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const calls: string[] = [];
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="anthropic#work/claude-haiku"
				effort={{ value: "medium", choices: efforts }}
				label="Model"
				onModel={(id) => {
					calls.push(`model:${id}`);
				}}
				onEffort={(value) => {
					calls.push(`effort:${value}`);
				}}
				onAddAccount={() => {
					calls.push("add");
				}}
				{...props}
			/>,
		);
	});
	return calls;
}

test("the button names model and effort, the account only in its tooltip", async () => {
	await picker();
	const button = container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
	expect(button.textContent).toBe("Claude Haiku · Medium");
	expect(button.textContent).not.toContain("Work");
	expect(button.title).toBe("Claude Haiku · Anthropic · Work · subscription · Medium");
});

const open = async () => act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
const mainMenu = () => document.querySelector<HTMLElement>('[role="menu"][aria-label="Model"]')!;
const mainModels = () => [...mainMenu().querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
const changeAgent = () => mainMenu().querySelector<HTMLButtonElement>(".model-change")!;
const agentRows = () => [...document.querySelectorAll<HTMLElement>('[role="menu"][aria-label="Change agent"] .model-account')];
const names = (rows: HTMLElement[]) => rows.map((row) => row.querySelector(".menu-item-text")?.textContent);
const hover = async (element: HTMLElement) => act(async () => element.dispatchEvent(new window.MouseEvent("mouseover", { bubbles: true })));
const press = async (key: string) => act(async () => mainMenu().dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true })));

test("the head names the agent's account, and its models are the menu: the one in use checked with its effort", async () => {
	await picker();
	await open();
	const menu = mainMenu();
	expect(menu.querySelector(".model-head-name")?.textContent).toBe("Anthropic · Work");
	expect(menu.querySelector(".model-head-kind")?.textContent).toBe("subscription");
	const models = mainModels();
	expect(models.map((model) => model.querySelector(".menu-item-text")?.textContent)).toEqual(["Claude Haiku", "Claude Opus"]);
	expect(models.map((model) => model.getAttribute("aria-checked"))).toEqual(["true", "false"]);
	expect(models[0]!.querySelector(".model-row-detail")?.textContent).toBe("Medium");
	expect(models[0]!.getAttribute("aria-haspopup")).toBe("menu");
	expect(models[1]!.getAttribute("aria-haspopup")).toBeNull();
	expect(models[1]!.querySelector(".model-row-detail")).toBeNull();
	// No slider, and no API or Agents headings: the others wait under Change agent.
	expect(menu.querySelector('[role="slider"]')).toBeNull();
	expect(menu.querySelectorAll(".menu-heading").length).toBe(0);
	expect(changeAgent().querySelector(".model-change-hint")?.textContent).toBe("API");
});

test("choosing another of the agent's models changes the model and closes the menu", async () => {
	const calls = await picker();
	await open();
	await act(async () => mainModels()[1]!.click());
	expect(calls).toEqual(["model:anthropic#work/claude-opus"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();
});

test("the model in use opens its effort levels beside it, the one in use checked, and a level sets the effort alone", async () => {
	const calls = await picker();
	await open();
	const inUse = mainModels()[0]!;
	await hover(inUse);
	expect(inUse.getAttribute("aria-expanded")).toBe("true");
	const levels = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Effort"] [role="menuitemradio"]')];
	expect(levels.map((level) => level.textContent)).toEqual(["Low", "Medium", "High", "Max"]);
	expect(levels.map((level) => level.getAttribute("aria-checked"))).toEqual(["false", "true", "false", "false"]);
	await act(async () => levels[2]!.click());
	expect(calls).toEqual(["effort:high"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();
});

test("Change agent opens every other account, and picking one of its models sets that account's model", async () => {
	const calls = await picker();
	await open();
	expect(changeAgent().getAttribute("aria-haspopup")).toBe("menu");
	await act(async () => changeAgent().click());
	expect(changeAgent().getAttribute("aria-expanded")).toBe("true");
	const list = document.querySelector('[role="menu"][aria-label="Change agent"]')!;
	expect([...list.querySelectorAll(".menu-heading")].map((heading) => heading.textContent)).toEqual(["API"]);
	expect(names(agentRows())).toEqual(["Anthropic · Personal", "OpenAI"]);
	await act(async () => agentRows()[0]!.click());
	const sub = document.querySelector('[role="menu"][aria-label="Anthropic · Personal"]')!;
	const models = [...sub.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
	expect(models.map((model) => model.textContent)).toEqual(["Claude Haiku", "Claude Opus"]);
	expect(models.some((model) => model.getAttribute("aria-checked") === "true")).toBe(false);
	await act(async () => models[1]!.click());
	expect(calls).toEqual(["model:anthropic#personal/claude-opus"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();
});

test("Add an account, under Change agent, goes to Settings › Providers", async () => {
	const calls = await picker();
	await open();
	await act(async () => changeAgent().click());
	const add = document.querySelector<HTMLButtonElement>('[role="menu"][aria-label="Change agent"] .model-add')!;
	expect(add.textContent).toBe("Add an account…");
	await act(async () => add.click());
	expect(calls).toEqual(["add"]);
});

test("a plain id runs on the account listing it plainly, and a model with one effort shows it but opens nothing", async () => {
	await picker({ modelId: "openai/gpt-6", effort: { value: "high", choices: [{ id: "high", name: "High" }] } });
	await open();
	expect(mainMenu().querySelector(".model-head-name")?.textContent).toBe("OpenAI");
	expect(mainMenu().querySelector(".model-head-kind")?.textContent).toBe("API key");
	const [gpt] = mainModels();
	expect(gpt!.getAttribute("aria-checked")).toBe("true");
	expect(gpt!.querySelector(".model-row-detail")?.textContent).toBe("High");
	expect(gpt!.getAttribute("aria-haspopup")).toBeNull();
	await hover(gpt!);
	expect(document.querySelector('[role="menu"][aria-label="Effort"]')).toBeNull();
});

test("the keyboard walks the menu: → opens the effort and Change agent, ← and Escape close them, Enter picks", async () => {
	const calls = await picker();
	await open();
	// The first arrow lights the model in use; → opens its efforts, on the one in use.
	await press("ArrowDown");
	expect(mainModels()[0]!.dataset["active"]).toBe("true");
	await press("ArrowRight");
	const effort = () => document.querySelector('[role="menu"][aria-label="Effort"]');
	expect(effort()).not.toBeNull();
	expect(effort()!.querySelector('[data-active="true"]')?.textContent).toBe("Medium");
	await press("ArrowLeft");
	expect(effort()).toBeNull();
	// Down past Claude Opus to Change agent; → opens the list, → again an account's models.
	await press("ArrowDown");
	await press("ArrowDown");
	expect(changeAgent().dataset["active"]).toBe("true");
	await press("ArrowRight");
	expect(changeAgent().getAttribute("aria-expanded")).toBe("true");
	expect(agentRows()[0]!.dataset["active"]).toBe("true");
	await press("ArrowRight");
	expect(document.querySelector('[role="menu"][aria-label="Anthropic · Personal"]')).not.toBeNull();
	await press("Escape");
	expect(document.querySelector('[role="menu"][aria-label="Anthropic · Personal"]')).toBeNull();
	await press("Escape");
	expect(document.querySelector('[role="menu"][aria-label="Change agent"]')).toBeNull();
	// Back up to the effort, down a level, Enter sets it.
	await press("ArrowUp");
	await press("ArrowUp");
	await press("Enter");
	await press("ArrowDown");
	await press("Enter");
	expect(calls).toEqual(["effort:high"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();
});

test("an agent with many models shows the first eight, the one in use among them, and More models opens them all", async () => {
	const many = Array.from({ length: 12 }, (_, at) => choice(`anthropic#work/m${at}`, `Model ${at}`, "Anthropic", work));
	const calls = await picker({ choices: many, modelId: "anthropic#work/m10" });
	await open();
	const shown = mainModels().map((model) => model.querySelector(".menu-item-text")?.textContent);
	expect(shown).toEqual(["Model 0", "Model 1", "Model 2", "Model 3", "Model 4", "Model 5", "Model 6", "Model 10"]);
	const more = mainMenu().querySelector<HTMLButtonElement>(".model-more")!;
	expect(more.textContent).toBe("More models");
	await hover(more);
	const all = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Anthropic · Work models"] [role="menuitemradio"]')];
	expect(all.length).toBe(12);
	expect(all[10]!.getAttribute("aria-checked")).toBe("true");
	await act(async () => all[11]!.click());
	expect(calls).toEqual(["model:anthropic#work/m11"]);
});

const agents = [
	{ id: "hotline", name: "Hotline Agent", choices: [] },
	{ id: "claude-acp", name: "Claude Code", choices: [] as ConfigChoice[] },
	// What Codex last reported on this desk, before any teammate runs on it.
	{ id: "codex-acp", name: "Codex", choices: [{ id: "gpt-6", name: "GPT-6" }] as ConfigChoice[] },
	{ id: "cursor", name: "Cursor", unavailable: "Not signed in.", choices: [] },
];

test("Change agent lists the accounts, then the other agents with the models they last reported; one with none is picked by its row, and one not ready does not open", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const moves: string[] = [];
	let opened = 0;
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="anthropic#work/claude-haiku"
				agents={agents}
				onOpen={() => {
					opened += 1;
				}}
				effort={null}
				label="Model"
				onModel={() => {}}
				onAgent={(backend, model) => {
					moves.push(`${backend}:${model ?? "own"}`);
				}}
				onEffort={() => {}}
			/>,
		);
	});
	await open();
	expect(changeAgent().querySelector(".model-change-hint")?.textContent).toBe("Claude, Codex, API…");
	await act(async () => changeAgent().click());
	const list = document.querySelector('[role="menu"][aria-label="Change agent"]')!;
	// What the teammate runs on is the menu itself; the rest are API (straight to a provider), then Agents.
	expect([...list.querySelectorAll(".menu-heading")].map((heading) => heading.textContent)).toEqual(["API", "Agents"]);
	const rows = agentRows();
	expect(names(rows)).toEqual(["Anthropic · Personal", "OpenAI", "Claude Code", "Codex", "Cursor"]);
	const cursor = rows[4]!;
	expect(cursor.getAttribute("aria-disabled")).toBe("true");
	expect(cursor.title).toBe("Not signed in.");
	await act(async () => cursor.click());
	expect(document.querySelector('[role="menu"][aria-label="Cursor"]')).toBeNull();
	// Codex opens the list it last reported; a model there moves the teammate onto it.
	expect(rows[3]!.getAttribute("aria-haspopup")).toBe("menu");
	await act(async () => rows[3]!.click());
	const gpt = document.querySelector('[role="menu"][aria-label="Codex"]')!.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!;
	expect(gpt.textContent).toBe("GPT-6");
	await act(async () => gpt.click());
	expect(moves).toEqual(["codex-acp:gpt-6"]);
	// Claude Code has reported nothing yet: no submenu, the row itself is the pick.
	await open();
	await act(async () => changeAgent().click());
	const claude = agentRows().find((row) => row.textContent?.includes("Claude Code"))!;
	expect(claude.getAttribute("aria-haspopup")).toBeNull();
	expect(claude.querySelector(".model-row-more")).toBeNull();
	await act(async () => claude.click());
	expect(document.querySelector('[role="menu"][aria-label="Claude Code"]')).toBeNull();
	expect(moves).toEqual(["codex-acp:gpt-6", "claude-acp:own"]);
	expect(document.body.textContent).not.toContain("Its own model");
	// Each opening reads the agents' lists again: the desk fills them in behind the first read.
	expect(opened).toBe(2);
});

test("on a harness, the head is the agent and its models are a model change; Hotline Agent's accounts are a move back", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const calls: string[] = [];
	const onClaude = agents.map((agent) => (agent.id === "claude-acp" ? { ...agent, choices: [{ id: "opus", name: "Opus" }, { id: "sonnet", name: "Sonnet" }] } : agent));
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="opus"
				backendId="claude-acp"
				agents={onClaude}
				effort={null}
				label="Model"
				plan={{ at: 0, windows: [{ kind: "five_hour", usedPercent: 8, status: "allowed" }] }}
				onModel={(id) => {
					calls.push(`model:${id}`);
				}}
				onAgent={(backend, model) => {
					calls.push(`move:${backend}:${model ?? "own"}`);
				}}
				onEffort={() => {}}
			/>,
		);
	});
	const button = container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
	expect(button.textContent).toBe("Opus");
	await open();
	// A harness that reports a plan is on a subscription.
	expect(mainMenu().querySelector(".model-head-name")?.textContent).toBe("Claude Code");
	expect(mainMenu().querySelector(".model-head-kind")?.textContent).toBe("subscription");
	expect(mainMenu().querySelectorAll(".plan-row").length).toBe(2);
	expect(mainModels().map((model) => model.textContent)).toEqual(["Opus", "Sonnet"]);
	await act(async () => changeAgent().click());
	// Claude Code, in use, is not listed again under Change agent.
	expect(names(agentRows())).toEqual(["Anthropic · Work", "Anthropic · Personal", "OpenAI", "Codex", "Cursor"]);
	await act(async () => mainModels()[1]!.click());
	await open();
	await act(async () => changeAgent().click());
	await act(async () => agentRows()[0]!.click());
	const haiku = document.querySelector('[role="menu"][aria-label="Anthropic · Work"]')!.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!;
	await act(async () => haiku.click());
	expect(calls).toEqual(["model:sonnet", "move:hotline:anthropic#work/claude-haiku"]);
});

test("a harness that lists no models shows the one in use, if named, and Change agent still moves it", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const moves: string[] = [];
	const render = async (modelId: string) =>
		act(async () => {
			root.render(
				<ModelPicker
					choices={choices}
					modelId={modelId}
					backendId="claude-acp"
					fallbackName="Claude Code"
					agents={agents}
					effort={null}
					label="Model"
					onModel={() => {}}
					onAgent={(backend, model) => {
						moves.push(`${backend}:${model ?? "own"}`);
					}}
					onEffort={() => {}}
				/>,
			);
		});
	await render("");
	await open();
	expect(mainMenu().querySelector(".model-head-name")?.textContent).toBe("Claude Code");
	expect(mainMenu().querySelector(".model-head-kind")).toBeNull();
	expect(mainModels().length).toBe(0);
	await act(async () => changeAgent().click());
	const codex = agentRows().find((row) => row.textContent?.includes("Codex"))!;
	await act(async () => codex.click());
	await act(async () => document.querySelector<HTMLButtonElement>('[role="menu"][aria-label="Codex"] [role="menuitemradio"]')!.click());
	expect(moves).toEqual(["codex-acp:gpt-6"]);
	await render("opus");
	await open();
	expect(mainModels().map((model) => [model.textContent, model.getAttribute("aria-checked")])).toEqual([["opus", "true"]]);
});

test("the registry's other ready agents share one row, and those not ready are left out", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const moves: string[] = [];
	const many = [
		...agents,
		{ id: "amp", name: "Amp", choices: [] },
		{ id: "cline", name: "Cline", choices: [] },
		{ id: "corust", name: "Corust Agent", unavailable: "Not installed.", choices: [] },
	];
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="anthropic#work/claude-haiku"
				agents={many}
				effort={null}
				label="Model"
				onModel={() => {}}
				onAgent={(backend, model) => {
					moves.push(`${backend}:${model ?? "own"}`);
				}}
				onEffort={() => {}}
			/>,
		);
	});
	await open();
	await act(async () => changeAgent().click());
	const rows = agentRows();
	expect(names(rows)).toEqual(["Anthropic · Personal", "OpenAI", "Claude Code", "Codex", "Cursor", "Other agents"]);
	expect(rows[5]!.querySelector(".model-row-detail")?.textContent).toBe("2");
	await act(async () => rows[5]!.click());
	const more = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Other agents"] [role="menuitemradio"]')];
	expect(more.map((item) => item.textContent)).toEqual(["Amp", "Cline"]);
	await act(async () => more[1]!.click());
	expect(moves).toEqual(["cline:own"]);
});

test("a model on an account that is gone keeps its name on the button", async () => {
	await picker({ modelId: "anthropic#gone/claude-opus" });
	const button = container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
	expect(button.textContent).toBe("Claude Opus · Medium");
});

test("the named agents are Antigravity, Claude Code, Codex, Cursor and Grok Build, alphabetical; Gemini CLI is one of the others", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const named = [
		{ id: "hotline", name: "Hotline Agent", choices: [] },
		{ id: "grok-build", name: "Grok Build", choices: [] },
		{ id: "gemini", name: "Gemini CLI", choices: [] },
		{ id: "cursor", name: "Cursor", choices: [] },
		{ id: "antigravity-acp", name: "Google Antigravity", choices: [] },
		{ id: "codex-acp", name: "Codex", choices: [] },
		{ id: "claude-acp", name: "Claude Code", choices: [] },
	];
	await act(async () => {
		root.render(
			<ModelPicker choices={choices} modelId="anthropic#work/claude-haiku" agents={named} effort={null} label="Model" onModel={() => {}} onAgent={() => {}} onEffort={() => {}} />,
		);
	});
	await open();
	await act(async () => changeAgent().click());
	expect(names(agentRows()).slice(2)).toEqual(["Claude Code", "Codex", "Cursor", "Google Antigravity", "Grok Build", "Other agents"]);
});

test("an agent that wants its sign-in says Sign in on its row, opens no stale list, and its row moves the teammate there", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const moves: string[] = [];
	const withSignIn = [
		{ id: "hotline", name: "Hotline Agent", choices: [] },
		{ id: "cursor", name: "Cursor", signIn: true, choices: [{ id: "auto", name: "Auto" }] },
		{ id: "codex-acp", name: "Codex", choices: [{ id: "gpt-6", name: "GPT-6" }] },
	];
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="anthropic#work/claude-haiku"
				agents={withSignIn}
				effort={null}
				label="Model"
				onModel={() => {}}
				onAgent={(backend, model) => {
					moves.push(`${backend}:${model ?? "own"}`);
				}}
				onEffort={() => {}}
			/>,
		);
	});
	await open();
	await act(async () => changeAgent().click());
	const cursor = agentRows().find((row) => row.textContent?.includes("Cursor"))!;
	expect(cursor.querySelector(".model-row-signin")?.textContent).toBe("Sign in");
	expect(cursor.getAttribute("aria-haspopup")).toBeNull();
	expect(cursor.title).toBe("Opens its sign-in");
	await act(async () => cursor.click());
	expect(document.querySelector('[role="menu"][aria-label="Cursor"]')).toBeNull();
	expect(moves).toEqual(["cursor:own"]);
});
