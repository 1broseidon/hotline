import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { RunningSubagent } from "../src/generated/contract";
import type { ThreadRef } from "../src/components/Transcript";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { shownState } = await import("../src/activity");

describe("a turn open only for its subagents", () => {
	test("reads as done, and any other turn as it is", () => {
		expect(shownState({ state: "thinking", awaitingSubagents: true })).toBe("ready");
		expect(shownState({ state: "thinking", awaitingSubagents: false })).toBe("thinking");
		expect(shownState({ state: "thinking" })).toBe("thinking");
		// The core only sets it mid-turn; a stale one never makes a stopped teammate ready.
		expect(shownState({ state: "stopped", awaitingSubagents: true })).toBe("stopped");
	});
});

const edges: RunningSubagent = { runId: "r1", title: "Edge cases", startedAt: 1 };
const brakes: RunningSubagent = { runId: "r2", title: "Brakes", startedAt: 2 };

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	dom = new Window();
	restores = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, localStorage: dom.localStorage, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => (descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key)));
	}
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});

describe("the subagents at the composer's corner", () => {
	test("stand on the pill's top right, in the pill's own column", async () => {
		const { Composer } = await import("../src/components/Composer");
		const composer = (corner?: ReactNode) => (
			<Composer
				personaId="ada"
				name="Ada"
				state="ready"
				replyQuote={null}
				onSend={() => {}}
				onCancel={() => {}}
				onClearReply={() => {}}
				{...(corner !== undefined ? { corner } : {})}
			/>
		);
		await act(async () => root.render(composer(<button type="button">Subagent · Edge cases</button>)));
		const corner = container.querySelector(".composer-corner");
		expect(corner?.textContent).toBe("Subagent · Edge cases");
		// The pill's centred column holds both, the corner first, so it is
		// placed against the pill and not the window.
		expect(corner?.parentElement?.className).toContain("max-w-[46rem]");
		expect(corner?.nextElementSibling?.className).toBe("composer");
		// Between turns there is no Interrupt key, and none here.
		expect(container.querySelector('[aria-label="Interrupt"]')).toBeNull();

		await act(async () => root.render(composer()));
		expect(container.querySelector(".composer-corner")).toBeNull();
	});

	test("name one subagent and open its run", async () => {
		const { SubagentChips } = await import("../src/components/Conversation");
		const opened: ThreadRef[] = [];
		await act(async () => {
			root.render(<SubagentChips subagents={[edges]} opened={(runId) => runId === "r1"} onOpenThread={(thread) => opened.push(thread)} />);
		});
		const chip = container.querySelector<HTMLButtonElement>('[aria-label="Subagent working: Edge cases"]');
		expect(chip?.textContent).toBe("Subagent · Edge cases");
		expect(chip?.getAttribute("aria-pressed")).toBe("true");
		await act(async () => chip!.click());
		expect(opened).toEqual([{ thread: { kind: "run", key: "r1" }, title: "Edge cases" }]);
	});

	test("count several, with a menu that opens any of them", async () => {
		const { SubagentChips } = await import("../src/components/Conversation");
		const opened: ThreadRef[] = [];
		await act(async () => {
			root.render(<SubagentChips subagents={[edges, brakes]} opened={() => false} onOpenThread={(thread) => opened.push(thread)} />);
		});
		const chip = container.querySelector<HTMLButtonElement>('[aria-label="2 subagents working"]');
		expect(chip?.textContent).toBe("2 subagents");
		expect(chip?.getAttribute("aria-haspopup")).toBe("menu");
		await act(async () => chip!.click());
		const items = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
		expect(items.map((item) => item.textContent)).toEqual(["Edge cases", "Brakes"]);
		await act(async () => items[1]!.click());
		expect(opened).toEqual([{ thread: { kind: "run", key: "r2" }, title: "Brakes" }]);
	});
});

describe("the conversation's foot under the corner", () => {
	test("keeps the last line clear of it and the way down above it", async () => {
		const { renderToStaticMarkup } = await import("react-dom/server");
		const { Transcript } = await import("../src/components/Transcript");
		const events = [{ kind: "agent" as const, id: "m1", ts: 1, text: "The lift is in crane.rs." }];
		const cornered = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} cornered focus={null} />);
		const plain = renderToStaticMarkup(<Transcript personaId="ada" name="Ada" events={events} streaming={[]} live={false} focus={null} />);
		expect(cornered).toContain("pb-14");
		expect(plain).not.toContain("pb-14");
	});
});
