import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	({ teardown } = installDom());
	act = (await import("react")).act;
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	delete (window as { __hotlineDesk?: unknown }).__hotlineDesk;
	await teardown();
});

const render = async (os: string) => {
	(window as { __hotlineDesk?: unknown }).__hotlineDesk = { platform: os };
	const { PaneClose } = await import("../src/ui/PaneClose");
	let closed = 0;
	await act(async () => root.render(<PaneClose onClose={() => closed++} />));
	const key = container.querySelector("button")!;
	await act(async () => key.click());
	return { key, closed };
};

test("a pane's close says Close where the page draws the window's own controls, and is an X on macOS", async () => {
	for (const os of ["windows", "linux"]) {
		const { key, closed } = await render(os);
		expect(key.textContent).toBe("Close");
		expect(key.querySelector("svg")).toBeNull();
		expect(closed).toBe(1);
	}
	const { key, closed } = await render("macos");
	expect(key.textContent).toBe("");
	expect(key.getAttribute("aria-label")).toBe("Close");
	expect(key.querySelector("svg")).not.toBeNull();
	expect(closed).toBe(1);
});
