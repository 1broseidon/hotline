import { afterEach, beforeEach, expect, spyOn, test } from "bun:test";
import type { Root } from "react-dom/client";
import type { InboundSource } from "../src/generated/contract";
import type { Wire } from "../src/wire";
import { installDom } from "./dom";

Object.assign(globalThis, { window: { matchMedia: () => ({ matches: false }) } });
const { InboundSection } = await import("../src/components/settings/Inbound");
const { replaceDesks, wireFor } = await import("../src/desks");

let act: typeof import("react").act;
let root: Root;
let container: HTMLDivElement;
let teardown: () => Promise<void>;
let restore: () => void;
let sent: { cmd: string; params: unknown }[];
let sources: InboundSource[];

const repo: InboundSource = {
	id: "gh",
	type: "github",
	name: "hotline repo",
	teammates: ["quill", "huck"],
	maxLevel: "note",
	rate: 10,
	createdAt: 1,
};

beforeEach(async () => {
	({ teardown } = installDom());
	replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }]);
	sent = [];
	sources = [repo, { ...repo, id: "mail", type: "mail", name: "support inbox", teammates: ["quill"], maxLevel: "person", muted: true, filesAllowed: true }];
	const command = spyOn(wireFor("local"), "command").mockImplementation((async (cmd: string, params: unknown) => {
		sent.push({ cmd, params });
		if (cmd === "inbound.list") return sources;
		if (cmd === "inbound.update") return sources[0];
		if (cmd === "inbound.remove") return null;
		throw new Error(`Not in this test: ${cmd}`);
	}) as Wire["command"]);
	restore = () => command.mockRestore();
	act = (await import("react")).act;
	const { createRoot } = await import("react-dom/client");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	restore();
	replaceDesks([]);
	await teardown();
});

const settle = () =>
	act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

async function show() {
	await act(async () => {
		root.render(<InboundSection team={[{ id: "quill", name: "Quill" }, { id: "huck", name: "Huck" }]} />);
	});
	await settle();
}

test("lists each source with whom it may aside and how, a switch and Remove, and no add form", async () => {
	await show();
	const rows = [...container.querySelectorAll(".group-row")];
	expect(rows.map((row) => row.querySelector(".group-row-title")?.textContent)).toEqual(["hotline repo", "support inbox"]);
	expect(rows[0]!.querySelector(".group-row-detail")?.textContent).toBe("GitHub · to Quill, Huck · note · 10 an hour");
	expect(rows[1]!.querySelector(".group-row-detail")?.textContent).toBe("Mailbox · to Quill · to you · 10 an hour · files");
	const switches = [...container.querySelectorAll<HTMLInputElement>('input[role="switch"]')];
	expect(switches.map((one) => one.checked)).toEqual([true, false]);
	// Nothing is drawn that adds one: no field, no Add.
	expect(container.querySelectorAll('input:not([role="switch"]), textarea').length).toBe(0);
	expect([...container.querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Remove…", "Remove…"]);
});

test("the switch mutes and unmutes, and Remove asks once more", async () => {
	await show();
	const [first, second] = [...container.querySelectorAll<HTMLInputElement>('input[role="switch"]')];
	await act(async () => first!.click());
	await act(async () => second!.click());
	expect(sent.filter((one) => one.cmd === "inbound.update").map((one) => one.params)).toEqual([
		{ id: "gh", muted: true },
		{ id: "mail", muted: false },
	]);

	const remove = container.querySelector<HTMLButtonElement>(".group-row button")!;
	await act(async () => remove.click());
	expect(sent.some((one) => one.cmd === "inbound.remove")).toBe(false);
	const confirm = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Remove")!;
	await act(async () => confirm.click());
	await settle();
	expect(sent.filter((one) => one.cmd === "inbound.remove").map((one) => one.params)).toEqual([{ id: "gh" }]);
});

test("says when there are none", async () => {
	sources = [];
	await show();
	expect(container.textContent).toContain("None yet.");
	expect(container.querySelector('input[role="switch"]')).toBeNull();
});
