import { expect, test } from "bun:test";
import type { FileChunk } from "../src/generated/contract";
import type { RosterEntry } from "../src/wire";
import { installDom } from "./dom";

/**
 * A working teammate's face blinks: its eyes-shut picture is laid over it
 * and a stepped keyframe shows it now and then. Not working, or with Reduce
 * motion on, the shut picture is never even read. In the rail a face blinks
 * while its teammate works and no card waits on the person, and its pin's
 * dot then holds steady; a face with no shut pose keeps the beat.
 */

const chunkOf = (name: string): FileChunk => ({
	name: `${name}.png`,
	mimeType: "image/png",
	size: 3,
	offset: 0,
	data: btoa("png"),
});

async function face(props: { blinking: boolean; shut?: string }, reduce = false) {
	const { teardown } = installDom();
	window.matchMedia = ((query: string) => ({ matches: reduce && query.includes("reduce") })) as unknown as typeof window.matchMedia;
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Avatar } = await import("../src/ui/Avatar");
	const { forgetAvatars } = await import("../src/avatars");
	forgetAvatars();
	const asked: string[] = [];
	const read = async (_personaId: string, hash: string) => {
		asked.push(hash);
		return chunkOf(hash);
	};
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(<Avatar id="ada" name="Ada" size={28} hash="open" {...(props.shut === undefined ? {} : { shut: props.shut })} blinking={props.blinking} read={read} />);
		});
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		const avatar = container.querySelector<HTMLElement>(".avatar")!;
		return {
			blinking: avatar.hasAttribute("data-blinking"),
			pictures: avatar.querySelectorAll("img").length,
			shut: avatar.querySelector(".avatar-shut") !== null,
			every: avatar.style.getPropertyValue("--blink-every"),
			asked,
		};
	} finally {
		await act(async () => {
			root.unmount();
		});
		await teardown();
	}
}

test("a working face with an eyes-shut picture lays it over itself and blinks", async () => {
	const seen = await face({ blinking: true, shut: "shut" });
	expect(seen.blinking).toBe(true);
	expect(seen.pictures).toBe(2);
	expect(seen.shut).toBe(true);
	expect(seen.every).toMatch(/^\d\.\d\ds$/);
	expect(seen.asked.sort()).toEqual(["open", "shut"]);
});

test("a face not working, or with no eyes-shut picture, is one picture and reads no other", async () => {
	for (const props of [{ blinking: false, shut: "shut" }, { blinking: true }]) {
		const seen = await face(props);
		expect(seen.blinking).toBe(false);
		expect(seen.pictures).toBe(1);
		expect(seen.shut).toBe(false);
		expect(seen.asked).toEqual(["open"]);
	}
});

test("with Reduce motion on, a working face does not blink", async () => {
	const seen = await face({ blinking: true, shut: "shut" }, true);
	expect(seen.blinking).toBe(false);
	expect(seen.pictures).toBe(1);
	expect(seen.asked).toEqual(["open"]);
});

test("faces blink at their own rhythm, from their ids", async () => {
	Object.assign(globalThis, { window: globalThis.window ?? { matchMedia: () => ({ matches: false }) } });
	const { blinkRhythm } = await import("../src/ui/Avatar");
	const rhythms = ["ada", "bert", "cleo", "dov", "eve"].map((id) => blinkRhythm(id));
	for (const rhythm of rhythms) {
		const every = Number.parseFloat(rhythm["--blink-every"]);
		const phase = Number.parseFloat(rhythm["--blink-phase"]);
		expect(every).toBeGreaterThanOrEqual(4.4);
		expect(every).toBeLessThanOrEqual(5.6);
		expect(phase).toBeLessThanOrEqual(0);
		expect(phase).toBeGreaterThan(-every);
	}
	expect(new Set(rhythms.map((rhythm) => `${rhythm["--blink-every"]} ${rhythm["--blink-phase"]}`)).size).toBe(5);
	expect(blinkRhythm("ada")).toEqual(blinkRhythm("ada"));
});

const row = (id: string, more: { state?: string; waiting?: boolean; shut?: boolean; awaitingSubagents?: boolean } = {}) =>
	({
		persona: { id, name: id.toUpperCase(), goal: "", ...(more.shut ? { avatar: { hash: `${id}-open`, by: "self", updatedAt: "", poses: { shut: `${id}-shut` } } } : {}) },
		session: { personaId: id, state: more.state ?? "idle", ...(more.awaitingSubagents ? { awaitingSubagents: true } : {}) },
		waiting: more.waiting ?? false,
		drawing: false,
		pin: 0,
	}) as unknown as RosterEntry;

test("a face blinks only while its teammate works with no card waiting on the person", async () => {
	const { faceBlinks } = await import("../src/activity");
	expect(faceBlinks(row("a", { state: "thinking", shut: true }))).toBe(true);
	expect(faceBlinks(row("a", { state: "starting", shut: true }))).toBe(true);
	expect(faceBlinks(row("a", { state: "thinking", shut: true, waiting: true }))).toBe(false);
	expect(faceBlinks(row("a", { state: "thinking", shut: true, awaitingSubagents: true }))).toBe(false);
	for (const state of ["idle", "ready", "stopped", "error"]) {
		expect(faceBlinks(row("a", { state, shut: true }))).toBe(false);
	}
	expect(faceBlinks(row("a", { state: "thinking" }))).toBe(false);
});

test("a pinned face that blinks has a steady dot, and one with no eyes-shut picture keeps the beat", async () => {
	const { teardown } = installDom();
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Rail } = await import("../src/components/Rail");
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const dotOf = (name: string) => {
		const pin = [...container.querySelectorAll<HTMLElement>("[data-teammate-row]")].find((node) => node.getAttribute("aria-label")?.startsWith(name));
		return pin?.querySelector(".rail-face-vital") ?? null;
	};
	try {
		await act(async () => {
			root.render(
				<Rail
					entries={[row("a", { state: "thinking", shut: true }), { ...row("b", { state: "thinking" }), pin: 1 } as RosterEntry]}
					selectedId={null}
					seen={{}}
					connection="open"
					onSelect={() => {}}
					onEdit={() => {}}
					onDelete={() => {}}
					onPin={() => {}}
					onArrange={() => {}}
					onLead={() => {}}
					onAddDesk={() => {}}
					onNew={() => {}}
					settings={null}
					search={null}
					deskKey={null}
				/>,
			);
		});
		const blinking = dotOf("A");
		const beating = dotOf("B");
		expect(blinking).not.toBeNull();
		expect(beating).not.toBeNull();
		expect(blinking!.classList.contains("beat")).toBe(false);
		expect(beating!.classList.contains("beat")).toBe(true);
	} finally {
		await act(async () => {
			root.unmount();
		});
		await teardown();
	}
});
