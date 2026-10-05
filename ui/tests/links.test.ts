import { describe, expect, test } from "bun:test";
import type { LinkEvent } from "../src/dock";
import { handedIn, handoffsUnderway, linkFailed, linkLine, namesSaid, runState, threadOfLink } from "../src/links";

const link = (extra: Partial<LinkEvent> = {}): LinkEvent => ({
	kind: "link",
	id: "link:side:s1",
	ts: 1_000,
	thread: "s1",
	threadKind: "side",
	personaId: "p",
	title: "Fix the CI badge",
	state: "live",
	...extra,
});

describe("a work thread's line in the conversation", () => {
	test("says it started while it runs, and whose hands it came from when a teammate opened it", () => {
		expect(linkLine(link())).toBe("Started a side thread · Fix the CI badge");
		expect(linkLine(link({ openerId: "mack", openerName: "Mack" }), "p")).toBe("From Mack · Fix the CI badge");
	});

	test("says who a handoff went to on the hands that gave it, all the way to its result", () => {
		const people = new Map([["p", { name: "Poe" }]]);
		const handed = link({ openerId: "mack", openerName: "Mack" });
		expect(linkLine(handed, "mack", people)).toBe("Handed to Poe · Fix the CI badge");
		expect(linkLine({ ...handed, state: "closed", end: "agent", outcome: "Badge is green." }, "mack", people)).toBe(
			"Handed to Poe · Fix the CI badge · Badge is green.",
		);
		expect(linkLine({ ...handed, state: "closed", end: "agent", outcome: "Badge is green." }, "p", people)).toBe(
			"From Mack · Fix the CI badge · Badge is green.",
		);
		expect(linkLine(handed, "mack")).toBe("Handed over · Fix the CI badge");
	});

	test("becomes its title and a one-line result once closed, and says when nobody ended it", () => {
		expect(linkLine(link({ state: "closed", end: "agent", outcome: "Badge is green." }))).toBe("Side thread · Fix the CI badge · Badge is green.");
		expect(linkLine(link({ state: "closed", end: "stopped", outcome: "Half done." }))).toBe("Side thread · Fix the CI badge · Half done. · stopped");
		expect(linkLine(link({ state: "closed", end: "idle", outcome: "Quiet." }))).toBe("Side thread · Fix the CI badge · Quiet. · archived, idle");
		expect(linkLine(link({ state: "closed", end: "person" }))).toBe("Side thread · Fix the CI badge · archived");
	});

	test("says it is parked while its agent is let go of and the thread is open", () => {
		expect(linkLine(link({ state: "parked" }))).toBe("Side thread · Fix the CI badge · parked");
	});
});

describe("a subagent's run's line", () => {
	const run = (extra: Partial<LinkEvent> = {}) => link({ threadKind: "run", thread: "r1", title: "Read the logs", ...extra });

	test("is working until it stops, then how it went and how long it took", () => {
		expect(linkLine(run())).toBe("Subagent · Read the logs · working");
		expect(linkLine(run({ state: "closed", end: "done", elapsedMs: 75_000 }))).toBe("Subagent · Read the logs · done in 1 min 15 s");
		expect(runState({ state: "closed", end: "failed", elapsedMs: 4_000 })).toBe("failed after 4 s");
		expect(runState({ state: "closed", end: "cancelled" })).toBe("stopped");
		expect(runState({ state: "closed", end: "done" })).toBe("done");
	});

	test("a failed one takes the warning colour", () => {
		expect(linkFailed(run({ state: "closed", end: "failed" }))).toBe(true);
		expect(linkFailed(run({ state: "closed", end: "done" }))).toBe(false);
		expect(linkFailed(run())).toBe(false);
	});
});

describe("a call's line", () => {
	const call = (extra: Partial<LinkEvent> = {}) => link({ threadKind: "call", thread: "c1", title: "Call", ...extra });

	test("reads as in progress while it is live", () => {
		expect(linkLine(call())).toBe("Call · in progress");
	});

	test("says how long it lasted and how it ended", () => {
		expect(linkLine(call({ state: "closed", at: 1_000 + 240_000, outcome: "Hung up" }))).toBe("Call · 4 min · Hung up");
	});

	test("a short call is under a minute, and no outcome is left out", () => {
		expect(linkLine(call({ state: "closed", at: 13_000 }))).toBe("Call · under a minute");
	});
});

test("a link names the thread it stands for", () => {
	expect(threadOfLink(link({ threadKind: "run", thread: "r9" }))).toEqual({ kind: "run", key: "r9" });
});

describe("work colleagues handed a teammate", () => {
	const handed = (id: string, state: LinkEvent["state"], openerName = "Poe", openerId = "poe") =>
		link({ id, thread: id, threadKind: "side", state, personaId: "toad", openerId, openerName });

	test("is told apart from the teammate's own threads and from what it handed on", () => {
		expect(handedIn(handed("a", "live"), "toad")).toBe(true);
		expect(handedIn(link({ threadKind: "side", personaId: "toad" }), "toad")).toBe(false);
		expect(handedIn(handed("a", "live", "Toad", "toad"), "toad")).toBe(false);
		// The copy on the sender's tape is what the sender handed on.
		expect(handedIn(handed("a", "live"), "poe")).toBe(false);
	});

	test("is one line for whatever is unfinished, newest state of each", () => {
		const events = [handed("a", "live"), handed("b", "live", "Clementine", "clem"), handed("a", "closed"), handed("c", "parked")];
		expect(handoffsUnderway(events, "toad")).toEqual({ count: 2, from: ["Clementine", "Poe"] });
		expect(handoffsUnderway([handed("a", "closed")], "toad")).toEqual({ count: 0, from: [] });
	});

	test("names its senders the way a person lists them", () => {
		expect(namesSaid(["Poe"])).toBe("Poe");
		expect(namesSaid(["Poe", "Mack"])).toBe("Poe and Mack");
		expect(namesSaid(["Poe", "Mack", "Ada"])).toBe("Poe, Mack and Ada");
	});
});
