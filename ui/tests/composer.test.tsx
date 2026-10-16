import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

// The mic shows where this Mac can hear.
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));

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

describe("the composer's right edge without a call key", () => {
	async function composer(props: { onCall?: () => void; draft?: string } = {}) {
		const { Composer } = await import("../src/components/Composer");
		await act(async () =>
			root.render(<Composer personaId="ada" name="Ada" state="ready" replyQuote={null} onSend={() => {}} onCancel={() => {}} onClearReply={() => {}} {...props} />),
		);
	}
	const keys = () => [...container.querySelectorAll("button.composer-key")].map((key) => key.getAttribute("aria-label"));

	test("the mic is the last key with the field empty, and no hidden Send holds the slot", async () => {
		await composer();
		expect(keys().at(-1)).toBe("Dictate");
		expect(container.querySelector('[aria-label="Send"]')).toBeNull();
	});

	test("the call key stays the last key where calls are available", async () => {
		await composer({ onCall: () => {} });
		expect(keys().slice(-2)).toEqual(["Dictate", "Call Ada"]);
	});
});
