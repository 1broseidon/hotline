import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { UpdateStatus, UpdateTrack } from "../src/native";
import { installDom } from "./dom";

// Loading the window's modules reads the platform before any test has a DOM.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false, addEventListener() {} }) } });
const native = await import("../src/native");
const { replaceDesks } = await import("../src/desks");
const { UpdatesSection, trackValue } = await import("../src/components/UpdatesSection");

let root: Root;
let container: HTMLDivElement;
let act: typeof import("react").act;
let restores: (() => void)[];
let teardown: () => Promise<void>;
let desk: UpdateStatus;
let asked: UpdateTrack[];

const status = (patch: Partial<UpdateStatus> = {}): UpdateStatus => ({
	current: "0.48.3", track: "stable", available: null, checkedAt: null, phase: "idle", downloaded: 0, total: null, error: null, disabledReason: null, ...patch,
});

beforeEach(async () => {
	({ teardown } = installDom());
	// A desk with no endpoint: the harness list below the row never arrives.
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	desk = status();
	asked = [];
	const spies = [
		spyOn(native, "watchUpdates").mockImplementation((onChange) => { onChange(desk); return () => {}; }),
		spyOn(native, "updateStatus").mockImplementation(async () => desk),
		spyOn(native, "setUpdateTrack").mockImplementation(async (track) => {
			asked.push(track);
			// The desk checks the new track before it answers.
			desk = { ...desk, track, available: track === "beta" ? { version: "0.49.0-beta.1", notes: "" } : null };
		}),
	];
	restores = spies.map((spy) => () => spy.mockRestore());
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	replaceDesks([]);
	for (const restore of restores) restore();
	container.remove();
	await teardown();
});

const mount = async () => {
	await act(async () => root.render(<UpdatesSection />));
};
const tracks = () => container.querySelector('[aria-label="Update track"]') as HTMLElement;
const chip = (name: string) => [...tracks().querySelectorAll("button")].find((button) => button.textContent === name) as HTMLButtonElement;
const row = () => tracks().parentElement as HTMLElement;

test("the track row says what each track is offered, and what a beta build does on Stable", () => {
	expect(trackValue("stable", "0.48.3")).toBe("Final releases");
	expect(trackValue("beta", "0.48.3")).toBe("Final releases and betas");
	expect(trackValue("beta", "0.49.0-beta.1")).toBe("Final releases and betas");
	expect(trackValue("stable", "0.49.0-beta.1")).toBe("Keeps 0.49.0-beta.1 until a newer final release");
});

test("a desk is on Stable until Beta is chosen, and choosing it asks the desk once", async () => {
	await mount();
	expect(row().className).toBe("nt-fold-row");
	expect(row().textContent).toContain("Track");
	expect(row().textContent).toContain("Final releases");
	expect(chip("Stable").getAttribute("aria-pressed")).toBe("true");
	expect(chip("Beta").getAttribute("aria-pressed")).toBe("false");
	await act(async () => chip("Stable").click());
	expect(asked).toEqual([]);
	await act(async () => chip("Beta").click());
	expect(asked).toEqual(["beta"]);
	expect(chip("Beta").getAttribute("aria-pressed")).toBe("true");
	expect(row().textContent).toContain("Final releases and betas");
	expect(container.textContent).toContain("0.49.0-beta.1 is available");
});

test("a beta build shows its whole version, and back on Stable the row says it stays", async () => {
	desk = status({ current: "0.49.0-beta.1", track: "beta" });
	await mount();
	expect(container.textContent).toContain("Hotline 0.49.0-beta.1");
	await act(async () => chip("Stable").click());
	expect(asked).toEqual(["stable"]);
	expect(row().textContent).toContain("Keeps 0.49.0-beta.1 until a newer final release");
});

test("an installation that cannot update itself cannot change track", async () => {
	desk = status({ disabledReason: "Update this installation from the release page." });
	await mount();
	expect(chip("Stable").disabled).toBe(true);
	expect(chip("Beta").disabled).toBe(true);
});

test("each choice has a tooltip of a few words", async () => {
	await mount();
	for (const name of ["Stable", "Beta"]) {
		const words = chip(name).title.split(" ");
		expect(words.length).toBeGreaterThanOrEqual(2);
		expect(words.length).toBeLessThanOrEqual(3);
	}
});
