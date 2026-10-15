import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { CallBar, DeskCallCard, DeskCallKey } = await import("../src/components/Call");
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

describe("the call strip's identity", () => {
	test("a direct call names its chosen teammate and reads their PNG on the call's pinned transport", async () => {
		const reads: Record<string, unknown>[] = [];
		const call = new Call({
			command: async (cmd, params) => {
				expect(cmd).toBe("avatar.read");
				reads.push(params);
				return { mimeType: "image/png", data: "AA==" };
			},
			subscribe: () => () => {},
		}, () => "Mack", undefined, undefined, { deskId: "desk-a", target: { personaId: "mack", name: "Mack", avatarHash: "mack-call-face" } });
		await act(async () => {
			root.render(<CallBar call={call} names={() => "Someone on another desk"} roster={[]} onOpenTeammate={() => {}} />);
		});
		await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
		expect(container.querySelector("aside")?.getAttribute("aria-label")).toBe("Call with Mack");
		expect(container.querySelector(".call-bar-status")?.textContent).toBe("MackCalling");
		expect(container.querySelector(".avatar-picture")).not.toBeNull();
		expect(reads).toEqual([{ personaId: "mack", hash: "mack-call-face", offset: 0 }]);
	});

	test("a desk call keeps the desk identity instead of borrowing the selected teammate", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} />));
		expect(container.querySelector("aside")?.getAttribute("aria-label")).toBe("Call with the desk");
		expect(container.querySelector(".avatar")).toBeNull();
		expect(container.querySelector(".call-bar-mark svg")).not.toBeNull();
		expect(container.textContent).not.toContain("Mack");
	});

	test("under the teammate's own band it leaves the face to the band, and atop another pane it is a card of its own", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} }, () => "Mack", undefined, undefined, { target: { personaId: "mack", name: "Mack" } });
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} bare />));
		// The face and the status are the band's there: the strip holds the keys.
		expect(container.querySelector(".call-bar-mark")).toBeNull();
		expect(container.querySelector(".call-bar-status")).toBeNull();
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} alone />));
		expect(container.querySelector(".call-bar-mark .avatar")).not.toBeNull();
		expect(container.querySelector("aside")?.className).toContain("call-bar-alone");
		// Dialling, the voice has nothing to show yet.
		expect(container.querySelector(".call-voice")).toBeNull();
		// Live, it holds and hangs up, and does not fold.
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		expect(container.querySelector("aside")?.hasAttribute("data-folding")).toBe(false);
	});

	test("its transcript key opens what was said under it, and closes it again", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} }, () => "Mack", undefined, undefined, { target: { personaId: "mack", name: "Mack" } });
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} bare />));
		const key = container.querySelector<HTMLButtonElement>('[aria-label="What was said"]');
		expect(key?.getAttribute("aria-pressed")).toBe("false");
		expect(container.querySelector(".call-transcript")).toBeNull();
		await act(async () => key?.click());
		expect(key?.getAttribute("aria-pressed")).toBe("true");
		expect(container.querySelector(".call-transcript")?.textContent).toBe("Nothing said yet.");
		await act(async () => key?.click());
		expect(container.querySelector(".call-transcript")).toBeNull();
	});
});

describe("a call to the desk, over the rail's dock", () => {
	test("is the call's card, and in the faces-only rail the ringed handset over Hang up", async () => {
		const desk = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<DeskCallCard call={desk} roster={[]} compact={false} />));
		expect(container.querySelector(".desk-call .call-bar-status")?.textContent).toBe("DeskCalling");
		expect(container.querySelector('[aria-label="What was said"]')).not.toBeNull();
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		await act(async () => root.render(<DeskCallCard call={desk} roster={[]} compact />));
		expect(container.querySelector(".desk-call-compact .desk-call-mark")).not.toBeNull();
		expect(container.querySelector(".desk-call-compact .call-bar-status")).toBeNull();
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

describe("a call's trouble", () => {
	test("is named in two or three words, with the whole sentence behind its info key", async () => {
		const { troubleWords } = await import("../src/components/Call");
		expect(troubleWords("No microphone found. Connect one or pick it in your computer's sound settings, then try again.")).toBe("No microphone");
		expect(troubleWords("The microphone is busy or not working. If another app is using it, close it, then try again.")).toBe("Microphone busy");
		expect(troubleWords("Hotline can't use the microphone. Turn it on for Hotline in System Settings › Privacy & Security › Microphone, then try again.")).toBe("Microphone blocked");
		expect(troubleWords("Hotline can't hear the microphone any more. Call again when it's back.")).toBe("Microphone lost");
		expect(troubleWords("This desk needs an update before it can call a teammate directly.")).toBe("Desk needs update");
		expect(troubleWords("Something nobody planned for.")).toBe("Call failed");
		for (const words of ["No microphone", "Microphone busy", "Desk needs update", "Call failed"]) expect(words.split(" ").length).toBeLessThanOrEqual(3);
	});
});
