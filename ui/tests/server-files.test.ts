import { describe, expect, test } from "bun:test";
Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { nameOf, parentOf } = await import("../src/serverFiles");
const { toastTarget } = await import("../src/notify");
const { deskKey } = await import("../src/desks");

describe("server paths", () => {
	test("a parent climbs one folder and stops at the root", () => {
		expect(parentOf("/srv/work/report.pdf")).toBe("/srv/work");
		expect(parentOf("/srv/work/")).toBe("/srv");
		expect(parentOf("/srv")).toBe("/");
		expect(parentOf("/")).toBe("/");
	});

	test("a name is the last part, on either kind of slash", () => {
		expect(nameOf("/srv/work/report.pdf")).toBe("report.pdf");
		expect(nameOf("C:\\Users\\ada\\notes.txt")).toBe("notes.txt");
		expect(nameOf("/srv/work/")).toBe("work");
	});
});

describe("toasts from another desk", () => {
	test("a plain id is a teammate on the desk on screen", () => {
		expect(toastTarget("5f0c")).toEqual({ deskId: null, personaId: "5f0c" });
	});

	test("a desk-qualified id names the desk to open", () => {
		expect(toastTarget("9ea8abda7eac/5f0c")).toEqual({ deskId: "9ea8abda7eac", personaId: "5f0c" });
	});
});

describe("per-desk keys", () => {
	test("the local desk keeps the key it always had", () => {
		expect(deskKey("hotline.rail.seen", "local")).toBe("hotline.rail.seen");
		expect(deskKey("hotline.rail.seen", "9ea8abda7eac")).toBe("hotline.rail.seen:9ea8abda7eac");
	});
});
