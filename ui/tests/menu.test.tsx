import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
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

describe("a dropdown's tooltip", () => {
	test("a picker's is gone while its menu is open, so it covers no item", async () => {
		const { Picker } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<Picker value="" choices={[{ id: "", name: "Automatic" }, { id: "off", name: "Off" }]} placeholder="Automatic" label="Model for images" onChange={() => {}} />,
			);
		});
		const trigger = container.querySelector<HTMLButtonElement>('[aria-label="Model for images"]')!;
		expect(trigger.getAttribute("title")).toBe("Automatic");
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).not.toBeNull();
		expect(trigger.hasAttribute("title")).toBe(false);
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).toBeNull();
		expect(trigger.getAttribute("title")).toBe("Automatic");
	});

	test("a menu button's is gone while its menu is open", async () => {
		const { MenuButton } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<MenuButton entries={[{ kind: "item", id: "rename", text: "Rename", onSelect: () => {} }]} className="control" label="More">
					…
				</MenuButton>,
			);
		});
		const trigger = container.querySelector<HTMLButtonElement>('[aria-label="More"]')!;
		expect(trigger.getAttribute("title")).toBe("More");
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).not.toBeNull();
		expect(trigger.hasAttribute("title")).toBe(false);
	});
});
