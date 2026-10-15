import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { CallBar } = await import("../src/components/Call");
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
		expect(container.textContent).toContain("Mack · Calling");
		expect(container.querySelector(".avatar-picture")).not.toBeNull();
		expect(reads).toEqual([{ personaId: "mack", hash: "mack-call-face", offset: 0 }]);
	});

	test("a desk call keeps the desk identity instead of borrowing the selected teammate", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} });
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} />));
		expect(container.querySelector("aside")?.getAttribute("aria-label")).toBe("Call with the desk");
		expect(container.querySelector(".avatar")).toBeNull();
		expect(container.querySelector(".call-toad")).not.toBeNull();
		expect(container.textContent).not.toContain("Mack");
	});

	test("under the teammate's own band it leaves the face to the band, and atop another pane it is a card of its own", async () => {
		const call = new Call({ command: async () => null, subscribe: () => () => {} }, () => "Mack", undefined, undefined, { target: { personaId: "mack", name: "Mack" } });
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} bare />));
		expect(container.querySelector(".call-bar-mark")).toBeNull();
		expect(container.textContent).toContain("Mack · Calling");
		await act(async () => root.render(<CallBar call={call} names={() => "Mack"} roster={[]} onOpenTeammate={() => {}} alone />));
		expect(container.querySelector(".call-bar-mark .avatar")).not.toBeNull();
		expect(container.querySelector("aside")?.className).toContain("call-bar-alone");
		// Live, it holds and hangs up, and does not fold.
		expect(container.querySelector('[aria-label="Hang up"]')).not.toBeNull();
		expect(container.querySelector("aside")?.hasAttribute("data-folding")).toBe(false);
	});
});

