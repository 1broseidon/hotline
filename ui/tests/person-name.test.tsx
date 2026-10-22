import { expect, spyOn, test } from "bun:test";
import type { Handlers } from "../src/wire";
import { installDom } from "./dom";

test("Your name round-trips through General settings and follows room updates", async () => {
	const { dom, teardown } = installDom({ ResizeObserver: class { observe() {} unobserve() {} disconnect() {} } });
	const { act } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { Settings } = await import("../src/components/Settings");
	const { replaceDesks } = await import("../src/desks");
	const { Wire, wire } = await import("../src/wire");
	let saved = "Pat";
	let room: Handlers<unknown, unknown> | undefined;
	const patches: unknown[] = [];
	const subscribe = spyOn(Wire.prototype, "subscribe").mockImplementation(((target: unknown, handlers: Handlers<unknown, unknown>) => {
		if (target === "room") {
			room = handlers;
			handlers.snapshot([{ kind: "setting", id: "personName", value: saved }]);
		}
		return () => {};
	}) as InstanceType<typeof Wire>["subscribe"]);
	const command = spyOn(wire, "command").mockImplementation((async (cmd: string, params: unknown) => {
		if (cmd === "settings.update") {
			patches.push(params);
			saved = (params as { patch: { personName: string } }).patch.personName;
			room?.event({ kind: "setting", id: "personName", value: saved });
		}
		return [];
	}) as typeof wire.command);
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		replaceDesks([{ id: "person-name-test", kind: "local", name: "Test", origin: "ws://localhost", token: "test" }]);
		const render = async (key: string) => act(async () => { root.render(<Settings key={key} section="general" roster={[]} />); });
		await render("first");
		const field = () => container.querySelector<HTMLInputElement>("#setting-person-name")!;
		expect(container.querySelector('label[for="setting-person-name"]')?.textContent).toBe("Your name");
		expect(field().value).toBe("Pat");
		const edit = async (value: string) => {
			await act(async () => {
				field().focus();
				Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value")!.set!.call(field(), value);
				field().dispatchEvent(new window.Event("input", { bubbles: true }));
			});
			await act(async () => { field().blur(); });
		};
		await edit(" George ");
		expect(patches).toEqual([{ patch: { personName: "George" } }]);
		await render("reopened");
		expect(field().value).toBe("George");
		await act(async () => { room?.event({ kind: "setting", id: "personName", value: "Jo" }); });
		expect(field().value).toBe("Jo");
		await edit("");
		expect(patches.at(-1)).toEqual({ patch: { personName: "" } });
		await render("cleared");
		expect(field().value).toBe("");
	} finally {
		await act(async () => { root.unmount(); });
		command.mockRestore();
		subscribe.mockRestore();
		replaceDesks([]);
		await teardown();
	}
});
