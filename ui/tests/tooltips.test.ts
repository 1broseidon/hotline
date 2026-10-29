import { afterEach, beforeEach, expect, test } from "bun:test";
import { installDom } from "./dom";

let teardown: () => Promise<void>;
beforeEach(() => {
	({ teardown } = installDom());
});
afterEach(async () => {
	await teardown();
});

test("a press takes an element's tooltip away until the pointer leaves it; one still open keeps it away", async () => {
	const { quietTooltipsOnPress } = await import("../src/tooltips");
	const stop = quietTooltipsOnPress(document);
	const key = document.createElement("button");
	key.title = "Settings (Ctrl+,)";
	const icon = document.createElement("span");
	key.append(icon);
	document.body.append(key);
	icon.dispatchEvent(new window.PointerEvent("pointerdown", { bubbles: true }));
	expect(key.hasAttribute("title")).toBe(false);
	key.dispatchEvent(new window.PointerEvent("pointerleave"));
	expect(key.getAttribute("title")).toBe("Settings (Ctrl+,)");

	const pill = document.createElement("button");
	pill.title = "Ada · Claude";
	document.body.append(pill);
	pill.dispatchEvent(new window.PointerEvent("pointerdown", { bubbles: true }));
	pill.setAttribute("aria-expanded", "true");
	pill.dispatchEvent(new window.PointerEvent("pointerleave"));
	expect(pill.hasAttribute("title")).toBe(false);
	stop();
});
