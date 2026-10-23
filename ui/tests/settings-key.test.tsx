import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { Rail } = await import("../src/components/Rail");
const { Titlebar } = await import("../src/ui/Titlebar");

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

const key = <button type="button" aria-label="Settings" />;

test("Settings leads the rail's dock, in the window's bottom-left corner", async () => {
	await act(async () => {
		root.render(
			<Rail
				entries={[]}
				selectedId={null}
				seen={{}}
				connection="open"
				onSelect={() => {}}
				onEdit={() => {}}
				onDelete={() => {}}
				onPin={() => {}}
				onLead={() => {}}
				onAddDesk={() => {}}
				settings={key}
				search={<input aria-label="Search" />}
				deskKey={null}
			/>,
		);
	});
	const dock = container.querySelector(".rail-dock");
	expect(dock?.firstElementChild?.getAttribute("aria-label")).toBe("Settings");
});

test("the title bar holds Settings only while the rail is closed", async () => {
	const titlebar = (open: boolean) => <Titlebar rail={{ open, onToggle: () => {} }} onNew={() => {}} settings={key} />;
	await act(async () => root.render(titlebar(true)));
	expect(container.querySelector('.titlebar-tools [aria-label="Settings"]')).toBeNull();
	expect(container.querySelector('.titlebar-tools [aria-label="New teammate"]')).not.toBeNull();
	await act(async () => root.render(titlebar(false)));
	expect(container.querySelector('.titlebar-tools [aria-label="Settings"]')).not.toBeNull();
});
