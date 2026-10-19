import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { Root } from "react-dom/client";
import { installDom } from "./dom";

// Rendering needs the shell's platform and motion preference, not a live desk.
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });

// The mic shows where this Mac can hear.
const real = await import("../src/voice/transcription");
mock.module("../src/voice/transcription", () => ({ ...real, hearsOnThisMac: async () => true }));

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
