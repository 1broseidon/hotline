import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { SettingsSection } from "../src/components/Settings";

// Loading the window's modules reads the platform, and the theme, before any test has a DOM.
Object.assign(globalThis, {
	window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
	localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
});
const { SettingsRail } = await import("../src/components/Settings");
Reflect.deleteProperty(globalThis, "localStorage");

let dom: Window;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let opened: SettingsSection[];

beforeEach(() => {
	restores = [];
	dom = new Window();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
	}
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { version: "0.39.0" };
	opened = [];
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => { root.unmount(); });
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});

async function mount(section: SettingsSection) {
	await act(async () => {
		root.render(<SettingsRail section={section} onSection={(one) => opened.push(one)} onBack={() => {}} />);
	});
}
function buttons() { return [...container.querySelectorAll("button")]; }

describe("settings rail", () => {
	test("the list has no About or Shortcuts, and the footer opens both", async () => {
		await mount("general");
		const names = buttons().map((node) => node.textContent);
		expect(names).not.toContain("Keyboard shortcuts");
		expect(names).not.toContain("About Hotline");
		const about = container.querySelector('button[aria-label="About Hotline, version 0.39.0"]') as HTMLButtonElement;
		const shortcuts = container.querySelector('button[aria-label="Keyboard shortcuts"]') as HTMLButtonElement;
		expect(about.textContent).toBe("Hotline 0.39.0");
		expect(shortcuts.textContent).toBe("Shortcuts");
		// Tab order: the list, then the version, then Shortcuts.
		const order = buttons();
		expect(order.indexOf(about)).toBeGreaterThan(order.findIndex((node) => node.textContent === "Import"));
		expect(order.indexOf(shortcuts)).toBe(order.indexOf(about) + 1);
		await act(async () => { about.click(); });
		await act(async () => { shortcuts.click(); });
		expect(opened).toEqual(["about", "shortcuts"]);
	});

	test("the open footer item carries aria-current", async () => {
		await mount("about");
		const about = container.querySelector('button[aria-label^="About Hotline"]')!;
		const shortcuts = container.querySelector('button[aria-label="Keyboard shortcuts"]')!;
		expect(about.getAttribute("aria-current")).toBe("true");
		expect(shortcuts.getAttribute("aria-current")).toBeNull();
	});
});
