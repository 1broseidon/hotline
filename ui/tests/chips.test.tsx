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

describe("a row of chips", () => {
	test("is a named group of buttons, the one on pressed", async () => {
		const { Chips } = await import("../src/ui/Chips");
		const picked: string[] = [];
		const choices = [
			{ id: "light", name: "Light" },
			{ id: "dark", name: "Dark" },
		] as const;
		await act(async () => {
			root.render(<Chips value="light" choices={choices} label="Theme" onChange={(id) => picked.push(id)} />);
		});
		const group = container.querySelector("fieldset");
		expect(group?.getAttribute("aria-label")).toBe("Theme");
		const chips = [...container.querySelectorAll<HTMLButtonElement>("fieldset > button")];
		expect(chips.map((chip) => [chip.textContent, chip.getAttribute("aria-pressed")])).toEqual([
			["Light", "true"],
			["Dark", "false"],
		]);
		expect(container.querySelector('[role="radio"]')).toBeNull();
		await act(async () => chips[1]!.click());
		expect(picked).toEqual(["dark"]);
	});
});
