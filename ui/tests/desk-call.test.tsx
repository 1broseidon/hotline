import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { DeskCall } = await import("../src/components/Call");
const { Call } = await import("../src/voice/call");
const { forgetAvatars } = await import("../src/avatars");

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	forgetAvatars();
	dom = new Window();
	restores = [];
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => descriptor ? Object.defineProperty(globalThis, key, descriptor) : Reflect.deleteProperty(globalThis, key));
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
	forgetAvatars();
	await dom.happyDOM.close();
});

describe("the desk's line at the foot of the rail", () => {
	test("is a call key at rest, which calls the desk", async () => {
		let called = 0;
		await act(async () => root.render(<DeskCall call={null} available onCall={() => called++} roster={[]} compact={false} />));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="Call the desk"]');
		expect(key?.disabled).toBe(false);
		await act(async () => key?.click());
		expect(called).toBe(1);
	});

	test("waits, disabled, while a teammate is on the line, and is absent where the desk cannot talk", async () => {
		const teammate = new Call({ command: async () => null, subscribe: () => () => {} }, () => "Mack", undefined, undefined, { target: { personaId: "mack", name: "Mack" } });
		await act(async () => root.render(<DeskCall call={teammate} available onCall={() => {}} roster={[]} compact={false} />));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="Call the desk"]');
		expect(key?.disabled).toBe(true);
		expect(key?.title).toBe("On a call with Mack");
		await act(async () => root.render(<DeskCall call={null} available={false} onCall={() => {}} roster={[]} compact={false} />));
		expect(container.querySelector(".desk-call-key")).toBeNull();
	});

	test("on a call to the desk it is the call's card, and in the faces-only rail the ringed handset over Hang up", async () => {
		const desk = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<DeskCall call={desk} available onCall={() => {}} roster={[]} compact={false} />));
		expect(container.querySelector('[aria-label="Call the desk"]')).toBeNull();
		expect(container.querySelector(".desk-call .call-bar-status")?.textContent).toBe("DeskCalling");
		expect(container.querySelector('[aria-label="What was said"]')).not.toBeNull();
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		await act(async () => root.render(<DeskCall call={desk} available onCall={() => {}} roster={[]} compact />));
		expect(container.querySelector(".desk-call-compact .desk-call-mark")).not.toBeNull();
		expect(container.querySelector(".desk-call-compact [aria-label=\"Hang up\"]")).not.toBeNull();
		expect(container.querySelector(".desk-call-compact .call-bar-status")).toBeNull();
	});
});
