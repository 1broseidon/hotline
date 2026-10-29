import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { SettingsSection } from "../src/components/Settings";
import { installDom } from "./dom";

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
let teardown: () => Promise<void>;
let opened: (SettingsSection | "back")[];

beforeEach(() => {
	({ dom, teardown } = installDom());
	(dom as unknown as { __hotlineDesk: unknown }).__hotlineDesk = { version: "0.39.0" };
	opened = [];
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => { root.unmount(); });
	await teardown();
});

async function mount(section: SettingsSection) {
	await act(async () => {
		root.render(<SettingsRail section={section} onSection={(one) => opened.push(one)} onBack={() => opened.push("back")} />);
	});
}
function buttons() { return [...container.querySelectorAll("button")]; }

describe("settings rail", () => {
	test("the list has no About or Shortcuts; the dock goes back, opens About by the version, and the shortcuts", async () => {
		await mount("general");
		const names = buttons().map((node) => node.textContent);
		expect(names).not.toContain("Keyboard shortcuts");
		expect(names).not.toContain("About Hotline");
		const dock = container.querySelector("footer.settings-dock")!;
		const [back, about, shortcuts] = [...dock.querySelectorAll("button")] as HTMLButtonElement[];
		// One pill, three keys: back, the version, the shortcuts' key.
		expect(back!.getAttribute("aria-label")).toBe("Back to the team");
		expect(about!.getAttribute("aria-label")).toBe("About Hotline, version 0.39.0");
		expect(about!.textContent).toBe("0.39.0");
		expect(shortcuts!.getAttribute("aria-label")).toBe("Keyboard shortcuts");
		// The head no longer carries the way back: the dock is the only one.
		expect(container.querySelectorAll('button[aria-label="Back to the team"]').length).toBe(1);
		// Tab order: the list, then the dock.
		const order = buttons();
		expect(order.indexOf(back!)).toBeGreaterThan(order.findIndex((node) => node.textContent === "Import"));
		await act(async () => { back!.click(); });
		await act(async () => { about!.click(); });
		await act(async () => { shortcuts!.click(); });
		expect(opened).toEqual(["back", "about", "shortcuts"]);
	});

	test("the open footer item carries aria-current", async () => {
		await mount("about");
		const about = container.querySelector('button[aria-label^="About Hotline"]')!;
		const shortcuts = container.querySelector('button[aria-label="Keyboard shortcuts"]')!;
		expect(about.getAttribute("aria-current")).toBe("true");
		expect(shortcuts.getAttribute("aria-current")).toBeNull();
	});
});
