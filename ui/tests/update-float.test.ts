import { expect, test } from "bun:test";
import { updateToShow } from "../src/components/UpdateFloat";
import type { UpdateStatus } from "../src/native";

const status = (patch: Partial<UpdateStatus> = {}): UpdateStatus => ({
	current: "0.35.0",
	available: { version: "0.35.1", notes: "" },
	checkedAt: 1,
	phase: "idle",
	downloaded: 0,
	total: null,
	error: null,
	disabledReason: null,
	...patch,
});

test("a waiting version shows until that version is put away, and a newer one shows again", () => {
	expect(updateToShow(status(), null)).toBe("0.35.1");
	expect(updateToShow(status(), "0.35.1")).toBeNull();
	expect(updateToShow(status({ available: { version: "0.35.2", notes: "" } }), "0.35.1")).toBe("0.35.2");
});

test("nothing waiting, updates off, or an update under way shows no card", () => {
	expect(updateToShow(null, null)).toBeNull();
	expect(updateToShow(status({ available: null }), null)).toBeNull();
	expect(updateToShow(status({ disabledReason: "Open the desktop application to check for updates." }), null)).toBeNull();
	expect(updateToShow(status({ phase: "downloading" }), null)).toBeNull();
});
