import { describe, expect, test } from "bun:test";
import { move, type Link, type LinkEvent } from "../src/voice/link";

const live: Link = { kind: "live", muted: false };
const run = (from: Link, ...events: LinkEvent[]) => events.reduce(move, from);

describe("a call's link to the desk", () => {
	test("a hold and its release move live to held and back", () => {
		expect(run(live, { type: "hold" })).toEqual({ kind: "held" });
		expect(run(live, { type: "hold" }, { type: "release" })).toEqual(live);
	});

	test("a barge-in mutes the front until the person is heard", () => {
		const muted = run(live, { type: "barge" });
		expect(muted).toEqual({ kind: "live", muted: true });
		expect(run(muted, { type: "heard" })).toEqual(live);
	});

	test("a hold, a release, or a lost socket each clear the mute", () => {
		const muted = run(live, { type: "barge" });
		expect(run(muted, { type: "hold" }, { type: "release" })).toEqual(live);
		expect(run(muted, { type: "socket", open: false }, { type: "attached" })).toEqual(live);
		expect(run(muted, { type: "provider", reconnecting: true }, { type: "provider", reconnecting: false })).toEqual(live);
	});

	test("a state event that is not a reconnect leaves the mute alone", () => {
		const muted = run(live, { type: "barge" });
		expect(run(muted, { type: "provider", reconnecting: false }, { type: "attached" })).toEqual(muted);
	});

	test("barging in while held or reconnecting changes nothing", () => {
		expect(run({ kind: "held" }, { type: "barge" })).toEqual({ kind: "held" });
		const away = run(live, { type: "socket", open: false });
		expect(run(away, { type: "barge" })).toEqual(away);
	});

	test("any fresh snapshot ends a lost socket, and returns to the hold the person had", () => {
		expect(run(live, { type: "socket", open: false }, { type: "attached" })).toEqual(live);
		expect(run({ kind: "held" }, { type: "socket", open: false }, { type: "attached" })).toEqual({ kind: "held" });
	});

	test("a hold taken or released while reconnecting is kept for after it", () => {
		const away = run(live, { type: "socket", open: false });
		expect(run(away, { type: "hold" }, { type: "attached" })).toEqual({ kind: "held" });
		expect(run({ kind: "held" }, { type: "socket", open: false }, { type: "release" }, { type: "attached" })).toEqual(live);
	});

	test("the provider's reconnect is its own cause and a fresh snapshot does not end it", () => {
		const provider = run(live, { type: "provider", reconnecting: true });
		expect(provider.kind).toBe("reconnecting");
		expect(run(provider, { type: "attached" })).toEqual(provider);
		expect(run(provider, { type: "provider", reconnecting: false })).toEqual(live);
	});

	test("a provider that recovers does not end a lost socket", () => {
		const both = run(live, { type: "socket", open: false }, { type: "provider", reconnecting: true });
		expect(run(both, { type: "provider", reconnecting: false }).kind).toBe("reconnecting");
		expect(run(both, { type: "attached" }).kind).toBe("reconnecting");
		expect(run(both, { type: "attached" }, { type: "provider", reconnecting: false })).toEqual(live);
	});

	test("closing is final for the link", () => {
		const closing = run(live, { type: "closing", reason: "goodbye" });
		expect(closing).toEqual({ kind: "closing", reason: "goodbye" });
		expect(run(closing, { type: "hold" }, { type: "release" }, { type: "attached" }, { type: "barge" })).toEqual(closing);
	});
});
