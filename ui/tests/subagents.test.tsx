import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

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

describe("what stands on the composer's corner", () => {
	test("stands on the pill's top right, in the pill's own column", async () => {
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
		await act(async () => root.render(composer(<button type="button">http.server 8770</button>)));
		const corner = container.querySelector(".composer-corner");
		expect(corner?.textContent).toBe("http.server 8770");
		// The pill's centred column holds both, the corner first, so it is
		// placed against the pill and not the window.
		expect(corner?.parentElement?.className).toContain("max-w-[46rem]");
		expect(corner?.nextElementSibling?.className).toBe("composer");
		// Between turns there is no Interrupt key, and none here.
		expect(container.querySelector('[aria-label="Interrupt"]')).toBeNull();

		await act(async () => root.render(composer()));
		expect(container.querySelector(".composer-corner")).toBeNull();
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
