import { expect, spyOn, test } from "bun:test";
import type { TranscriptEvent } from "../src/generated/contract";
import { installDom } from "./dom";

/** A clock and timer the test drives, so a watchdog fires when told to. */
function fakeTimers() {
	const pending = new Map<number, () => void>();
	let next = 1;
	return {
		set: (fn: () => void) => {
			pending.set(next, fn);
			return next++;
		},
		clear: (id: number) => void pending.delete(id),
		fire: () => {
			for (const [id, fn] of [...pending]) {
				pending.delete(id);
				fn();
			}
		},
		count: () => pending.size,
	};
}

test("a card is answered once: a second answer while the first is out is refused, and a refusal frees it", async () => {
	const { Answers } = await import("../src/answering");
	const answers = new Answers(fakeTimers());
	let sent = 0;
	let fail: (error: Error) => void = () => {};
	expect(answers.begin("permission:1", () => { sent++; return new Promise((_, reject) => { fail = reject; }); })).toBe(true);
	expect(answers.begin("permission:1", async () => { sent++; })).toBe(false);
	expect(sent).toBe(1);
	expect(answers.get("permission:1")).toEqual({ k: "answering" });
	fail(new Error("Not now"));
	await Promise.resolve();
	await Promise.resolve();
	expect(answers.get("permission:1")).toEqual({ k: "idle", failure: "Not now" });
	expect(answers.begin("permission:1", async () => { sent++; })).toBe(true);
});

test("an answer the core took but the tape never shows frees the card after the watchdog, and a settled card forgets it", async () => {
	const { Answers } = await import("../src/answering");
	const timers = fakeTimers();
	const answers = new Answers(timers);
	expect(answers.begin("human:1", async () => null)).toBe(true);
	await Promise.resolve();
	expect(answers.get("human:1").k).toBe("answering");
	timers.fire();
	expect(answers.get("human:1").k).toBe("idle");
	expect(answers.begin("human:1", async () => null)).toBe(true);
	answers.settle("human:1");
	expect(answers.get("human:1")).toEqual({ k: "idle" });
	expect(timers.count()).toBe(0);
});

test("a credential or secret-use card past its time stops counting as needing the person", async () => {
	const { openAsks } = await import("../src/components/transcript/AskCard");
	const base = { ts: 1 };
	const credential: TranscriptEvent = { ...base, kind: "credential_ask", id: "c", actionId: "c", site: "s", reason: "r", credentialKind: "login", name: "N", status: "pending", expiresAt: 1000, existing: [], armed: false };
	const secret: TranscriptEvent = { ...base, kind: "secret_use_ask", id: "u", useId: "u", secret: "K", secretKind: "login", site: "s", path: "/", teammate: "ada", askedAt: 0, expiresAt: 2000, status: "pending" };
	const human: TranscriptEvent = { ...base, kind: "human_action", id: "h", actionId: "h", reason: "r", status: "pending" } as TranscriptEvent;
	expect(openAsks([credential, secret, human], 500).map((a) => a.id)).toEqual(["c", "u", "h"]);
	expect(openAsks([credential, secret, human], 1500).map((a) => a.id)).toEqual(["u", "h"]);
	expect(openAsks([credential, secret, human], 2500).map((a) => a.id)).toEqual(["h"]);
});

test("the composer is told when the next card runs out", async () => {
	const { nextLapse } = await import("../src/components/transcript/AskCard");
	const card = (id: string, expiresAt: number, status = "pending") => ({ kind: "secret_use_ask", id, ts: 1, useId: id, secret: "K", secretKind: "login", site: "s", path: "/", teammate: "ada", askedAt: 0, expiresAt, status }) as TranscriptEvent;
	expect(nextLapse([card("a", 900), card("b", 2000), card("c", 1500, "approved")], 1000)).toBe(2000);
	expect(nextLapse([card("a", 900)], 1000)).toBeUndefined();
});

test("the same card mounted inline and in the composer panel answers once, and both mounts go dead together", async () => {
	const { teardown } = installDom({ ResizeObserver: class { observe() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { SecretUseAsk } = await import("../src/components/transcript/SecretUseAsk");
	const { wire } = await import("../src/wire");
	const calls: string[] = [];
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string) => { calls.push(cmd); return new Promise(() => {}); }) as typeof wire.command);
	const event: Extract<TranscriptEvent, { kind: "secret_use_ask" }> = { kind: "secret_use_ask", id: "twice", ts: 1, useId: "opaque", secret: "GH", secretKind: "login", site: "github.com", path: "/", teammate: "ada", askedAt: Date.now(), expiresAt: Date.now() + 300_000, status: "pending" };
	const inline = document.createElement("div");
	const panel = document.createElement("div");
	document.body.append(inline, panel);
	const a = createRoot(inline);
	const b = createRoot(panel);
	const approve = (box: HTMLElement) => [...box.querySelectorAll("button")].find((x) => x.textContent === "Approve")!;
	try {
		await act(async () => { a.render(<SecretUseAsk event={event} teammate="Ada" />); b.render(<SecretUseAsk event={event} teammate="Ada" />); });
		await act(async () => approve(inline).click());
		expect(approve(panel).disabled).toBe(true);
		await act(async () => approve(panel).click());
		expect(calls).toEqual(["secrets.use.answer"]);
	} finally {
		await act(async () => { a.unmount(); b.unmount(); });
		command.mockRestore();
		await teardown();
	}
});
