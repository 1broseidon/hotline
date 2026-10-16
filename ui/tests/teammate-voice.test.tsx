import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo } from "../src/generated/contract";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

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
// Legacy voice speaks with OpenAI's model and its voices.
const options = {
	tts: { options: [{ providerId: "openai", models: [{ id: "gpt-4o-mini-tts", voices: ["marin", "nova"] }] }] },
};
const legacy = { capabilities: [], available: true, budget: { spentDayUsd: 0, spentMonthUsd: 0 }, tts: { providerId: "openai", modelId: "gpt-4o-mini-tts", voice: "marin" } };
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	dom = new Window();
	restores = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true, ResizeObserver: NoResize })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key));
	}
	// A desk with no endpoint: the room's wire never opens, so the pane reads no settings.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	status = legacy;
	// The desk's voice status and options are what the test sets; everything
	// else is refused, which the pane reads as "nothing to show".
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
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
	await dom.happyDOM.close();
});

async function mount(who: Persona, info: SessionInfo) {
	await act(async () => {
		root.render(
			<Teammate persona={who} session={info} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} onOpenThread={() => {}} />,
		);
	});
}

function voiceButton(): HTMLButtonElement {
	const button = container.querySelector<HTMLButtonElement>('button[aria-label="The voice Ada answers a call in"]');
	if (!button) throw new Error("No voice picker");
	return button;
}

describe("a teammate's voice", () => {
	test("is picked from the voice front's voices when calls run on one", async () => {
		status = {
			...legacy,
			capabilities: ["voiceFront"],
			front: { providerId: "gpt-live", modelId: "gpt-live-1-codex", voice: "cove", voices: ["cove", "maple"] },
		};
		await mount(persona, session);
		expect(voiceButton().title).toBe("The desk's · Cove");
		await act(async () => voiceButton().click());
		expect(document.body.textContent).toContain("Maple");
		expect(document.body.textContent).not.toContain("Nova");
	});

	test("an own voice for the front reads as picked, one for Legacy voice as the desk's", async () => {
		status = { ...legacy, front: { providerId: "grok", modelId: "grok-voice-latest", voice: "eve", voices: ["eve", "rex"] } };
		await mount({ ...persona, voice: { providerId: "grok", modelId: "grok-voice-latest", voice: "rex" } }, session);
		expect(voiceButton().title).toBe("Rex");
		await act(async () => root.unmount());
		root = (await import("react-dom/client")).createRoot(container);
		await mount({ ...persona, voice: { providerId: "openai", modelId: "gpt-4o-mini-tts", voice: "nova" } }, session);
		expect(voiceButton().title).toBe("The desk's · Eve");
	});

	test("is Legacy voice's when the desk runs no front", async () => {
		await mount(persona, session);
		expect(voiceButton().title).toBe("The desk's · Marin");
	});
});
