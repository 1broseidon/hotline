import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

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

describe("the call key in the composer", () => {
	async function composer(props: { onCall?: () => void; onHangUp?: () => void } = {}) {
		const { Composer } = await import("../src/components/Composer");
		await act(async () =>
			root.render(<Composer personaId="ada" name="Ada" state="ready" replyQuote={null} onSend={() => {}} onCancel={() => {}} onClearReply={() => {}} {...props} />),
		);
	}

	test("is a soundwave with the field empty, and calls", async () => {
		const calls: string[] = [];
		await composer({ onCall: () => calls.push("call") });
		const call = container.querySelector<HTMLButtonElement>('[aria-label="Call Ada"]');
		expect(call?.className).toContain("composer-call");
		// A soundwave in a plain circle, the same for every teammate, and at rest.
		expect(call?.querySelector("svg")).not.toBeNull();
		expect(call?.getAttribute("style")).toBeNull();
		expect(call?.dataset["call"]).toBeUndefined();
		// No Send waits behind it with nothing to send.
		expect(container.querySelector('[aria-label="Send"]')).toBeNull();
		await act(async () => call?.click());
		expect(calls).toEqual(["call"]);
	});

	test("calls until a call is on, and is absent where the desk cannot call", async () => {
		await composer({ onCall: () => {}, onHangUp: () => {} });
		// A call snapshot that has ended is no call: the key still calls.
		expect(container.querySelector('[aria-label="Call Ada"]')).not.toBeNull();
		await composer();
		expect(container.querySelector('[aria-label="Call Ada"]')).toBeNull();
		expect(container.querySelector(".composer-call")).toBeNull();
	});
});
