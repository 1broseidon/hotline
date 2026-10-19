import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { PeerThreadSummary, ThreadSummary } from "../src/generated/contract";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Threads } = await import("../src/components/teammate/Threads");
const { wire } = await import("../src/wire");

let answers: Record<string, () => unknown>;
let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let restores: (() => void)[];
let teardown: () => Promise<void>;

beforeEach(async () => {
	restores = [];
	({ teardown } = installDom());
	answers = {};
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => {
		const answer = answers[cmd];
		if (!answer) throw new Error(`unknown variant \`${cmd}\``);
		return answer();
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
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function openThreads() {
	await act(async () => {
		root.render(<Threads personaId="ada" roster={[]} onOpen={() => {}} />);
	});
	const fold = [...container.querySelectorAll("button")].find((node) => node.textContent?.includes("thread"));
	if (!fold) throw new Error(`No threads fold in: ${container.textContent}`);
	await act(async () => fold.click());
	return container.textContent ?? "";
}

describe("a teammate card's threads", () => {
	test("are read from thread.list", async () => {
		answers["thread.list"] = () =>
			[
				{ thread: { kind: "pair", key: "ada:grace" }, withName: "Grace", preview: "Shipped it", updatedAt: 1 },
				{ thread: { kind: "side", key: "side-1" }, preview: "Not a pair", updatedAt: 1 },
			] as unknown as ThreadSummary[];
		const text = await openThreads();
		expect(text).toContain("1 thread");
		expect(text).toContain("Grace");
		expect(text).toContain("Shipped it");
		expect(text).not.toContain("Not a pair");
	});

	test("still list on a desk older than thread.list, from peers.list", async () => {
		answers["peers.list"] = () =>
			[
				{
					threadKey: "ada:grace",
					withName: "Grace",
					preview: { fromName: "Grace", text: "Shipped it", at: 1 },
					exchanges: 2,
					lastAt: 1,
				},
			] as unknown as PeerThreadSummary[];
		const text = await openThreads();
		expect(text).toContain("1 thread");
		expect(text).toContain("Grace");
		expect(text).toContain("Grace: Shipped it");
	});
});
