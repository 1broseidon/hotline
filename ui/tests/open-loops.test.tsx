import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { OpenLoop } from "../src/generated/contract";
import { wire } from "../src/wire";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

let act: typeof import("react").act;
let OpenLoops: typeof import("../src/components/OpenLoops").OpenLoops;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restores: (() => void)[];

beforeEach(async () => {
	restores = [];
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	act = (await import("react")).act;
	({ OpenLoops } = await import("../src/components/OpenLoops"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	for (const restore of restores.reverse()) restore();
	await act(async () => root.unmount());
	await teardown();
});

const loop = (id: string, extra: Partial<OpenLoop>): OpenLoop => ({
	id,
	key: id,
	summary: `Loop ${id}`,
	state: "active",
	source: "chapter",
	createdAt: Date.now() - 3_600_000,
	updatedAt: Date.now() - 3_600_000,
	...extra,
});

test("renders open loops, supports Ask and Dismiss, and keeps completed/archived collapsed until opened", async () => {
	let pushSnapshot: ((entries: OpenLoop[]) => void) | undefined;
	const subSpy = spyOn(wire, "subscribe").mockImplementation((_target, handlers) => {
		pushSnapshot = handlers.snapshot as (entries: OpenLoop[]) => void;
		return () => {};
	});
	const dismissed: { personaId: string; id: string }[] = [];
	const cmdSpy = spyOn(wire, "command").mockImplementation(async (name, params) => {
		if (name === "loops.dismiss") {
			dismissed.push(params as { personaId: string; id: string });
			return loop((params as { id: string }).id, { state: "archived", closedAt: Date.now() }) as never;
		}
		throw new Error(`unexpected command ${name}`);
	});
	restores.push(() => subSpy.mockRestore(), () => cmdSpy.mockRestore());

	const asked: string[] = [];
	await act(async () => root.render(<OpenLoops personaId="ada" onAskLoop={(text) => asked.push(text)} />));
	// Nothing rendered while empty.
	expect(container.textContent).toBe("");

	const initial: OpenLoop[] = [
		loop("l-stale", { key: "ca-filing", summary: "File California report", state: "stale" }),
		loop("l-active", { key: "larder", summary: "Verify Larder backup", state: "active", jobId: "job-1" }),
		loop("l-done", { key: "dns", summary: "Updated DNS record", state: "done", closedAt: Date.now() - 60_000 }),
	];
	await act(async () => pushSnapshot?.(initial));

	expect(container.textContent).toContain("Open loops");
	expect(container.textContent).toContain("File California report");
	expect(container.textContent).toContain("Verify Larder backup");
	expect(container.textContent).toContain("Tracks job");
	// Settled loop is tucked behind the collapsed disclosure.
	expect(container.textContent).not.toContain("Updated DNS record");
	expect(container.textContent).toContain("Completed & archived (1)");

	// Ask prefills the conversation composer.
	const askBtn = container.querySelector<HTMLButtonElement>('button[aria-label="Ask about File California report"]')!;
	await act(async () => askBtn.click());
	expect(asked).toEqual(['About "File California report": ']);

	// Dismiss calls loops.dismiss.
	const dismissBtn = container.querySelector<HTMLButtonElement>('button[aria-label="Dismiss Verify Larder backup"]')!;
	await act(async () => dismissBtn.click());
	expect(dismissed).toEqual([{ personaId: "ada", id: "l-active" }]);

	// Expanding disclosure reveals completed/archived rows.
	const toggle = [...container.querySelectorAll("button")].find((btn) =>
		(btn.textContent ?? "").includes("Completed & archived"),
	)!;
	await act(async () => toggle.click());
	expect(container.textContent).toContain("Updated DNS record");
});
