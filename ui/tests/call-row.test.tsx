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
