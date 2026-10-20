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
