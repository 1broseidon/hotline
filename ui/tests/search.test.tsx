import { expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";

test("a failed search is shown as unavailable and a later empty search clears it", async () => {
	const dom = new Window();
	const originals = new Map<string, PropertyDescriptor | undefined>();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		originals.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
	}
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Search } = await import("../src/components/Search");
	const { wire } = await import("../src/wire");
	let fail = true;
	const calls: string[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (name: string) => {
		if (name === "chapter.list") return [];
		calls.push(name);
		if (fail) throw new Error("Search is unavailable right now. Please try again.");
		return { hits: [], truncated: false };
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => { root.render(<Search personaId="ada" roster={[]} onClose={() => {}} onPick={() => {}} />); });
		const input = container.querySelector("input")!;
		await act(async () => {
			Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, "harbour");
			input.dispatchEvent(new dom.Event("input", { bubbles: true }));
		});
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 200)); });
		expect(calls).toEqual(["search.thread"]);
		expect(container.querySelector('[role="alert"]')?.textContent).toBe("Search is unavailable right now. Please try again.");
		expect(container.textContent).not.toContain("No matches");
		expect(container.textContent).not.toContain("Searching…");
		fail = false;
		await act(async () => {
			const everywhere = [...container.querySelectorAll("button")].find((button) => button.textContent === "Everywhere")!;
			everywhere.click();
		});
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 200)); });
		expect(calls).toEqual(["search.thread", "search.all"]);
		expect(container.querySelector('[role="alert"]')).toBeNull();
		expect(container.textContent).toContain("No matches");
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		await dom.happyDOM.close();
		for (const [key, descriptor] of originals) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	}
});
