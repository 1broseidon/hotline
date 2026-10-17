import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import type { Root } from "react-dom/client";
import { activeDeskId, allDesks, replaceDesks, setActiveDesk, type Desk } from "../src/desks";
import { wire } from "../src/wire";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { AddDesk, sshTarget } = await import("../src/components/AddDesk");

describe("an SSH target", () => {
	test("is user@host or a config host", () => {
		expect(sshTarget("ada@studio.example")).toBe(true);
		expect(sshTarget("studio")).toBe(true);
	});

	test("never starts with a dash, has no spaces and is not empty, as the shell requires", () => {
		expect(sshTarget("-oProxyCommand=evil")).toBe(false);
		expect(sshTarget("-p")).toBe(false);
		expect(sshTarget("ada@studio example")).toBe(false);
		expect(sshTarget("")).toBe(false);
	});
});

// Each desk's core is a socket the real Wire dials; the shell answers at Tauri's invoke.
const local: Desk = { id: "local", name: "This computer", kind: "local", origin: "http://127.0.0.1:2", token: "t" };
const grizzly: Desk = { id: "grizzly", name: "Grizzly", kind: "remote", origin: "http://127.0.0.1:3", token: "t", state: "open" };
const muskox = { deskId: "muskox", name: "Muskox", url: "https://muskox.example", deskKey: "k", serves: ["relay"] };
const status = { enabled: true, host: "all", endpoint: null, endpoints: [], addresses: [], devices: [], error: null, publicUrl: null };

type Call = { cmd: string; params: Record<string, unknown> };
let relays: (typeof muskox)[];
let chosen: string | null;
let paired: { id: string; relay: boolean };
let closed: number;

/** A core that answers every command at once: the local one holds this computer's relays, a server's says nothing of them. */
class FakeSocket {
	static readonly OPEN = 1;
	static all: FakeSocket[] = [];
	readyState = 1;
	calls: Call[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) { FakeSocket.all.push(this); }
	send(data: string) {
		const { id, cmd, params } = JSON.parse(data) as { id: number; cmd?: string; params: Record<string, unknown> };
		if (cmd === undefined) return;
		this.calls.push({ cmd, params });
		const answer = (result: unknown) => queueMicrotask(() => this.onmessage?.({ data: JSON.stringify({ id, ok: true, result }) }));
		if (cmd === "client.hello") return answer({ capabilities: ["threads2"] });
		if (!this.url.startsWith(`${local.origin}/`)) return queueMicrotask(() => this.onmessage?.({ data: JSON.stringify({ id, ok: false, error: `${cmd} is this computer's` }) }));
		if (cmd === "remote.relays") return answer(relays);
		if (cmd === "remote.relay") chosen = typeof params.deskId === "string" ? params.deskId : null;
		answer({ ...status, relay: chosen === null ? null : { deskId: chosen, name: chosen, url: "https://relay.example", error: null } });
	}
	close() { this.readyState = 3; }
}
/** Scroll measures its content; nothing here lays out. */
class NoResize { observe() {} disconnect() {} }
function core(desk: Desk) { return FakeSocket.all.find((socket) => socket.url.startsWith(`${desk.origin}/`))!; }

let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];

beforeEach(async () => {
	restores = [];
	dom = new Window();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, WebSocket: FakeSocket, ResizeObserver: NoResize, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
	}
	FakeSocket.all = [];
	relays = [];
	chosen = null;
	closed = 0;
	(dom as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
		invoke: async (cmd: string) => {
			if (cmd !== "desk_pair_link") throw new Error(`Unexpected shell command ${cmd}`);
			// The shell keeps a relay in relays.json; a desk joins the list it sends.
			if (paired.relay) relays = [...relays, muskox];
			else replaceDesks([...allDesks(), grizzly]);
			return paired.id;
		},
	};
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => { root?.unmount(); root = null; });
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});

/** Opens a window on `desks`, every desk's core connected, then pastes `link` into Add a server and pairs. */
async function pairFrom(desks: Desk[], link: string) {
	replaceDesks(desks);
	wire.connect();
	for (const socket of FakeSocket.all) socket.onopen?.();
	await act(async () => { root!.render(createElement(AddDesk, { onClose: () => closed++ })); });
	const field = container.querySelector("#desk-link") as HTMLTextAreaElement;
	await act(async () => {
		Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype, "value")!.set!.call(field, link);
		field.dispatchEvent(new dom.Event("input", { bubbles: true }));
	});
	const pair = [...container.querySelectorAll("button")].find((node) => node.textContent === "Pair")!;
	await act(async () => {
		pair.click();
		for (let turn = 0; turn < 10; turn++) await new Promise((resolve) => setTimeout(resolve, 0));
	});
}
function said() { return container.querySelector('[role="status"]')?.textContent ?? null; }

describe("Add a server", () => {
	test("a relay link says it added a relay, chooses it when none is chosen, and never lists a desk", async () => {
		paired = { id: "muskox", relay: true };
		await pairFrom([local], "hotline://pair?p=relay");
		expect(said()).toBe("Added Muskox as a relay. Phones also reach this desk through it.");
		expect(core(local).calls.at(-1)).toEqual({ cmd: "remote.relay", params: { deskId: "muskox" } });
		expect(allDesks().map((desk) => desk.id)).toEqual(["local"]);
		expect(activeDeskId()).toBe("local");
		expect(closed).toBe(0);
	});

	test("a relay link while a relay is chosen leaves the choice and points to where to change it, asking this computer from a server's desk", async () => {
		paired = { id: "muskox", relay: true };
		chosen = "grizzly";
		replaceDesks([local, grizzly]);
		setActiveDesk("grizzly");
		await pairFrom([local, grizzly], "hotline://pair?p=relay");
		expect(said()).toBe("Added Muskox as a relay. Choose it under Settings › Remote › Relay.");
		expect(chosen).toBe("grizzly");
		expect(core(local).calls.some((call) => call.cmd === "remote.relay")).toBe(false);
		expect(core(grizzly).calls.map((call) => call.cmd)).toEqual(["client.hello"]);
		expect(activeDeskId()).toBe("grizzly");
		expect(closed).toBe(0);
	});

	test("a desk link switches to the new desk and closes, as before", async () => {
		paired = { id: "grizzly", relay: false };
		await pairFrom([local], "hotline://pair?p=desk");
		expect(said()).toBeNull();
		expect(activeDeskId()).toBe("grizzly");
		expect(closed).toBe(1);
		expect(core(local).calls.some((call) => call.cmd === "remote.relay")).toBe(false);
	});
});
