import { expect, spyOn, test } from "bun:test";
import * as native from "../src/native";
import { noticeRoster } from "../src/notify";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

test("a thread post notifies without a home turn, once, and snapshots stay quiet", async () => {
	const restore = installDom();
	const toast = spyOn(native, "postToast").mockResolvedValue(undefined);
	const focus = spyOn(document, "hasFocus").mockReturnValue(false);
	const entry = (at: number): RosterEntry => ({
		persona: { id: "call-notifications", name: "Ada" },
		session: { state: "ready" },
		latest: at,
		preview: { from: "agent", text: "Harbour plan: the beacon is repaired.", at, posted: true },
	}) as unknown as RosterEntry;
	try {
		noticeRoster([entry(1)]);
		expect(toast).not.toHaveBeenCalled();
		noticeRoster([entry(2)]);
		expect(toast).toHaveBeenCalledTimes(1);
		expect(toast).toHaveBeenCalledWith("call-notifications", "Ada", "Harbour plan: the beacon is repaired.");
		noticeRoster([entry(2)]);
		expect(toast).toHaveBeenCalledTimes(1);
		focus.mockReturnValue(true);
		noticeRoster([entry(3)]);
		expect(toast).toHaveBeenCalledTimes(1);
	} finally {
		noticeRoster([]);
		toast.mockRestore();
		focus.mockRestore();
		await restore.teardown();
	}
});

test("a turn that put no new line in the chat posts no toast, and one that did posts its line", async () => {
	const restore = installDom();
	const toast = spyOn(native, "postToast").mockResolvedValue(undefined);
	const focus = spyOn(document, "hasFocus").mockReturnValue(false);
	const entry = (state: "ready" | "thinking", at: number, text: string): RosterEntry =>
		({
			persona: { id: "quiet-turns", name: "Bob" },
			session: { state },
			latest: at,
			preview: { from: "them", text, at },
		}) as unknown as RosterEntry;
	try {
		noticeRoster([entry("ready", 1, "Morning.")]);
		// The inbox woke Bob: he sorted it and said nothing to the person.
		noticeRoster([entry("thinking", 1, "Morning.")]);
		noticeRoster([entry("ready", 1, "Morning.")]);
		expect(toast).not.toHaveBeenCalled();
		// The next quiet turn posted.
		noticeRoster([entry("thinking", 1, "Morning.")]);
		noticeRoster([entry("thinking", 2, "Ada needs the crane permit signed.")]);
		noticeRoster([entry("ready", 2, "Ada needs the crane permit signed.")]);
		expect(toast).toHaveBeenCalledTimes(1);
		expect(toast).toHaveBeenCalledWith("quiet-turns", "Bob", "Ada needs the crane permit signed.");
	} finally {
		noticeRoster([]);
		toast.mockRestore();
		focus.mockRestore();
		await restore.teardown();
	}
});
