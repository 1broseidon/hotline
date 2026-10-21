import { afterEach, beforeEach, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { LinkEvent } from "../src/dock";
import { installDom } from "./dom";

let act: typeof import("react").act;
let Transcript: typeof import("../src/components/Transcript").Transcript;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;

beforeEach(async () => {
	const installed = installDom({ requestAnimationFrame: () => 0, cancelAnimationFrame: () => {} });
	teardown = installed.teardown;
	Object.assign(globalThis, { ResizeObserver: installed.dom.ResizeObserver });
	act = (await import("react")).act;
	({ Transcript } = await import("../src/components/Transcript"));
	container = document.createElement("div");
	document.body.append(container);
	root = (await import("react-dom/client")).createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await teardown();
});

test("a call row follows its own turn and then its parked link, and still opens the call", async () => {
	let link: LinkEvent = { kind: "link", id: "link:call:c1", ts: 1, thread: "c1", threadKind: "call", title: "Release checks", state: "live" };
	const opened: string[] = [];
	const draw = async (workingCalls: string[]) => {
		await act(async () => root.render(
			<Transcript personaId="ada" name="Ada" events={[link]} streaming={[]} live={false} focus={null}
				workingCalls={workingCalls} onOpenThread={(ref) => opened.push(ref.thread.key)} />,
		));
		return container.querySelector("button.rule-line")!;
	};
	expect((await draw(["c1"])).textContent).toBe("Release checks · running· Open");
	expect((await draw(["other-call"])).textContent).toBe("Release checks · open· Open");
	link = { ...link, state: "parked" };
	const parked = await draw([]);
	expect(parked.textContent).toBe("Release checks · parked· Open");
	expect(parked.getAttribute("aria-label")).toBe("Release checks · parked. Open it");
	await act(async () => (parked as HTMLButtonElement).click());
	expect(opened).toEqual(["c1"]);
	// A resumed turn can arrive before the rewritten link.
	expect((await draw(["c1"])).textContent).toBe("Release checks · running· Open");
});

test("a call thread folds speech updates and opens work results as work", async () => {
	const { NOTHING_YET, reduceThread } = await import("../src/tape");
	const said = (text: string): import("../src/generated/contract").TranscriptEvent => ({ kind: "agent", id: "front-c1-0", ts: 1, text });
	let held = reduceThread(NOTHING_YET, { type: "snapshot", items: [said("Done."), said("Done. Saved in crane.txt.")] });
	held = reduceThread(held, { type: "event", item: said("Done. Saved in crane.txt. It’s in the chat.") });
	held = reduceThread(held, { type: "event", item: { kind: "tool", id: "work-result", ts: 2, toolCallId: "work-result", title: "Work result", status: "completed", output: [{ type: "text", text: "Farmer Crane tends the field.\nThe corn bows in the wind.\nEvening brings him home." }] } });
	await act(async () => root.render(<Transcript personaId="ada" name="Ada" events={held.events} streaming={held.streaming} live={false} focus={null} />));
	expect(container.textContent?.match(/Done\./g)).toHaveLength(1);
	expect(container.textContent).toContain("It’s in the chat.");
	// Work is disclosed by the same working control as a tool's output.
	const steps = container.querySelector<HTMLButtonElement>(".steps-caption");
	if (steps) await act(async () => steps.click());
	const result = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Work result");
	expect(result).toBeDefined();
	await act(async () => result!.click());
	expect(container.querySelector(".step-out")?.textContent).toContain("Farmer Crane tends the field.");
	expect(container.textContent?.match(/Farmer Crane/g)).toHaveLength(1);
});
