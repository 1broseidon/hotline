import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import type { Window } from "happy-dom";
import { act } from "react";
import type { Root } from "react-dom/client";
import { DesksSection } from "../src/components/DesksSection";
import { RemoteSection } from "../src/components/RemoteSection";
import { replaceDesks, wireFor, type Desk } from "../src/desks";
import { installDom } from "./dom";

// The desktop's own core answers at its own wire's `command`; the shell answers at
// Tauri's invoke; a paired server's bridge is a socket the test speaks for.
const local: Desk = { id: "local", name: "This computer", kind: "local", origin: "http://127.0.0.1:2", token: "t" };
const status = { enabled: true, host: "all", endpoint: "https://192.0.2.1:8788", endpoints: ["https://192.0.2.1:8788"], addresses: ["192.0.2.1"], devices: [], error: null, publicUrl: null };
const muskox = { deskId: "muskox", name: "Muskox", url: "https://muskox.example", deskKey: "k" };

class FakeSocket {
	static all: FakeSocket[] = [];
	sent: Record<string, unknown>[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) { FakeSocket.all.push(this); }
	send(data: string) { this.sent.push(JSON.parse(data) as Record<string, unknown>); }
	close() { this.onclose = null; }
	say(frame: Record<string, unknown>) { this.onmessage?.({ data: JSON.stringify(frame) }); }
}

type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let shell: Call[];
let relays: (typeof muskox)[];
let chosen: string | null;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];
let teardown: () => Promise<void>;
const spied = new WeakSet<object>();

function remoteDesk(id: string, extra: Partial<Desk> = {}): Desk {
	return { id, name: id === "grizzly" ? "Grizzly" : id, kind: "remote", origin: `http://127.0.0.1:${id.length + 10}`, token: "t", state: "open", ...extra };
}
function statusNow() {
	const relay = chosen === null ? null : { deskId: chosen, name: chosen === "muskox" ? "Muskox" : "Grizzly", url: `https://relay.example/${chosen}`, error: null };
	return { ...status, relay };
}

beforeEach(async () => {
	restores = [];
	({ dom, teardown } = installDom({ WebSocket: FakeSocket }));
	FakeSocket.all = [];
	const schedule = spyOn(globalThis, "setInterval").mockImplementation((() => 0) as unknown as typeof setInterval);
	restores.push(() => schedule.mockRestore());
	calls = [];
	shell = [];
	relays = [];
	chosen = null;
	(dom as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
		invoke: async (cmd: string, args: Record<string, unknown>) => {
			shell.push({ cmd, params: args });
			if (cmd === "desk_pair_link") { relays = [...relays, muskox]; return muskox.deskId; }
			if (cmd === "relay_forget") { relays = relays.filter((one) => one.deskId !== args.deskId); return null; }
			// The shell says so by sending the list again.
			if (cmd === "desk_decline_relay") { replaceDesks([local, remoteDesk(String(args.deskId), { relayDeclined: true })]); return null; }
			if (cmd === "desk_move_to_relay") {
				const moved = { ...muskox, deskId: String(args.deskId), name: "Grizzly" };
				relays = [...relays, moved];
				replaceDesks([local]);
				return moved;
			}
			throw new Error(`Unexpected shell command ${cmd}`);
		},
	};
	// React reads the DOM when it loads, so it loads once the DOM is here (as search.test.tsx does).
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => { root?.unmount(); root = null; });
	replaceDesks([]);
	for (const restore of restores.reverse()) restore();
	await teardown();
});

async function settle(action: () => unknown = () => {}) { await act(async () => { await action(); }); }
function core(cmd: string, params: Record<string, unknown>) {
	calls.push({ cmd, params });
	if (cmd === "remote.relays") return relays;
	if (cmd === "remote.relay") chosen = typeof params.deskId === "string" ? params.deskId : null;
	if (cmd === "remote.status" || cmd === "remote.relay") return statusNow();
	if (cmd === "desks.links") return [];
	throw new Error(`Unexpected command ${cmd}`);
}
/** The desktop's own core answers on its own connection, which `wire` reaches while it is the active desk. */
async function mount(node = <RemoteSection />) {
	const own = wireFor(local.id);
	if (!spied.has(own)) {
		spied.add(own);
		const command = spyOn(own, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => core(cmd, params)) as typeof own.command);
		restores.push(() => command.mockRestore());
	}
	await settle(() => root!.render(node));
}
function text() { return container.textContent ?? ""; }
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found;
}
async function click(label: string) { await settle(() => button(label).click()); }
async function openMenu(label: string) {
	await settle(() => (container.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement).click());
	return [...document.querySelectorAll<HTMLElement>('[role="menuitem"], [role="menuitemradio"]')];
}
function offer() { return container.querySelector('[aria-label="Relay only"]'); }

/** The server's bridge opens: its hello says what it runs, the window's hello is taken, and the roster answers. */
async function serverSays(serves: string[] | undefined, roster: unknown[]) {
	const socket = FakeSocket.all.at(-1)!;
	await settle(() => socket.onopen?.());
	const hello = socket.sent.find((frame) => frame.cmd === "client.hello")!;
	await settle(() => {
		socket.say(serves === undefined ? { type: "hello" } : { type: "hello", serves });
		socket.say({ id: hello.id, ok: true, result: { capabilities: ["threads2"] } });
	});
	const sub = socket.sent.find((frame) => (frame.sub as { view?: string } | undefined)?.view === "roster")!;
	await settle(() => socket.say({ sub: sub.id, snapshot: roster }));
	return { socket, sub: sub.id as number };
}

describe("the Relay picker", () => {
	test("lists paired desks and relay servers together, and labels the relays by how this desktop holds them", async () => {
		replaceDesks([local, remoteDesk("grizzly")]);
		relays = [muskox];
		await mount();
		const items = await openMenu("Relay");
		expect(items.map((item) => item.textContent)).toEqual(["None", "Grizzly", "MuskoxRelay"]);
		await settle(() => items[2]!.click());
		expect(calls.at(-1)).toEqual({ cmd: "remote.relay", params: { deskId: "muskox" } });
		expect(text()).toContain("Phones also reach this desk through Muskox.");
	});

	test("a relay-only server is never a desk in the window", async () => {
		replaceDesks([local, remoteDesk("grizzly")]);
		relays = [muskox];
		await mount(<DesksSection onAddDesk={() => {}} />);
		expect(text()).toContain("Grizzly");
		expect(text()).not.toContain("Muskox");
	});

	test("Add a relay pairs from the relay link and stands in on it", async () => {
		replaceDesks([local]);
		await mount();
		expect(container.querySelector('[aria-label="Relay"]')).toBeNull();
		await settle(() => [...container.querySelectorAll("button")].find((node) => node.textContent?.startsWith("Relay servers"))!.click());
		expect(text()).toContain("hotline pair --relay --link");
		const input = container.querySelector('[aria-label="Relay link"]') as HTMLInputElement;
		await settle(() => {
			Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(input, "hotline://pair?p=relay");
			input.dispatchEvent(new dom.Event("input", { bubbles: true }));
		});
		await click("Pair");
		expect(shell).toEqual([{ cmd: "desk_pair_link", params: { link: "hotline://pair?p=relay" } }]);
		expect(calls.at(-1)).toEqual({ cmd: "remote.relay", params: { deskId: "muskox" } });
		expect(text()).toContain("Phones also reach this desk through Muskox.");
	});

	test("forgetting the chosen relay first stops standing in on it", async () => {
		replaceDesks([local]);
		relays = [muskox];
		chosen = "muskox";
		await mount();
		await settle(() => [...container.querySelectorAll("button")].find((node) => node.textContent?.startsWith("Relay servers"))!.click());
		await click("Forget");
		const relayCall = calls.findIndex((call) => call.cmd === "remote.relay");
		expect(calls[relayCall]).toEqual({ cmd: "remote.relay", params: {} });
		expect(shell).toEqual([{ cmd: "relay_forget", params: { deskId: "muskox" } }]);
		expect(chosen).toBeNull();
		expect(text()).not.toContain("Muskox");
	});
});

describe("the offer to keep a server only as a relay", () => {
	test("shows for the chosen desk that runs a relay and has no teammates here, and moving it keeps it as the relay", async () => {
		replaceDesks([local, remoteDesk("grizzly")]);
		chosen = "grizzly";
		await mount();
		expect(offer()).toBeNull();
		await serverSays(["agents", "relay"], []);
		expect(offer()?.textContent).toContain("Use Grizzly only as your relay? Your desktop will no longer be able to run or read anything on it.");
		await click("Use only as relay");
		expect(shell).toEqual([{ cmd: "desk_move_to_relay", params: { deskId: "grizzly" } }]);
		expect(offer()).toBeNull();
		const items = await openMenu("Relay");
		// Grizzly still runs agents, so it is labelled by this desktop's grant, not as a relay only.
		expect(items.map((item) => item.textContent)).toEqual(["None", "GrizzlyRelay"]);
	});

	test("declining is remembered and the offer does not come back", async () => {
		replaceDesks([local, remoteDesk("grizzly")]);
		chosen = "grizzly";
		await mount();
		await serverSays(["agents", "relay"], []);
		await click("Keep as desk");
		expect(shell).toEqual([{ cmd: "desk_decline_relay", params: { deskId: "grizzly" } }]);
		expect(offer()).toBeNull();
		const sockets = FakeSocket.all.length;
		await settle(() => root!.render(<></>));
		await mount();
		// A declined desk is not even asked again.
		expect(FakeSocket.all.length).toBe(sockets);
		expect(offer()).toBeNull();
	});

	for (const [why, serves, roster, choose] of [
		["the server does not run a relay", ["agents"], [], true],
		["the server says nothing of what it runs", undefined, [], true],
		["this desktop has teammates there", ["agents", "relay"], [{ persona: { id: "p1" } }], true],
		["the desk is not the chosen relay", ["agents", "relay"], [], false],
	] as const) {
		test(`never shows when ${why}`, async () => {
			replaceDesks([local, remoteDesk("grizzly")]);
			chosen = choose ? "grizzly" : null;
			await mount();
			if (choose) await serverSays(serves as string[] | undefined, [...roster]);
			expect(offer()).toBeNull();
			expect(shell).toEqual([]);
		});
	}

	test("a teammate made there takes the offer away", async () => {
		replaceDesks([local, remoteDesk("grizzly")]);
		chosen = "grizzly";
		await mount();
		const { socket, sub } = await serverSays(["agents", "relay"], []);
		expect(offer()).not.toBeNull();
		await settle(() => socket.say({ sub, event: { persona: { id: "p1" } } }));
		expect(offer()).toBeNull();
	});
});
