import { afterEach, beforeEach, expect, test } from "bun:test";
import { act } from "react";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

const boot = installDom();
const { createRoot } = await import("react-dom/client");
const { Swatches } = await import("../src/ui/Swatches");
const { ACCENTS, accentOf, applyAccent, startAccent } = await import("../src/accent");
await boot.teardown();

let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let picked: string[];

beforeEach(() => {
	({ teardown } = installDom());
	picked = [];
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	delete document.documentElement.dataset["accent"];
	await teardown();
});

// The Accent row of Settings › Appearance, as General.tsx draws it.
async function mount(accent: string) {
	await act(async () => {
		root.render(
			<Swatches value={accentOf(accent)} choices={ACCENTS} label="Accent" attribute="data-accent" onChange={(next) => picked.push(next)} />,
		);
	});
}

const swatches = () => [...container.querySelectorAll<HTMLButtonElement>('fieldset[aria-label="Accent"] .swatch')];
const pressed = () => swatches().filter((one) => one.getAttribute("aria-pressed") === "true").map((one) => one.title);

test("the six accents by name, each painting its own preset, the room's one pressed", async () => {
	await mount("sky");
	expect(swatches().map((one) => [one.getAttribute("aria-label"), one.title, one.dataset["accent"]])).toEqual([
		["Mint", "Mint", "mint"],
		["Teal", "Teal", "teal"],
		["Sky", "Sky", "sky"],
		["Iris", "Iris", "iris"],
		["Orchid", "Orchid", "orchid"],
		["Rose", "Rose", "rose"],
	]);
	expect(pressed()).toEqual(["Sky"]);
});

test("an accent this window doesn't know shows as mint, the default", async () => {
	await mount("amber");
	expect(pressed()).toEqual(["Mint"]);
});

test("picking a swatch hands that accent on", async () => {
	await mount("mint");
	await act(async () => swatches()[4]!.click());
	expect(picked).toEqual(["orchid"]);
});

test("the window draws the room's accent on <html>, mint as none, and opens in the last one it drew", () => {
	applyAccent("iris");
	expect(document.documentElement.dataset["accent"]).toBe("iris");
	delete document.documentElement.dataset["accent"];
	startAccent();
	expect(document.documentElement.dataset["accent"]).toBe("iris");
	applyAccent("mint");
	expect(document.documentElement.dataset["accent"]).toBeUndefined();
	applyAccent("not-a-preset");
	expect(document.documentElement.dataset["accent"]).toBeUndefined();
	startAccent();
	expect(document.documentElement.dataset["accent"]).toBeUndefined();
});
