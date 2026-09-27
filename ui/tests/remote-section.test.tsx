import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { RemoteSection } from "../src/components/RemoteSection";
import { wire } from "../src/wire";

// No desk, sockets, native shell, or real clock: drive the mounted UI at its wire boundary.
const epoch = 1_800_000_000_000;
const owner = { id: "owner-test", name: "Test owner", role: "owner", pairedAt: epoch };
const companion = { id: "companion-test", name: "Test companion", role: "companion", pairedAt: epoch };
const enabled = {
	enabled: true, host: "all", endpoint: "https://192.0.2.1:8788",
	endpoints: ["https://192.0.2.1:8788", "https://[2001:db8::1]:8788"],
	addresses: ["192.0.2.1", "2001:db8::1"], devices: [], error: null,
};
function legacy(address = "192.0.2.1") {
	return {
		invitation: { expiresAt: epoch + 120_000 }, qrSvg: "<svg>legacy-test</svg>",
		manual: { address, port: 8788, code: "123456" },
	};
}
function sealed(id = "invitation-test", expiresAt = epoch + 120_000) {
	return { id, expiresAt, qrSvg: `<svg>${id}</svg>` };
}
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason: unknown) => void;
	const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
	return { promise, resolve, reject };
}
type Call = { cmd: string; params: Record<string, unknown> };
let calls: Call[];
let respond: (call: Call) => unknown;
let now: number;
let intervals: Map<number, () => void>;
let root: Root | null;
let container: HTMLDivElement;
let dom: Window;
let restores: (() => void)[];
let clipboard: string[];

beforeEach(() => {
	restores = [];
	dom = new Window();
	for (const [key, value] of Object.entries({ window: dom, document: dom.document, navigator: dom.navigator, IS_REACT_ACT_ENVIRONMENT: true })) {
		const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
		Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
		restores.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
	}
	clipboard = [];
	const copy = spyOn(dom.navigator.clipboard, "writeText").mockImplementation(async (text) => { clipboard.push(text); });
	restores.push(() => copy.mockRestore());
	now = epoch;
	const date = spyOn(Date, "now").mockImplementation(() => now);
	restores.push(() => date.mockRestore());
	intervals = new Map();
	let timerId = 0;
	const schedule = spyOn(globalThis, "setInterval").mockImplementation(((callback: () => void) => {
		const id = ++timerId;
		intervals.set(id, callback);
		return id;
	}) as typeof setInterval);
	const clear = spyOn(globalThis, "clearInterval").mockImplementation((id) => { intervals.delete(Number(id)); });
	restores.push(() => schedule.mockRestore(), () => clear.mockRestore());
	calls = [];
	respond = ({ cmd, params }) => {
		if (cmd === "remote.status") return enabled;
		if (cmd === "remote.configure") return { ...enabled, ...params };
		if (cmd === "remote.revoke") return enabled;
		if (cmd === "remote.pairing") return params.id ? null : params.legacy ? legacy() : sealed();
		throw new Error(`Unexpected command ${cmd}`);
	};
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: Record<string, unknown>) => {
		const call = { cmd, params };
		calls.push(call);
		return respond(call);
	}) as typeof wire.command);
	restores.push(() => command.mockRestore());
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await unmount();
	for (const restore of restores.reverse()) restore();
	await dom.happyDOM.close();
});
async function mount() { await act(async () => { root!.render(<RemoteSection />); }); }
async function unmount() { await act(async () => { root?.unmount(); root = null; }); }
async function tick(ms = 2000) {
	await act(async () => {
		now += ms;
		for (const [id, callback] of [...intervals]) if (intervals.has(id)) callback();
	});
}
async function settle(action: () => void) { await act(async () => { action(); }); }
function text() { return container.textContent ?? ""; }
function button(label: string) {
	const found = [...container.querySelectorAll("button")].find((node) => node.textContent === label);
	if (!found) throw new Error(`Missing button ${label}: ${text()}`);
	return found;
}
async function click(label: string) { await settle(() => button(label).click()); }
function pairingCalls() { return calls.filter(({ cmd }) => cmd === "remote.pairing"); }
function polls() { return pairingCalls().filter(({ params }) => params.id && !params.cancel); }
function cancellations() { return pairingCalls().filter(({ params }) => params.cancel); }

describe("Remote settings over the wire", () => {
	test("status, enable, listen address, device roles and revoke use wire commands", async () => {
		respond = ({ cmd, params }) => cmd === "remote.status" ? { ...enabled, enabled: false }
			: cmd === "remote.configure" ? { ...enabled, ...params, devices: [owner, companion] } : enabled;
		await mount();
		expect(calls).toEqual([{ cmd: "remote.status", params: {} }]);
		expect(text()).not.toContain("Show pairing code");
		await settle(() => (container.querySelector('[role="switch"]') as HTMLInputElement).click());
		expect(calls.at(-1)).toEqual({ cmd: "remote.configure", params: { enabled: true, host: "all" } });
		expect(text()).toContain("Owner · Paired");
		expect(text()).toContain("Companion · Paired");
		expect(text()).toContain("https://[2001:db8::1]:8788");
		await settle(() => {
			const select = container.querySelector("select")!;
			select.value = "2001:db8::1";
			select.dispatchEvent(new dom.Event("change", { bubbles: true }));
		});
		expect(calls.at(-1)).toEqual({ cmd: "remote.configure", params: { enabled: true, host: "2001:db8::1" } });
		await click("Revoke access");
		expect(calls.at(-1)).toEqual({ cmd: "remote.revoke", params: { deviceId: owner.id } });
		expect(text()).not.toContain("Test owner");
	});

	for (const address of ["192.0.2.1", "2001:db8::1"]) {
		test(`manual/legacy remains the default, including copying ${address}`, async () => {
			const fallback = respond;
			respond = (call) => call.cmd === "remote.pairing" ? legacy(address) : fallback(call);
			await mount();
			expect(pairingCalls()).toHaveLength(0);
			await click("Show pairing code");
			expect(pairingCalls()).toEqual([{ cmd: "remote.pairing", params: { legacy: true, cancel: false } }]);
			expect(text()).toContain("123456");
			const expected = address.includes(":") ? `[${address}]:8788` : `${address}:8788`;
			expect(text()).toContain(expected);
			expect(container.querySelector("img")?.getAttribute("src")).toContain(encodeURIComponent("<svg>legacy-test</svg>"));
			await click("Copy address");
			expect(clipboard).toEqual([expected]);
			expect(text()).toContain("Copied");
			await tick();
			expect(polls()).toHaveLength(0);
			await unmount();
			expect(cancellations()).toHaveLength(0);
		});
	}

	test("sealed QR polls null, then links exactly once without offering a manual code", async () => {
		await mount();
		await click("Show sealed QR (v2)");
		expect(pairingCalls()[0]).toEqual({ cmd: "remote.pairing", params: { legacy: false, cancel: false } });
		expect(container.querySelector("img") !== null).toBe(true);
		expect(text()).not.toContain("Copy address");
		expect(text()).not.toContain("123456");
		await tick();
		expect(polls()).toEqual([{ cmd: "remote.pairing", params: { id: "invitation-test", cancel: false, legacy: false } }]);
		expect(text()).not.toContain("Phone linked.");
		const fallback = respond;
		respond = (call) => call.params.id && !call.params.cancel ? owner : fallback(call);
		await tick();
		expect(text()).toContain("Phone linked.");
		expect(container.querySelector("img") === null).toBe(true);
		const count = polls().length;
		await tick();
		await tick(120_000);
		expect(polls()).toHaveLength(count);
		expect(text()).not.toContain("This code expired");
	});

	test("expired sealed QR disappears and never polls again", async () => {
		await mount();
		await click("Show sealed QR (v2)");
		await tick(120_000);
		expect(text()).toContain("This code expired");
		expect(container.querySelector("img") === null).toBe(true);
		expect(polls()).toHaveLength(0);
		await tick();
		expect(polls()).toHaveLength(0);
	});

	for (const action of ["replace", "disable", "unmount"] as const) {
		test(`${action} cancels the sealed invitation and ignores its in-flight poll`, async () => {
			const pending = deferred<unknown>();
			const fallback = respond;
			respond = (call) => call.params.id && !call.params.cancel ? pending.promise : fallback(call);
			await mount();
			await click("Show sealed QR (v2)");
			await tick();
			if (action === "replace") await click("New pairing code");
			if (action === "disable") await settle(() => (container.querySelector('[role="switch"]') as HTMLInputElement).click());
			if (action === "unmount") await unmount();
			expect(cancellations()).toEqual([{ cmd: "remote.pairing", params: { id: "invitation-test", cancel: true, legacy: false } }]);
			await settle(() => pending.resolve(owner));
			expect(text()).not.toContain("Phone linked.");
			expect(polls()).toHaveLength(1);
			if (action === "replace") expect(text()).toContain("123456");
			if (action === "unmount") expect(intervals.size).toBe(0);
		});
	}

	test("slow polls are serialized rather than piling up every two seconds", async () => {
		const pending = deferred<unknown>();
		const fallback = respond;
		respond = (call) => call.params.id && !call.params.cancel ? pending.promise : fallback(call);
		await mount();
		await click("Show sealed QR (v2)");
		await tick();
		await tick();
		await tick();
		expect(polls()).toHaveLength(1);
		await settle(() => pending.resolve(null));
		await tick();
		expect(polls()).toHaveLength(2);
	});

	test("an invitation returned after unmount is cancelled instead of orphaned", async () => {
		const pending = deferred<unknown>();
		const fallback = respond;
		respond = (call) => call.cmd === "remote.pairing" && !call.params.id ? pending.promise : fallback(call);
		await mount();
		await click("Show sealed QR (v2)");
		await unmount();
		await settle(() => pending.resolve(sealed()));
		expect(cancellations()).toEqual([{ cmd: "remote.pairing", params: { id: "invitation-test", cancel: true, legacy: false } }]);
		expect(intervals.size).toBe(0);
	});

	test("replacement invalidates the old QR and poll before the new request finishes", async () => {
		const oldPoll = deferred<unknown>();
		const replacement = deferred<unknown>();
		const fallback = respond;
		respond = (call) => call.params.id && !call.params.cancel ? oldPoll.promise : fallback(call);
		await mount();
		await click("Show sealed QR (v2)");
		await tick();
		respond = (call) => call.cmd === "remote.pairing" && !call.params.id ? replacement.promise : fallback(call);
		await click("Show sealed QR (v2)");
		expect(container.querySelector("img") === null).toBe(true);
		expect(cancellations()).toHaveLength(1);
		await settle(() => oldPoll.reject(new Error("stale poll failure")));
		expect(text()).not.toContain("stale poll failure");
		await settle(() => replacement.resolve(sealed("replacement-test")));
		expect(container.querySelector("img")?.getAttribute("src")).toContain("replacement-test");
		expect(text()).not.toContain("Phone linked.");
	});

	test("a refresh started before configure cannot overwrite the newer settings", async () => {
		const oldStatus = deferred<unknown>();
		await mount();
		const fallback = respond;
		respond = (call) => call.cmd === "remote.status" ? oldStatus.promise : fallback(call);
		await tick();
		await settle(() => (container.querySelector('[role="switch"]') as HTMLInputElement).click());
		await settle(() => oldStatus.resolve(enabled));
		expect((container.querySelector('[role="switch"]') as HTMLInputElement).checked).toBe(false);
		expect(text()).not.toContain("Show pairing code");
	});

	test("slow status refreshes do not overlap and regress device state", async () => {
		const pending = deferred<unknown>();
		respond = () => pending.promise;
		await mount();
		await tick();
		await tick();
		expect(calls.filter(({ cmd }) => cmd === "remote.status")).toHaveLength(1);
		await settle(() => pending.resolve({ ...enabled, devices: [owner] }));
		expect(text()).toContain("Test owner");
	});

	test("wire failures are visible and a failed action releases the controls", async () => {
		await mount();
		respond = () => { throw new Error("test wire refused"); };
		await click("Show sealed QR (v2)");
		expect(text()).toContain("test wire refused");
		expect(button("Show pairing code").disabled).toBe(false);
		expect(container.querySelector("img") === null).toBe(true);
		respond = ({ cmd }) => cmd === "remote.pairing" ? legacy() : enabled;
		await click("Show pairing code");
		expect(text()).not.toContain("test wire refused");
		expect(text()).toContain("123456");
	});
});
