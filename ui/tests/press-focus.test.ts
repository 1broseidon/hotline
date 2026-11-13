import { expect, test } from "bun:test";
import { keepFocusOffPressedKeys } from "../src/pressFocus";
import { installDom } from "./dom";

test("a pressed key keeps no focus, but one that can be dragged keeps its press so the drag can start", async () => {
	const { dom, teardown } = installDom();
	const stop = keepFocusOffPressedKeys(dom as unknown as Window);
	try {
		dom.document.body.innerHTML = '<button id="plain">Open</button><button id="row" draggable="true"><span id="name">Ada</span></button><input id="field">';
		const press = (id: string) => {
			const event = new dom.MouseEvent("mousedown", { bubbles: true, cancelable: true });
			dom.document.getElementById(id)!.dispatchEvent(event);
			return event.defaultPrevented;
		};
		expect(press("plain")).toBe(true);
		expect(press("field")).toBe(false);
		expect(press("name")).toBe(false);
		// The row lets go of the focus a press gave it once the press ends.
		const row = dom.document.getElementById("row") as unknown as HTMLElement;
		row.focus();
		expect(dom.document.activeElement).toBe(row as never);
		dom.dispatchEvent(new dom.MouseEvent("mouseup", { bubbles: true }));
		expect(dom.document.activeElement).not.toBe(row as never);
	} finally {
		stop();
		await teardown();
	}
});
