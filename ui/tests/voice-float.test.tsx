import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { CallIsland, DeskCallKey } = await import("../src/components/Call");
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

const island = (call: InstanceType<typeof Call>) => <CallIsland call={call} names={() => "Someone on another desk"} roster={[]} onOpenTeammate={() => {}} />;

describe("the call island", () => {
	test("a call to a teammate leads with their face, read on the call's pinned transport, and their name", async () => {
		const reads: Record<string, unknown>[] = [];
		const call = new Call({
			command: async (cmd, params) => {
				expect(cmd).toBe("avatar.read");
				reads.push(params);
				return { mimeType: "image/png", data: "AA==" };
			},
			subscribe: () => () => {},
		}, () => "Mack", undefined, undefined, { deskId: "desk-a", target: { personaId: "mack", name: "Mack", avatarHash: "mack-call-face" } });
		await act(async () => root.render(island(call)));
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
		expect(container.querySelector(".island")?.getAttribute("aria-label")).toBe("Call with Mack");
		expect(container.querySelector(".island-name")?.textContent).toBe("Mack");
		expect(container.querySelector(".call-status")?.textContent).toBe("Calling");
		expect(container.querySelector(".island-mark .avatar-picture")).not.toBeNull();
		expect(reads).toEqual([{ personaId: "mack", hash: "mack-call-face", offset: 0 }]);
		// Dialling, the voice has nothing to show yet; it holds and hangs up, and does not fold.
		expect(container.querySelector(".call-voice")).toBeNull();
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		expect(container.querySelector(".island")?.hasAttribute("data-folding")).toBe(false);
	});

	test("a desk call is the desk's handset, and never borrows a teammate's name", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<CallIsland call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} />));
		expect(container.querySelector(".island")?.getAttribute("aria-label")).toBe("Call with the desk");
		expect(container.querySelector(".island-name")?.textContent).toBe("Desk");
		expect(container.querySelector(".island-mark .avatar")).toBeNull();
		expect(container.querySelector(".island-mark svg")).not.toBeNull();
		expect(container.textContent).not.toContain("Mack");
	});

	test("its transcript key drops what was said down under it, and takes it away again", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(island(call)));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="What was said"]');
		expect(key?.getAttribute("aria-pressed")).toBe("false");
		expect(container.querySelector(".call-transcript")).toBeNull();
		await act(async () => key?.click());
		expect(container.querySelector(".call-transcript")?.textContent).toBe("Nothing said yet.");
		await act(async () => key?.click());
		expect(container.querySelector(".call-transcript")).toBeNull();
	});
});

describe("the desk's call key", () => {
	test("calls the desk at rest", async () => {
		let called = 0;
		await act(async () => root.render(<DeskCallKey call={null} available onCall={() => called++} />));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="Call the desk"]');
		expect(key?.disabled).toBe(false);
		await act(async () => key?.click());
		expect(called).toBe(1);
	});

	test("waits, disabled, while a teammate is on the line; is absent where the desk cannot talk", async () => {
		const teammate = new Call({ command: async () => null, subscribe: () => () => {} }, () => "Mack", undefined, undefined, { target: { personaId: "mack", name: "Mack" } });
		await act(async () => root.render(<DeskCallKey call={teammate} available onCall={() => {}} />));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="Call the desk"]');
		expect(key?.disabled).toBe(true);
		expect(key?.title).toBe("On a call with Mack");
		await act(async () => root.render(<DeskCallKey call={null} available={false} onCall={() => {}} />));
		expect(container.querySelector(".desk-call-key")).toBeNull();
	});

	test("is green on a call to the desk, and hangs it up", async () => {
		const desk = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<DeskCallKey call={desk} available onCall={() => {}} />));
		const key = container.querySelector<HTMLButtonElement>(".desk-call-key");
		expect(key?.hasAttribute("data-on")).toBe(true);
		expect(key?.getAttribute("aria-label")).toBe("Hang up on the desk");
	});
});
