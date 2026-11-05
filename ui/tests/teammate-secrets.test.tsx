import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Persona, SessionInfo, SharedSecret } from "../src/generated/contract";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Teammate } = await import("../src/components/Teammate");
const { wire } = await import("../src/wire");
const { replaceDesks } = await import("../src/desks");

const persona: Persona = {
	id: "ada", name: "Ada", goal: "", backendId: "hotline", cwd: "/work/ada",
	mcpPolicy: { mode: "none", serverIds: [] }, skillPolicy: { mode: "none", names: [] },
	backgroundWork: false, allowedSenders: [], sessionCheckpoints: [], createdAt: 1, updatedAt: 1,
	computer: { enabled: true },
	secrets: ["GITHUB_TOKEN"],
};
const session: SessionInfo = {
	personaId: "ada", state: "idle", contextRestored: false, models: [], modes: [], configs: [], slashCommands: [],
	capabilities: { activeInput: false, loadSession: false, resume: false, fork: false, mcpHttp: false, image: false, additionalDirectories: false },
};
const stored: SharedSecret[] = [
	{ name: "GITHUB_TOKEN", kind: "variable", use: "auto", updatedAt: 1 },
	{ name: "NPM_TOKEN", kind: "variable", use: "auto", updatedAt: 1 },
];

class NoResize {
	observe() {}
	unobserve() {}
	disconnect() {}
}

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom({ ResizeObserver: NoResize }));
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	calls = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		calls.push({ cmd, params });
		if (cmd === "persona.update") return persona;
		if (cmd === "secrets.list") return stored;
		if (cmd === "computer.status") return { state: "absent" };
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

async function mount(who: Persona) {
	await act(async () => {
		root.render(<Teammate persona={who} session={session} jobs={[]} roster={[]} focusSchedules={false} onClose={() => {}} onDeleted={() => {}} onOpenThread={() => {}} />);
	});
}

const section = (heading: string) => [...container.querySelectorAll("section")].find((node) => node.querySelector("h3")?.textContent === heading);

test("Secrets is the teammate's own section, not a fold under Computer, and a tick saves on the teammate", async () => {
	await mount(persona);
	const computer = section("Computer");
	const secrets = section("Secrets");
	if (!computer || !secrets) throw new Error("No Computer or Secrets section");
	expect(computer.textContent).not.toContain("Secrets");
	expect(computer.contains(secrets)).toBe(false);
	expect(secrets.textContent).toContain("Logins and passkeys · 1 given");

	const fold = secrets.querySelector<HTMLButtonElement>("[aria-expanded]");
	await act(async () => fold!.click());
	const npm = [...secrets.querySelectorAll("label")].find((node) => node.textContent?.includes("NPM_TOKEN"))?.querySelector("input");
	if (!npm) throw new Error("No NPM_TOKEN tick");
	await act(async () => npm.click());
	expect(calls.at(-1)).toEqual({ cmd: "persona.update", params: { id: "ada", patch: { secrets: ["GITHUB_TOKEN", "NPM_TOKEN"] } } });
});

test("a teammate with no computer is told nothing uses its ticks yet", async () => {
	await mount({ ...persona, computer: { enabled: false } });
	expect(section("Secrets")?.textContent).toContain("It has no computer, so nothing does yet.");
	await act(async () => root.render(null));
	await mount(persona);
	expect(section("Secrets")?.textContent).not.toContain("nothing does yet");
});

test("on the Mac app, a teammate with no computer arms its desk browser for a passkey under Secrets", async () => {
	const shell = window as unknown as { __hotlineDesk?: { platform: string } };
	shell.__hotlineDesk = { platform: "macos" };
	restores.push(() => delete shell.__hotlineDesk);
	await mount({ ...persona, computer: { enabled: false } });
	const secrets = section("Secrets");
	if (!secrets) throw new Error("No Secrets section");
	expect(secrets.textContent).toContain("signs in with these passkeys");
	expect(secrets.textContent).not.toContain("nothing does yet");
	const add = [...secrets.querySelectorAll("button")].find((node) => node.textContent?.includes("Add a passkey"));
	if (!add) throw new Error("No Add a passkey");
	await act(async () => add.click());
	expect(secrets.textContent).toContain("Arm the browser");
	expect(secrets.textContent).not.toContain("starts Ada's computer");

	await act(async () => root.render(null));
	await mount(persona);
	expect(
		[...(section("Secrets")?.querySelectorAll("button") ?? [])].some((node) => node.textContent?.includes("Add a passkey")),
		"a teammate with a computer arms it under Computer",
	).toBe(false);
});
