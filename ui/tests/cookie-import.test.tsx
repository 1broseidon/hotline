import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { wire } = await import("../src/wire");
const { replaceDesks, setActiveDesk } = await import("../src/desks");
const { CookieImport } = await import("../src/components/CookieImport");
Reflect.deleteProperty(globalThis, "localStorage");

const local = { id: "local", name: "This computer", kind: "local" as const, origin: "", token: "" };
const brave = { id: "brave", name: "Brave", family: "chromium", profiles: [], blocked: true };
const arc = { id: "arc", name: "Arc", family: "chromium", profiles: [], blocked: true };
const firefox = { id: "firefox", name: "Firefox", family: "firefox", profiles: [{ id: "Profiles/a.default", name: "default" }], blocked: false };

let browsers: unknown[];
let asked: string[];
let root: Root | null;
let container: HTMLDivElement;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom());
	asked = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		asked.push(cmd === "computer.cookies.preview" ? `${cmd} ${String(params.browserId)}` : cmd);
		if (cmd === "computer.browsers.list") return browsers;
		if (cmd === "computer.cookies.preview") return [];
		throw new Error(`Unexpected command ${cmd}`);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => {
		root?.unmount();
		root = null;
	});
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function mount() {
	replaceDesks([local]);
	setActiveDesk(local.id);
	await act(async () => {
		root!.render(<CookieImport personaId="poe" running onClose={() => undefined} />);
	});
}
const text = () => container.textContent ?? "";
const choices = () => [...container.querySelectorAll("select[aria-label='Browser'] option")].map((option) => option.textContent);

describe("bringing cookies over from a browser the host keeps the desk out of", () => {
	test("names the blocked browsers and says what to allow, and offers only the readable ones", async () => {
		browsers = [brave, arc, firefox];
		await mount();
		expect(text()).toContain("Brave and Arc");
		expect(choices()).toEqual(["Firefox"]);
		// The preview starts on the readable browser; a blocked one is never read.
		expect(asked).toContain("computer.cookies.preview firefox");
		expect(asked.some((call) => call.endsWith("brave") || call.endsWith("arc"))).toBe(false);
	});

	test("says why there is nothing to pick when every browser is blocked, rather than that none was found", async () => {
		browsers = [brave];
		await mount();
		expect(text()).toContain("Brave");
		expect(text()).not.toContain("No browser with cookies was found");
		expect(container.querySelector("select")).toBeNull();
	});
});
