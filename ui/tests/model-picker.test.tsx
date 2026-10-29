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
const mainModels = () => [...mainMenu().querySelectorAll<HTMLButtonElement>('.model-row[role="menuitemradio"]')];
const tabs = () => [...mainMenu().querySelectorAll<HTMLButtonElement>('fieldset[aria-label="Effort"] [role="menuitemradio"]')];
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
	// The effort is the tabs over the models; no row opens levels beside it.
	expect(tabs().map((tab) => [tab.textContent, tab.getAttribute("aria-checked")])).toEqual([
		["Low", "false"],
		["Med", "true"],
		["High", "false"],
		["Max", "false"],
	]);
	expect(models.map((model) => model.getAttribute("aria-haspopup"))).toEqual([null, null]);
	expect(models.some((model) => model.querySelector(".model-row-more") !== null)).toBe(false);
	// No slider, and no API or Agents headings: the others wait under Change agent.
	expect(menu.querySelector('[role="slider"]')).toBeNull();
	expect(menu.querySelectorAll(".menu-heading").length).toBe(0);
	expect(changeAgent().querySelector(".model-change-hint")?.textContent).toBe("API");
});

test("choosing another of the agent's models changes the model and keeps the menu open", async () => {
	const calls = await picker();
	await open();
	await act(async () => mainModels()[1]!.click());
	expect(calls).toEqual(["model:anthropic#work/claude-opus"]);
	expect(mainMenu()).not.toBeNull();
	// The model in use, chosen again, is nothing to do.
	await act(async () => mainModels()[1]!.click());
	expect(calls).toEqual(["model:anthropic#work/claude-opus"]);
});

test("a tab sets the effort and keeps the menu open, the tab chosen at once", async () => {
	const calls = await picker();
	await open();
	await act(async () => tabs()[2]!.click());
	expect(calls).toEqual(["effort:high"]);
	expect(mainMenu()).not.toBeNull();
	expect(tabs().map((tab) => tab.getAttribute("aria-checked"))).toEqual(["false", "false", "true", "false"]);
	// The tab in use, chosen again, sends nothing.
	await act(async () => tabs()[2]!.click());
	expect(calls).toEqual(["effort:high"]);
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

test("a plain id runs on the account listing it plainly, and a model with one effort or none has no tabs", async () => {
	await picker({ modelId: "openai/gpt-6", effort: { value: "high", choices: [{ id: "high", name: "High" }] } });
	await open();
	expect(mainMenu().querySelector(".model-head-name")?.textContent).toBe("OpenAI");
	expect(mainMenu().querySelector(".model-head-kind")?.textContent).toBe("API key");
	const [gpt] = mainModels();
	expect(gpt!.getAttribute("aria-checked")).toBe("true");
	expect(tabs()).toEqual([]);
	await picker({ modelId: "openai/gpt-6", effort: null });
	expect(tabs()).toEqual([]);
});

test("the keyboard walks the menu: ← and → walk the tabs, → opens Change agent, ← and Escape close it, Enter picks and stays", async () => {
	const calls = await picker();
	await open();
	// The first arrow lights the model in use; up is the effort row, on the tab in use.
	await press("ArrowDown");
	expect(mainModels()[0]!.dataset["active"]).toBe("true");
	await press("ArrowUp");
	expect(tabs()[1]!.dataset["active"]).toBe("true");
	await press("ArrowRight");
	expect(tabs()[2]!.dataset["active"]).toBe("true");
	await press("Enter");
	expect(calls).toEqual(["effort:high"]);
	expect(mainMenu()).not.toBeNull();
	// Down to Claude Opus: Enter switches to it and the menu stays.
	await press("ArrowDown");
	await press("ArrowDown");
	await press("Enter");
	expect(calls).toEqual(["effort:high", "model:anthropic#work/claude-opus"]);
	expect(mainMenu()).not.toBeNull();
	// On to Change agent; → opens the list, → again an account's models.
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
	await press("Escape");
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
	expect(mainMenu()).not.toBeNull();
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
				plan={{ at: 0, plan: "Max", windows: [{ kind: "five_hour", usedPercent: 8, resetsAt: Date.now() + 3 * 3_600_000, windowMinutes: 300, status: "allowed" }] }}
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
	// A harness that reports a plan is on a subscription, named when the plan says.
	expect(mainMenu().querySelector(".model-head-name")?.textContent).toBe("Claude Code");
	expect(mainMenu().querySelector(".model-head-kind")?.textContent).toBe("Max subscription");
	// The plan's lines are the band gauge's, not the menu's.
	expect(mainMenu().querySelectorAll(".plan-row").length).toBe(0);
	expect(mainModels().map((model) => model.textContent)).toEqual(["Opus", "Sonnet"]);
	await act(async () => changeAgent().click());
	// Claude Code, in use, is not listed again under Change agent.
	expect(names(agentRows())).toEqual(["Anthropic · Work", "Anthropic · Personal", "OpenAI", "Codex", "Cursor"]);
	await act(async () => mainModels()[1]!.click());
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

test("with the teammate it is one pill and one menu: the face ends the pill, the menu is who, what it runs on, then its actions", async () => {
	const done: string[] = [];
	const calls = await picker({
		plan: { at: 0, plan: "Max", windows: [{ kind: "five_hour", usedPercent: 91, status: "warning" }] },
		teammate: {
			name: "Goldie",
			goal: "Help me with whatever I bring you.",
			working: true,
			face: (size) => <span className="face" data-size={size} />,
			actions: [
				{ kind: "item", id: "teammate", text: "Settings", shortcut: "⌘I", checked: true, onSelect: () => done.push("settings") },
				{ kind: "item", id: "chapter", text: "New chapter", disabled: true, onSelect: () => done.push("chapter") },
				{ kind: "item", id: "side-list", text: "Threads", onSelect: () => done.push("threads") },
				{ kind: "rule" },
				{ kind: "item", id: "delete", text: "Remove", danger: true, onSelect: () => done.push("remove") },
			],
		},
	});
	const pill = container.querySelector<HTMLButtonElement>("button.model-pill")!;
	expect(pill.getAttribute("aria-label")).toBe("Goldie, working: Claude Haiku · Medium");
	expect(pill.querySelector('.model-pill-face .face[data-size="24"]')).not.toBeNull();
	expect(pill.querySelector(".model-pill-beat")).not.toBeNull();
	// Near the limit is the band gauge's to say, not the pill's.
	expect(pill.querySelector(".plan-dot")).toBeNull();
	expect(pill.querySelector(".model-pill-chevron")).toBeNull();

	await act(async () => pill.click());
	const menu = document.querySelector<HTMLElement>('[role="menu"][aria-label="Goldie"]')!;
	expect(menu.querySelector(".model-who .menu-card-name")?.textContent).toBe("Goldie");
	expect(menu.querySelector(".model-who .menu-card-detail")?.textContent).toBe("Help me with whatever I bring you.");
	expect(menu.querySelector(".model-head-name")?.textContent).toBe("Anthropic · Work");
	// The effort tabs, models, Change agent, then the actions, a rule above them and above Remove.
	const items = [...menu.querySelectorAll<HTMLElement>("[data-row]")].slice(1);
	expect(menu.querySelector("[data-row]")?.getAttribute("aria-label")).toBe("Effort");
	expect(items.map((one) => one.textContent)).toEqual(["Claude Haiku", "Claude Opus", "Change agentAPI", "Settings⌘I", "New chapter", "Threads", "Remove"]);
	expect(menu.querySelectorAll("hr.menu-rule").length).toBe(2);
	expect(items[3]!.querySelector(".model-check svg")).not.toBeNull();
	expect(items[4]!.getAttribute("aria-disabled")).toBe("true");
	expect(items[6]!.classList.contains("model-danger")).toBe(true);

	// The keyboard walks on into the actions: a disabled one does nothing, Enter on one closes and does it.
	const press = async (key: string) => act(async () => menu.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true })));
	for (let at = 0; at < 5; at++) await press("ArrowDown");
	expect(items[4]!.dataset["active"]).toBe("true");
	await press("Enter");
	expect(done).toEqual([]);
	await press("ArrowDown");
	await press("Enter");
	expect(done).toEqual(["threads"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();

	await act(async () => pill.click());
	await act(async () => document.querySelector<HTMLElement>(".model-danger")!.click());
	expect(done).toEqual(["threads", "remove"]);
	expect(calls).toEqual([]);
});

test("a compact pill is the model's mark and the face, the model in its label", async () => {
	await picker({ compact: true, teammate: { name: "Goldie", goal: "", working: false, face: () => <span className="face" />, actions: [] } });
	const pill = container.querySelector<HTMLButtonElement>("button.model-pill")!;
	expect(pill.querySelector(".model-pill-label")).toBeNull();
	expect(pill.getAttribute("aria-label")).toBe("Goldie: Claude Haiku · Medium");
	expect(pill.querySelector(".model-pill-beat")).toBeNull();
});

const levels = (ids: string[]): ConfigChoice[] => ids.map((id) => ({ id, name: id === "default" ? "Default" : id[0]!.toUpperCase() + id.slice(1) }));
// Claude Code's models as the desk read them out of band, each with its own levels.
const claudeModels: ConfigChoice[] = [
	{ id: "opus", name: "Opus 5.5", efforts: levels(["default", "low", "medium", "high", "xhigh", "max"]), defaultEffort: "default" },
	{ id: "sonnet", name: "Sonnet 4.6", efforts: levels(["default", "low", "medium", "high", "max"]), defaultEffort: "default" },
	{ id: "haiku", name: "Haiku 4.5", efforts: [] },
];

/** A Claude Code teammate on Opus at `value`; `answer` is what the desk says a switch landed on. */
async function claudePicker(value: string, answer: () => Promise<string | null>, landsOn?: (id: string) => string | undefined) {
	const calls: string[] = [];
	await picker({
		choices: [],
		backendId: "claude-acp",
		modelId: "opus",
		agents: [{ id: "claude-acp", name: "Claude Code", choices: claudeModels }],
		onAgent: () => {},
		effort: { value, choices: claudeModels[0]!.efforts! },
		onModel: (id) => {
			calls.push(`model:${id}`);
			return answer();
		},
		onEffort: (level) => {
			calls.push(`effort:${level}`);
		},
		landsOn,
	});
	return calls;
}

test("the tabs are the model in use's levels: Default is Auto, which Claude decides, and Medium is Med", async () => {
	await claudePicker("xhigh", async () => null);
	await open();
	expect(tabs().map((tab) => tab.textContent)).toEqual(["Auto", "Low", "Med", "High", "Xhigh", "Max"]);
	expect(tabs()[0]!.title).toBe("Claude decides");
	expect(tabs()[2]!.title).toBe("Medium");
	expect(tabs()[4]!.getAttribute("aria-checked")).toBe("true");
	const { tabLabel } = await import("../src/components/ModelPicker");
	expect(tabLabel({ id: "default", name: "Default" }, "codex-acp")).toEqual({ label: "Auto", title: "The agent decides" });
	expect(tabLabel({ id: "ultra", name: "Ultra" }, "codex-acp")).toEqual({ label: "Ultra" });
});

test("switching model keeps the menu open and shows the new model's tabs at once, dimmed with its check beating until the desk answers", async () => {
	let answer: (effort: string | null) => void = () => {};
	const calls = await claudePicker(
		"xhigh",
		() =>
			new Promise((resolve) => {
				answer = resolve;
			}),
		(id) => (id === "sonnet" ? "high" : undefined),
	);
	await open();
	await act(async () => mainModels()[1]!.click());
	expect(calls).toEqual(["model:sonnet"]);
	expect(mainMenu()).not.toBeNull();
	const [opus, sonnet] = mainModels();
	expect([opus!.getAttribute("aria-checked"), sonnet!.getAttribute("aria-checked")]).toEqual(["false", "true"]);
	expect(sonnet!.getAttribute("aria-busy")).toBe("true");
	expect(sonnet!.querySelector(".model-check")?.classList.contains("beat")).toBe(true);
	// Sonnet's levels, no Xhigh, at the level it is likely to land on, dimmed.
	expect(tabs().map((tab) => tab.textContent)).toEqual(["Auto", "Low", "Med", "High", "Max"]);
	expect(tabs().find((tab) => tab.getAttribute("aria-checked") === "true")?.textContent).toBe("High");
	const group = mainMenu().querySelector<HTMLElement>('fieldset[aria-label="Effort"]')!;
	expect(group.dataset["busy"]).toBe("true");
	// A tab while the switch waits does nothing.
	await act(async () => tabs()[1]!.click());
	expect(calls).toEqual(["model:sonnet"]);

	// The desk answers: the effort it landed on is the one selected, not dimmed.
	await act(async () => answer("medium"));
	expect(group.dataset["busy"]).toBeUndefined();
	expect(mainModels()[1]!.getAttribute("aria-busy")).toBeNull();
	expect(tabs().find((tab) => tab.getAttribute("aria-checked") === "true")?.textContent).toBe("Med");
});

test("a switch to a model without levels shows no tabs, and a refused switch goes back", async () => {
	let refuse: (error: Error) => void = () => {};
	await claudePicker(
		"high",
		() =>
			new Promise((_, reject) => {
				refuse = reject;
			}),
	);
	await open();
	await act(async () => mainModels()[2]!.click());
	expect(tabs()).toEqual([]);
	expect(mainModels()[2]!.getAttribute("aria-checked")).toBe("true");
	await act(async () => refuse(new Error("no")));
	expect(mainModels()[0]!.getAttribute("aria-checked")).toBe("true");
	expect(tabs().length).toBe(6);
});
