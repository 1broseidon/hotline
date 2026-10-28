import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { Step } from "../src/scheduledRuns";
import { installDom } from "./dom";

let teardown: () => Promise<void>;
let root: Root;
let container: HTMLElement;
let act: typeof import("react").act;

beforeEach(async () => {
	({ teardown } = installDom());
	({ act } = await import("react"));
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	await teardown();
});

const steps: Step[] = [
	{ kind: "thought", id: "t1", ts: 1, text: "Looking at the mail first." },
	{ kind: "tool", id: "s1", ts: 2, toolCallId: "c1", title: "Search mail since Monday", status: "completed", output: [{ type: "text", text: "142 messages, 9 unread" }] },
	{ kind: "tool", id: "s2", ts: 3, toolCallId: "c2", title: "Read 9 threads", status: "completed" },
];

describe("the work in the pane", () => {
	test("the line pressed in the peek arrives open, the others closed and still openable", async () => {
		const { StepRows } = await import("../src/components/transcript/Steps");
		await act(async () => root.render(<StepRows items={steps} settled openId="s1" />));
		const rows = [...container.querySelectorAll<HTMLButtonElement>("button.step")];
		expect(rows[0]!.getAttribute("aria-expanded")).toBe("false");
		expect(rows[1]!.getAttribute("aria-expanded")).toBe("true");
		expect(container.querySelector(".step-out")?.textContent).toContain("142 messages");
		await act(async () => rows[0]!.click());
		expect(rows[0]!.getAttribute("aria-expanded")).toBe("true");
	});
});
