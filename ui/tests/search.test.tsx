import { expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { RosterEntry } from "../src/wire";

const roster = [
	{ persona: { id: "ada", name: "Ada" }, session: { personaId: "ada", state: "idle" } },
	{ persona: { id: "kit", name: "Kit" }, session: { personaId: "kit", state: "idle" } },
] as unknown as RosterEntry[];

async function withSearch(run: (helpers: {
	container: HTMLDivElement;
	type(text: string): Promise<void>;
	settle(): Promise<void>;
	act: typeof import("react").act;
}) => Promise<void>, answer: (name: string) => unknown, picked: [string, string | undefined][] = []) {
	const dom = new Window();
	const originals = new Map<string, PropertyDescriptor | undefined>();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true, requestAnimationFrame: (run: () => void) => setTimeout(run, 0) })) {
		originals.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
	}
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { TeamSearch } = await import("../src/components/Search");
	const { wire } = await import("../src/wire");
	const command = spyOn(wire, "command").mockImplementation((async (name: string) => answer(name)) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(<TeamSearch roster={roster} selectedId="ada" placement="dock" ask={0} onPick={(personaId, eventId) => picked.push([personaId, eventId])} />);
		});
		const input = container.querySelector("input")!;
		await run({
			container,
			act,
			type: async (text) => {
				await act(async () => {
					input.dispatchEvent(new dom.FocusEvent("focusin", { bubbles: true }));
					Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, text);
					input.dispatchEvent(new dom.Event("input", { bubbles: true }));
				});
			},
			settle: async () => {
				await act(async () => { await new Promise((resolve) => setTimeout(resolve, 200)); });
			},
		});
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		await dom.happyDOM.close();
		for (const [key, descriptor] of originals) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	}
}

test("the dock's field is real: typing shows teammates by name, then matches grouped by teammate, and a pick lands on the line", async () => {
	const picked: [string, string | undefined][] = [];
	await withSearch(async ({ container, type, settle, act }) => {
		await type("ki");
		await settle();
		const heads = [...container.querySelectorAll(".team-results-head")].map((head) => head.textContent);
		expect(heads).toEqual(["Teammates", "Kit"]);
		expect(container.querySelector(".hit-person")?.textContent).toEndWith("Kit");
		const hit = [...container.querySelectorAll<HTMLButtonElement>(".hit")].find((button) => button.textContent?.includes("the kiln"))!;
		await act(async () => hit.click());
		expect(picked).toEqual([["kit", "e1"]]);
	}, (name) => {
		if (name === "chapter.list") return [];
		return { hits: [{ kind: "message", personaId: "kit", eventId: "e1", ts: 1, from: "them", excerpt: "the kiln is lit" }], truncated: false };
	}, picked);
});

test("a failed search says so, and never shows a stale count", async () => {
	await withSearch(async ({ container, type, settle }) => {
		await type("harbour");
		await settle();
		expect(container.querySelector('[role="alert"]')?.textContent).toBe("Search is unavailable right now. Please try again.");
		expect(container.textContent).not.toContain("Searching…");
	}, (name) => {
		if (name === "chapter.list") return [];
		throw new Error("Search is unavailable right now. Please try again.");
	});
});
