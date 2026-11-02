import { expect, spyOn, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

const gate = <T = null,>() => {
	let resolve: (value: T) => void = () => {};
	let reject: (error: Error) => void = () => {};
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
};
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("sends are kept per conversation: one settling never wipes another that is still out", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 1000);
	const a = gate();
	const b = gate();
	sends.send("ada", { text: "A", attachments: [] }, () => a.promise);
	sends.send("ada", { text: "B", attachments: [] }, () => b.promise);
	sends.send("grace", { text: "G", attachments: [] }, async () => null);
	expect(sends.get("ada").lines.map((l) => l.text)).toEqual(["A", "B"]);
	a.resolve(null);
	await tick();
	expect(sends.get("ada").lines.map((l) => l.text)).toEqual(["B"]);
	await tick();
	expect(sends.get("grace").lines).toEqual([]);
	b.resolve(null);
});

test("sends in one conversation reach the core in the order they were made", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 1);
	const order: string[] = [];
	const slow = gate();
	sends.send("ada", { text: "first", attachments: [] }, async () => { await slow.promise; order.push("first"); });
	sends.send("ada", { text: "second", attachments: [] }, async () => { order.push("second"); });
	await tick();
	expect(order).toEqual([]);
	slow.resolve(null);
	await tick();
	expect(order).toEqual(["first", "second"]);
});

test("a refused send is kept for its conversation until it is taken, after the person has gone elsewhere", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 5);
	const reply = { eventId: "e1", text: "quoted" };
	sends.send("ada", { text: "one", attachments: [], reply }, async () => { throw new Error("No session"); });
	sends.send("ada", { text: "two", attachments: [] }, async () => { throw new Error("No session"); });
	await tick();
	await tick();
	// Nobody is looking at ada: grace's conversation has nothing.
	expect(sends.get("grace").returned).toEqual([]);
	expect(sends.get("ada").lines).toEqual([]);
	expect(sends.get("ada").notice).toBe("No session");
	const back = sends.take("ada");
	expect(back?.text).toBe("one\n\ntwo");
	expect(back?.reply).toEqual(reply);
	expect(back?.message).toBe("No session");
	expect(sends.take("ada")).toBeUndefined();
});

test("a socket drop on a send the core may have run keeps it on screen and does not hand the words back", async () => {
	const { Sends, DROPPED } = await import("../src/sends");
	const sends = new Sends(() => 10);
	sends.send("ada", { text: "maybe", attachments: [] }, async () => { throw new Error(DROPPED); });
	await tick();
	await tick();
	const state = sends.get("ada");
	expect(state.returned).toEqual([]);
	expect(state.lines.map((l) => [l.text, l.phase])).toEqual([["maybe", "unsure"]]);
	expect(state.notice).toContain("dropped");
	const landed: TranscriptEvent = { kind: "user", id: "u1", ts: 11, text: "maybe" };
	sends.landed("ada", [landed]);
	expect(sends.get("ada").lines).toEqual([]);
});

test("a send not made because the socket is down is handed back like any refusal", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 10);
	sends.send("ada", { text: "unsent", attachments: [] }, async () => { throw new Error("Hotline is not connected."); });
	await tick();
	await tick();
	expect(sends.take("ada")?.text).toBe("unsent");
});

test("a success clears the notice an earlier refusal left", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 10);
	sends.send("ada", { text: "x", attachments: [] }, async () => { throw new Error("Nope"); });
	await tick();
	await tick();
	expect(sends.get("ada").notice).toBe("Nope");
	sends.send("ada", { text: "y", attachments: [] }, async () => null);
	await tick();
	await tick();
	expect(sends.get("ada").notice).toBeNull();
});

test("two starts asked for at once are one start", async () => {
	const { Sends } = await import("../src/sends");
	const sends = new Sends(() => 1);
	let starts = 0;
	const slow = gate();
	const make = () => { starts++; return slow.promise; };
	const one = sends.once("ada", make);
	const two = sends.once("ada", make);
	expect(starts).toBe(1);
	slow.resolve(null);
	await Promise.all([one, two]);
	await sends.once("ada", make);
	expect(starts).toBe(2);
});

test("each line that landed hides one placeholder, so the same words sent twice show twice until both land", async () => {
	const { unlanded } = await import("../src/sends");
	const line = (id: number, at: number) => ({ id, text: "hi", attachments: [], at, phase: "sending" as const });
	const user = (id: string, ts: number): TranscriptEvent => ({ kind: "user", id, ts, text: "hi" });
	const lines = [line(1, 1000), line(2, 1500)];
	expect(unlanded(lines, []).map((l) => l.id)).toEqual([1, 2]);
	expect(unlanded(lines, [user("a", 1100)]).map((l) => l.id)).toEqual([2]);
	expect(unlanded(lines, [user("a", 1100), user("b", 1600)]).map((l) => l.id)).toEqual([]);
});

test("a composer given returned words keeps what was typed since, and puts the returned ones first", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Composer } = await import("../src/components/Composer");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const props = { personaId: "merge-test", name: "Ada", state: "ready" as const, replyQuote: null, onSend() {}, onCancel() {}, onClearReply() {} };
	const area = () => container.querySelector("textarea")!;
	const set = async (value: string) => act(async () => {
		const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
		setter.call(area(), value);
		area().dispatchEvent(new window.Event("input", { bubbles: true }));
	});
	try {
		await act(async () => root.render(<Composer {...props} />));
		await set("typed since");
		await act(async () => root.render(<Composer {...props} refill={{ text: "returned", attachments: [], nonce: 7, merge: true }} />));
		expect(area().value).toBe("returned\n\ntyped since");
		await act(async () => root.render(<Composer {...props} refill={{ text: "replaced", attachments: [], nonce: 8 }} />));
		expect(area().value).toBe("replaced");
	} finally {
		await act(async () => root.unmount());
		await teardown();
	}
});
void spyOn;
