import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { TranscriptEvent } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

// task-419: a permission card nobody answered in the driver's time. The desk
// settles it on the tape as expired and refuses an answer that comes after.

type Card = Extract<TranscriptEvent, { kind: "permission" }>;

const REFUSED = "That request is no longer waiting for an answer.";
const live: Card = {
	kind: "permission", id: "perm:req", ts: 1, requestId: "req", title: "Run rm -rf build",
	options: [
		{ optionId: "allow", name: "Allow", kind: "allow_once" },
		{ optionId: "deny", name: "Deny", kind: "reject_once" },
	],
};
/** As the desk writes it when the wait runs out. */
const expired: Card = { ...live, ts: 2, decision: "expired", decidedOptionName: "Expired unanswered" };
const thread = { kind: "dm", key: "pork" } as const;
const who = { id: "pork", name: "Pork Chop" };

let act: typeof import("react").act;
let root: Root;
let box: HTMLDivElement;
let sent: { cmd: string; params: unknown }[];
let refuse: boolean;
let restore: () => void;
let teardown: () => Promise<void>;

beforeEach(async () => {
	teardown = installDom().teardown;
	const { replaceDesks, wireFor } = await import("../src/desks");
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	sent = [];
	refuse = false;
	const command = spyOn(wireFor("local"), "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		if (refuse) throw new Error(REFUSED);
		return null;
	}) as Wire["command"]);
	restore = () => command.mockRestore();
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	box = document.createElement("div");
	document.body.append(box);
	root = createRoot(box);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	const { replaceDesks } = await import("../src/desks");
	replaceDesks([]);
	await teardown();
});

async function show(event: Card) {
	const { Permission } = await import("../src/components/transcript/Asks");
	await act(async () => root.render(<Permission thread={thread} event={event} who={who} />));
}
const buttons = () => [...box.querySelectorAll<HTMLButtonElement>("button")];

test("a permission card that expired unanswered says Expired, has no buttons, and no longer waits on the person", async () => {
	const { openAsks } = await import("../src/components/transcript/AskCard");
	await show(live);
	expect(buttons().map((one) => one.textContent)).toEqual(["Allow", "Deny"]);
	expect(openAsks([live]).map((ask) => ask.id)).toEqual(["perm:req"]);

	// The tape supersedes the card by its id; the fold shows the expired one.
	await show(expired);
	expect(buttons()).toEqual([]);
	expect(box.querySelector(".ask-card")).toBeNull();
	expect(box.querySelector(".ask-settled .ask-chip")?.textContent).toBe("Expired");
	expect(box.querySelector(".ask-settled-what")?.textContent).toBe("Run rm -rf build");
	// The composer's chip and a Needs you row both read the open cards.
	expect(openAsks([expired])).toEqual([]);

	// Nothing on an expired card sends an answer.
	expect(sent).toEqual([]);
});

test("an answer sent after the request expired is refused in words on the card, and the card then reads Expired", async () => {
	await show(live);
	refuse = true;
	const allow = buttons().find((one) => one.textContent === "Allow");
	await act(async () => allow?.click());
	expect(sent).toEqual([{ cmd: "thread.answer", params: { thread, answer: { kind: "permission", requestId: "req", optionId: "allow" } } }]);

	// The desk's reason is on the card; the answer was not taken for given.
	expect(box.querySelector('[role="alert"]')?.textContent).toBe(REFUSED);
	expect(box.querySelector(".ask-settled")).toBeNull();
	expect(buttons().every((one) => !one.disabled)).toBe(true);

	// The tape then says why: the card is expired, and takes no more answers.
	await show(expired);
	expect(buttons()).toEqual([]);
	expect(box.querySelector(".ask-settled .ask-chip")?.textContent).toBe("Expired");
	expect(box.querySelector('[role="alert"]')).toBeNull();
	expect(sent).toHaveLength(1);
});
