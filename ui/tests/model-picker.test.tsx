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

test("the head names the account and the slider changes the effort alone; the account in use has the dot", async () => {
	const calls = await picker();
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const menu = document.querySelector('[role="menu"][aria-label="Model"]')!;
	expect(menu.querySelector(".model-head-account")?.textContent).toBe("Anthropic · Work · subscription");
	const slider = menu.querySelector<HTMLElement>('[role="slider"]')!;
	expect(slider.getAttribute("aria-valuetext")).toBe("Medium");
	const ticks = [...menu.querySelectorAll<HTMLButtonElement>(".effort-tick")];
	expect(ticks.map((tick) => tick.textContent)).toEqual(["Low", "Medium", "High", "Max"]);
	await act(async () => ticks[2]!.click());
	expect(calls).toEqual(["effort:high"]);
	const rows = [...menu.querySelectorAll<HTMLElement>(".model-row")];
	expect(rows.map((row) => row.querySelector(".menu-item-text")?.textContent)).toEqual(["Anthropic · Work", "Anthropic · Personal", "OpenAI"]);
	expect(rows.map((row) => row.querySelector(".model-dot") !== null)).toEqual([true, false, false]);
});

test("an account opens its models, and picking one sets that account's model", async () => {
	const calls = await picker();
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const personalRow = [...document.querySelectorAll<HTMLElement>(".model-row")].find((row) => row.textContent?.includes("Personal"))!;
	await act(async () => personalRow.click());
	const sub = document.querySelector('[role="menu"][aria-label="Anthropic · Personal"]')!;
	const models = [...sub.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
	expect(models.map((model) => model.textContent)).toEqual(["Claude Haiku", "Claude Opus"]);
	expect(models.some((model) => model.getAttribute("aria-checked") === "true")).toBe(false);
	await act(async () => models[1]!.click());
	expect(calls).toEqual(["model:anthropic#personal/claude-opus"]);
	expect(document.querySelector('[role="menu"]')).toBeNull();
});

test("Add an account goes to Settings › Providers", async () => {
	const calls = await picker();
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const add = [...document.querySelectorAll<HTMLButtonElement>(".model-add")][0]!;
	expect(add.textContent).toBe("Add an account…");
	await act(async () => add.click());
	expect(calls).toEqual(["add"]);
});

test("a plain id runs on the account listing it plainly, and a model with one effort has no slider", async () => {
	await picker({ modelId: "openai/gpt-6", effort: { value: "high", choices: [{ id: "high", name: "High" }] } });
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const menu = document.querySelector('[role="menu"][aria-label="Model"]')!;
	expect(menu.querySelector(".model-head-account")?.textContent).toBe("OpenAI · API key");
	expect(menu.querySelector('[role="slider"]')).toBeNull();
});

const agents = [
	{ id: "hotline", name: "Hotline Agent", choices: [] },
	{ id: "claude-acp", name: "Claude Code", choices: [] as ConfigChoice[] },
	// What Codex last reported on this desk, before any teammate runs on it.
	{ id: "codex-acp", name: "Codex", choices: [{ id: "gpt-6", name: "GPT-6" }] as ConfigChoice[] },
	{ id: "cursor", name: "Cursor", unavailable: "Not signed in.", choices: [] },
];

test("the other agents follow the accounts with the models they last reported; one with none is picked by its row, and one not ready does not open", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const moves: string[] = [];
	await act(async () => {
		root.render(
			<ModelPicker
				choices={choices}
				modelId="anthropic#work/claude-haiku"
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
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const menu = document.querySelector('[role="menu"][aria-label="Model"]')!;
	expect([...menu.querySelectorAll(".menu-heading")].map((heading) => heading.textContent)).toEqual(["Agents"]);
	const rows = [...menu.querySelectorAll<HTMLElement>(".model-row")];
	expect(rows.map((row) => row.querySelector(".menu-item-text")?.textContent)).toEqual(["Anthropic · Work", "Anthropic · Personal", "OpenAI", "Claude Code", "Codex", "Cursor"]);
	const cursor = rows[5]!;
	expect(cursor.getAttribute("aria-disabled")).toBe("true");
	expect(cursor.title).toBe("Not signed in.");
	await act(async () => cursor.click());
	expect(document.querySelector('[role="menu"][aria-label="Cursor"]')).toBeNull();
	// Codex opens the list it last reported; a model there moves the teammate onto it.
	expect(rows[4]!.getAttribute("aria-haspopup")).toBe("menu");
	await act(async () => rows[4]!.click());
	const gpt = document.querySelector('[role="menu"][aria-label="Codex"]')!.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!;
	expect(gpt.textContent).toBe("GPT-6");
	await act(async () => gpt.click());
	expect(moves).toEqual(["codex-acp:gpt-6"]);
	// Claude Code has reported nothing yet: no submenu, the row itself is the pick.
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const claude = [...document.querySelectorAll<HTMLElement>(".model-row")].find((row) => row.textContent?.includes("Claude Code"))!;
	expect(claude.getAttribute("aria-haspopup")).toBeNull();
	expect(claude.querySelector(".model-row-more")).toBeNull();
	await act(async () => claude.click());
	expect(document.querySelector('[role="menu"][aria-label="Claude Code"]')).toBeNull();
	expect(moves).toEqual(["codex-acp:gpt-6", "claude-acp:own"]);
	expect(document.body.textContent).not.toContain("Its own model");
});

test("on a harness, its models are a model change and Hotline Agent's are a move back", async () => {
	const { ModelPicker } = await import("../src/components/ModelPicker");
	const calls: string[] = [];
	const onClaude = agents.map((agent) => (agent.id === "claude-acp" ? { ...agent, choices: [{ id: "opus", name: "Opus" }, { id: "sonnet", name: "Sonnet" }] } : agent));
	const render = async () =>
		act(async () => {
			root.render(
				<ModelPicker
					choices={choices}
					modelId="opus"
					backendId="claude-acp"
					agents={onClaude}
					effort={null}
					label="Model"
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
	await render();
	const button = container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
	expect(button.textContent).toBe("Opus");
	await act(async () => button.click());
	const rows = [...document.querySelectorAll<HTMLElement>(".model-row")];
	expect(rows.map((row) => row.querySelector(".model-dot") !== null)).toEqual([false, false, false, true, false, false]);
	await act(async () => rows[3]!.click());
	const sonnet = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Claude Code"] [role="menuitemradio"]')].find((item) => item.textContent === "Sonnet")!;
	await act(async () => sonnet.click());
	await act(async () => button.click());
	const work = [...document.querySelectorAll<HTMLElement>(".model-row")][0]!;
	await act(async () => work.click());
	const haiku = document.querySelector('[role="menu"][aria-label="Anthropic · Work"]')!.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!;
	await act(async () => haiku.click());
	expect(calls).toEqual(["model:sonnet", "move:hotline:anthropic#work/claude-haiku"]);
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
	await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
	const rows = [...document.querySelectorAll<HTMLElement>(".model-row")];
	expect(rows.map((row) => row.querySelector(".menu-item-text")?.textContent)).toEqual(["Anthropic · Work", "Anthropic · Personal", "OpenAI", "Claude Code", "Codex", "Cursor", "Other agents"]);
	await act(async () => rows[6]!.click());
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
