import { beforeEach, describe, expect, test } from "bun:test";
// desks.ts reads the shell's globals when it loads; outside the shell there are none.
const { activeDeskId, replaceDesks, setActiveDesk, LOCAL_DESK } = await import("../src/desks");

const desk = (id: string, kind: "local" | "remote" = "remote") => ({
	id,
	name: id,
	kind,
	origin: `http://127.0.0.1:${id.length}`,
	token: `token-${id}`,
});
const local = desk(LOCAL_DESK, "local");
const server = desk("server-1");

beforeEach(() => {
	replaceDesks([local]);
});

describe("switching to a desk that was just paired", () => {
	test("switches when the shell's list already has it", () => {
		replaceDesks([local, server]);
		setActiveDesk(server.id);
		expect(activeDeskId()).toBe(server.id);
	});

	test("switches when the shell's list arrives after the pairing answer", () => {
		expect(activeDeskId()).toBe(LOCAL_DESK);
		setActiveDesk(server.id);
		expect(activeDeskId()).toBe(LOCAL_DESK);
		replaceDesks([local, server]);
		expect(activeDeskId()).toBe(server.id);
	});

	test("lets the ask lapse if the list that follows does not have the desk", () => {
		setActiveDesk(server.id);
		replaceDesks([local]);
		replaceDesks([local, server]);
		expect(activeDeskId()).toBe(LOCAL_DESK);
	});

	test("a desk chosen by name that is listed wins over one still awaited", () => {
		const other = desk("server-2");
		setActiveDesk(server.id);
		replaceDesks([local, other]);
		setActiveDesk(other.id);
		replaceDesks([local, server, other]);
		expect(activeDeskId()).toBe(other.id);
	});
});
