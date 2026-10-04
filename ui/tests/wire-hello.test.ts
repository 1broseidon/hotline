import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { Wire, type Connection } from "../src/wire";

/** A socket the test drives: it records what the window sends and says what the core answers. */
class FakeSocket {
	static readonly OPEN = 1;
	static all: FakeSocket[] = [];
	readyState = 1;
	sent: Record<string, unknown>[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((message: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(readonly url: string) {
		FakeSocket.all.push(this);
	}
	send(data: string) {
		this.sent.push(JSON.parse(data) as Record<string, unknown>);
	}
	close() {
		if (this.readyState === 3) return;
		this.readyState = 3;
		this.onclose?.();
	}
	/** The core answers the window's last command. */
	async answer(frame: Record<string, unknown>) {
		this.onmessage?.({ data: JSON.stringify(frame) });
		// The window reacts to an answer on the promise's next turn.
		await new Promise((resolve) => setTimeout(resolve, 0));
	}
}

const real = globalThis.WebSocket;
beforeEach(() => {
	FakeSocket.all = [];
	(globalThis as { WebSocket: unknown }).WebSocket = FakeSocket;
});
afterEach(() => {
	(globalThis as { WebSocket: unknown }).WebSocket = real;
});

/** A wire with one subscription waiting, connected to a fake core, and the states it reports. */
function dial() {
	const wire = new Wire({ origin: "ws://core", token: "t" });
	const states: Connection[] = [];
	wire.onConnection((state) => states.push(state));
	wire.subscribe({ view: "roster" }, { snapshot() {}, event() {} });
	wire.connect();
	const socket = FakeSocket.all[0]!;
	socket.onopen?.();
	const hello = socket.sent.find((frame) => frame["cmd"] === "client.hello");
	return { wire, states, socket, hello: hello! };
}

describe("the window's hello to the core", () => {
	test("opens the connection and asks for everything again once the core takes threads2", async () => {
		const { wire, states, socket, hello } = dial();
		expect(hello["params"]).toEqual({ capabilities: ["threads2"] });
		expect(socket.sent.some((frame) => "sub" in frame)).toBe(false);
		await socket.answer({ id: hello["id"], ok: true, result: { capabilities: ["threads", "threads2"] } });
		expect(states.at(-1)).toBe("open");
		expect(socket.sent.some((frame) => "sub" in frame)).toBe(true);
		wire.close();
	});

	test("refuses a core that rejects the hello: no subscriptions are replayed, and the state says why", async () => {
		const { wire, states, socket, hello } = dial();
		await socket.answer({ id: hello["id"], ok: false, error: "unknown command client.hello" });
		expect(states).not.toContain("open");
		expect(states.at(-1)).toBe("outdated");
		expect(socket.sent.some((frame) => "sub" in frame)).toBe(false);
		wire.close();
	});

	test("refuses a core that answers without threads2", async () => {
		const { wire, states, socket, hello } = dial();
		await socket.answer({ id: hello["id"], ok: true, result: { capabilities: ["threads"] } });
		expect(states).not.toContain("open");
		expect(states.at(-1)).toBe("outdated");
		expect(socket.sent.some((frame) => "sub" in frame)).toBe(false);
		wire.close();
	});

	test("a socket that drops before the core answers is a dropped connection, not an old core", async () => {
		const { wire, states, socket } = dial();
		socket.close();
		expect(states.at(-1)).toBe("closed");
		wire.close();
	});
});
