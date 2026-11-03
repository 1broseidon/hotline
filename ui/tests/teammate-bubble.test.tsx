import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks, wireFor } = await import("../src/desks");

const persona: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "hotline", cwd: "/work/ada",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
};
const session: SessionInfo = {
	personaId: "ada", state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
};

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let status: Record<string, unknown>;
// Turn by turn speaks with OpenAI's model and its voices.
const options = {
	tts: { options: [{ providerId: "openai", models: [{ id: "gpt-4o-mini-tts", voices: ["marin", "nova"] }] }] },
};
const turnByTurn = { capabilities: [], available: true, budget: { spentDayUsd: 0, spentMonthUsd: 0 }, tts: { providerId: "openai", modelId: "gpt-4o-mini-tts", voice: "marin" } };
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	// A test desk whose room snapshot and commands are supplied below.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	status = turnByTurn;
	// The desk's voice status and options, so the sheet draws; everything
	// else is refused, which the pane reads as "nothing to show".
	const connection = wireFor("local");
	const subscription = spyOn(connection, "subscribe").mockImplementation((_target, handlers) => {
		let cancelled = false;
		queueMicrotask(() => { if (!cancelled) handlers.snapshot([]); });
		return () => { cancelled = true; };
	});
	restores.push(() => subscription.mockRestore());
	const command = spyOn(connection, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "persona.update") return persona;
		if (cmd === "voice.status") return status;
		if (cmd === "capabilities.options") return options;
		throw new Error(`Not in this test: ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

const { AgentSay } = await import("../src/components/transcript/Bubbles");
const { bubbleChoices } = await import("../src/bubbles");
const { faceHue } = await import("../src/ui/Avatar");

async function mount(who: Persona) {
	await act(async () => {
		root.render(
			<Teammate persona={who} session={session} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} onOpenThread={() => {}} />,
		);
	});
}

const swatches = () => [...container.querySelectorAll<HTMLButtonElement>('fieldset[aria-label="Bubble"] .swatch')];
const pressed = () => swatches().filter((one) => one.getAttribute("aria-pressed") === "true").map((one) => one.title);
const FACES = ["gold", "lime", "green", "teal", "blue", "violet", "pink"];

describe("a teammate's bubble", () => {
	test("is grey until picked, offered as grey, its own face hue, then the rest, never gold", async () => {
		await mount(persona);
		const names = swatches().map((one) => one.dataset["bubble"]);
		const own = FACES[faceHue("ada")]!;
		expect(names[0]).toBe("grey");
		if (own !== "gold") expect(names[1]).toBe(own);
		expect([...names].sort()).toEqual(["blue", "green", "grey", "lime", "pink", "teal", "violet"]);
		expect(swatches().every((one) => one.title !== "" && one.title === one.getAttribute("aria-label"))).toBe(true);
		expect(pressed()).toEqual(["Grey"]);
	});

	test("shows the colour it was given, and a name this window doesn't know as grey", async () => {
		await mount({ ...persona, bubble: "teal" });
		expect(pressed()).toEqual(["Teal"]);
		await act(async () => root.unmount());
		root = (await import("react-dom/client")).createRoot(container);
		await mount({ ...persona, bubble: "gold" });
		expect(pressed()).toEqual(["Grey"]);
	});

	test("picking a colour saves it on the teammate, and grey clears it", async () => {
		await mount({ ...persona, bubble: "teal" });
		await act(async () => swatches().find((one) => one.dataset["bubble"] === "violet")!.click());
		await act(async () => swatches().find((one) => one.dataset["bubble"] === "grey")!.click());
		expect(calls.filter((call) => call.cmd === "persona.update").map((call) => call.params)).toEqual([
			{ id: "ada", patch: { bubble: "violet" } },
			{ id: "ada", patch: { bubble: null } },
		]);
	});

	test("a teammate whose face is gold is offered grey and the six in hue order", () => {
		let id = "a";
		while (faceHue(id) !== 0) id += "a";
		expect(bubbleChoices(id).map((one) => one.id)).toEqual(["grey", "lime", "green", "teal", "blue", "violet", "pink"]);
	});

	test("colours that teammate's replies, and leaves a grey one without a colour of its own", async () => {
		const event = { kind: "agent" as const, id: "a1", ts: 1, text: "Done." };
		await act(async () => root.render(<AgentSay personaId="ada" event={event} run={{ top: true, bottom: true }} reactions={undefined} bubble="violet" />));
		expect(container.querySelector<HTMLElement>(".said-them")!.dataset["bubble"]).toBe("violet");
		await act(async () => root.render(<AgentSay personaId="ada" event={event} run={{ top: true, bottom: true }} reactions={undefined} />));
		expect(container.querySelector<HTMLElement>(".said-them")!.hasAttribute("data-bubble")).toBe(false);
	});
});
