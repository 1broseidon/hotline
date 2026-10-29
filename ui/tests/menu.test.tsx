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

describe("a dropdown's tooltip", () => {
	test("a picker's is gone while its menu is open, so it covers no item", async () => {
		const { Picker } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<Picker value="" choices={[{ id: "", name: "Automatic" }, { id: "off", name: "Off" }]} placeholder="Automatic" label="Model for images" onChange={() => {}} />,
			);
		});
		const trigger = container.querySelector<HTMLButtonElement>('[aria-label="Model for images"]')!;
		expect(trigger.getAttribute("title")).toBe("Automatic");
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).not.toBeNull();
		expect(trigger.hasAttribute("title")).toBe(false);
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).toBeNull();
		expect(trigger.getAttribute("title")).toBe("Automatic");
	});

	test("a menu button's is gone while its menu is open", async () => {
		const { MenuButton } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<MenuButton entries={[{ kind: "item", id: "rename", text: "Rename", onSelect: () => {} }]} className="control" label="More">
					…
				</MenuButton>,
			);
		});
		const trigger = container.querySelector<HTMLButtonElement>('[aria-label="More"]')!;
		expect(trigger.getAttribute("title")).toBe("More");
		await act(async () => trigger.click());
		expect(document.querySelector('[role="menu"]')).not.toBeNull();
		expect(trigger.hasAttribute("title")).toBe(false);
	});
});

describe("the conversation's band", () => {
	test("one pill is the model and the teammate's menu, with its face; the name stands in the band only with the rail closed", async () => {
		const source = await Bun.file(new URL("../src/components/Conversation.tsx", import.meta.url)).text();
		// No face of its own any more: the pill carries it and the teammate's actions.
		expect(source).not.toContain("band-face");
		expect(source).toMatch(/<SessionPickers[^>]*teammate=\{teammate\}/s);
		expect(source).toMatch(/working: state === "thinking"/);
		expect(source).not.toContain("<MoreIcon");
		// Every action the face menu had, its shortcut and Remove under a rule.
		for (const id of ['id: "teammate"', 'id: "chapter"', 'id: "side-list"', 'id: "delete"']) expect(source).toContain(id);
		expect(source).toContain('shortcut: chordGlyph("teammate")');
		// The name leaves the band's left while the rail names it.
		expect(source).toMatch(/\{!railOpen && ourCall === null && \(/);
		expect(source).not.toMatch(/title=\{`Teammate \(\$\{chordKeys/);
	});

	test("a heading with a detail draws as a card: the name over the goal", async () => {
		const { MenuButton } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<MenuButton
					className="control"
					label="Goldie"
					entries={[
						{ kind: "heading", text: "Goldie", detail: "Help me with whatever I bring you." },
						{ kind: "item", id: "settings", text: "Settings", onSelect: () => {} },
					]}
				>
					G
				</MenuButton>,
			);
		});
		await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
		const card = document.querySelector(".menu-card")!;
		expect(card.querySelector(".menu-card-name")?.textContent).toBe("Goldie");
		expect(card.querySelector(".menu-card-detail")?.textContent).toBe("Help me with whatever I bring you.");
	});

	test("a card heading with no detail is still a card: the name beside the face, no empty line under it", async () => {
		const { MenuButton } = await import("../src/ui/Menu");
		await act(async () => {
			root.render(
				<MenuButton
					className="control"
					label="Flight Risk"
					entries={[
						{ kind: "heading", card: true, text: "Flight Risk", icon: <span className="face" /> },
						{ kind: "item", id: "settings", text: "Settings", onSelect: () => {} },
					]}
				>
					F
				</MenuButton>,
			);
		});
		await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
		const card = document.querySelector(".menu-card")!;
		expect(card.querySelector(".face")).not.toBeNull();
		expect(card.querySelector(".menu-card-name")?.textContent).toBe("Flight Risk");
		expect(card.querySelector(".menu-card-detail")).toBeNull();
	});
});
