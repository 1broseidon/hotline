import { expect, test } from "bun:test";
import { installDom } from "./dom";

// A window with no change events, as a test that never draws useNarrow stubs it.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { useNarrow } = await import("../src/narrow");

test("the narrow query is the window's own at each mount, not the one there when the module loaded", async () => {
	const { teardown } = installDom();
	const listened: string[] = [];
	window.matchMedia = ((query: string) => ({
		matches: true,
		media: query,
		addEventListener: (type: string) => listened.push(type),
		removeEventListener: () => {},
	})) as unknown as typeof window.matchMedia;
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const seen: boolean[] = [];
	function Probe() {
		seen.push(useNarrow());
		return null;
	}
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => root.render(<Probe />));
		expect(listened).toEqual(["change"]);
		expect(seen.at(-1)).toBe(true);
	} finally {
		await act(async () => root.unmount());
		await teardown();
	}
});
